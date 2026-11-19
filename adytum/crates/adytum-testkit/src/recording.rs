use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use adytum_exec::{Cancel, Chunk, Command, Duplex, DuplexControl, ExecError, Output, Runner};
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

use crate::transcript::{abstract_paths, default_vars, set_var, Call, Transcript};

/// Wraps any `Runner`, passes every call through, and keeps a transcript of
/// it. Run a driver once against the real `limactl`, call
/// [`RecordingRunner::transcript`], save it with `Transcript::write_to`, and
/// the driver's tests replay it from then on through a `FakeRunner`.
///
/// The home folder is written as `{home}` and the temp folder as `{tmp}` (or
/// whatever `with_var` names), so a golden holds no real paths. Stdin is
/// recorded as text, so binary input is not kept exactly. Review a recorded
/// file before committing it: output can hold other details of the machine
/// it was recorded on.
pub struct RecordingRunner<R> {
    inner: R,
    tool: String,
    version: String,
    vars: Vec<(String, String)>,
    calls: Arc<Mutex<Vec<Call>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<R: Runner> RecordingRunner<R> {
    pub fn new(inner: R, tool: &str, version: &str) -> RecordingRunner<R> {
        RecordingRunner { inner, tool: tool.to_owned(), version: version.to_owned(), vars: default_vars(), calls: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Writes this folder as `{name}` in the transcript.
    #[must_use]
    pub fn with_var(mut self, name: &str, value: impl AsRef<std::path::Path>) -> RecordingRunner<R> {
        set_var(&mut self.vars, name, &value.as_ref().to_string_lossy());
        self
    }

    /// The calls so far.
    pub fn transcript(&self) -> Transcript {
        Transcript { tool: self.tool.clone(), version: self.version.clone(), calls: lock(&self.calls).clone() }
    }

    fn record(&self, cmd: &Command, stdin: Option<&[u8]>, stdout: &[u8], stderr: &[u8], status: Option<i32>) {
        record(&self.calls, &self.vars, cmd, stdin, stdout, stderr, status);
    }
}

fn record(calls: &Mutex<Vec<Call>>, vars: &[(String, String)], cmd: &Command, stdin: Option<&[u8]>, stdout: &[u8], stderr: &[u8], status: Option<i32>) {
    let text = |bytes: &[u8]| abstract_paths(&String::from_utf8_lossy(bytes), vars);
    lock(calls).push(Call {
        argv: cmd.argv().iter().map(|a| abstract_paths(a, vars)).collect(),
        stdin: stdin.map(text),
        stdout: text(stdout),
        stderr: text(stderr),
        // A signal-killed child has no code; 255 stands for "did not exit cleanly".
        status: status.unwrap_or(255),
    });
}

#[async_trait]
impl<R: Runner> Runner for RecordingRunner<R> {
    async fn run(&self, cmd: &Command, cancel: &Cancel) -> Result<Output, ExecError> {
        let out = self.inner.run(cmd, cancel).await?;
        self.record(cmd, cmd.stdin(), &out.stdout, &out.stderr, out.status);
        Ok(out)
    }

    async fn stream(&self, cmd: &Command, cancel: &Cancel, sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        let (tx, mut rx) = mpsc::channel(64);
        let forward = async {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            while let Some(chunk) = rx.recv().await {
                match &chunk {
                    Chunk::Stdout(b) => out.extend_from_slice(b),
                    Chunk::Stderr(b) => err.extend_from_slice(b),
                }
                let _ = sink.send(chunk).await;
            }
            (out, err)
        };
        let (result, (out, err)) = tokio::join!(self.inner.stream(cmd, cancel, tx), forward);
        let result = result?;
        self.record(cmd, cmd.stdin(), &out, &err, result.status);
        Ok(result)
    }

    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError> {
        let (reader, writer, control) = self.inner.duplex(cmd).await?.into_parts();
        let read_log = Arc::new(Mutex::new(Vec::new()));
        let write_log = Arc::new(Mutex::new(Vec::new()));
        let tee_reader = TeeReader { inner: reader, log: Arc::clone(&read_log) };
        let tee_writer = TeeWriter { inner: writer, log: Arc::clone(&write_log) };
        let control = RecordingControl { inner: control, calls: Arc::clone(&self.calls), vars: self.vars.clone(), cmd: cmd.clone(), read_log, write_log };
        Ok(Duplex::from_parts(Box::new(tee_reader), Box::new(tee_writer), Box::new(control)))
    }
}

struct TeeReader {
    inner: Box<dyn AsyncRead + Send + Unpin>,
    log: Arc<Mutex<Vec<u8>>>,
}

impl AsyncRead for TeeReader {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) {
            lock(&self.log).extend_from_slice(&buf.filled()[before..]);
        }
        polled
    }
}

struct TeeWriter {
    inner: Option<Box<dyn AsyncWrite + Send + Unpin>>,
    log: Arc<Mutex<Vec<u8>>>,
}

impl AsyncWrite for TeeWriter {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let Some(inner) = self.inner.as_mut() else { return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())) };
        let polled = Pin::new(inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &polled {
            lock(&self.log).extend_from_slice(&buf[..*n]);
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.inner.as_mut() {
            Some(inner) => Pin::new(inner).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(inner) = self.inner.as_mut() {
            match Pin::new(inner).poll_shutdown(cx) {
                Poll::Ready(Ok(())) => {}
                other => return other,
            }
        }
        self.inner = None;
        Poll::Ready(Ok(()))
    }
}

struct RecordingControl {
    inner: Box<dyn DuplexControl>,
    calls: Arc<Mutex<Vec<Call>>>,
    vars: Vec<(String, String)>,
    cmd: Command,
    read_log: Arc<Mutex<Vec<u8>>>,
    write_log: Arc<Mutex<Vec<u8>>>,
}

#[async_trait]
impl DuplexControl for RecordingControl {
    async fn finish(self: Box<Self>) -> Result<Output, ExecError> {
        let out = self.inner.finish().await?;
        let written = lock(&self.write_log).clone();
        let read = lock(&self.read_log).clone();
        record(&self.calls, &self.vars, &self.cmd, Some(&written), &read, &out.stderr, out.status);
        Ok(out)
    }

    fn kill(&mut self) {
        self.inner.kill();
    }
}
