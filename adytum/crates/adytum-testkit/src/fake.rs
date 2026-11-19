use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use adytum_exec::{Cancel, Chunk, Command, Duplex, DuplexControl, ExecError, Output, Runner};
use async_trait::async_trait;
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

use crate::transcript::{default_vars, set_var, substitute, Call, Transcript, TranscriptError};

/// A `Runner` that replays a golden transcript, in order.
///
/// A call is matched on the program's file name and its arguments. A call
/// that doesn't match the next one in the transcript returns an error naming
/// both, and is remembered: [`FakeRunner::assert_done`] fails for it even if
/// the code under test swallowed the error.
pub struct FakeRunner {
    state: Mutex<State>,
}

struct State {
    tool: String,
    version: String,
    pending: VecDeque<Call>,
    taken: usize,
    vars: Vec<(String, String)>,
    failures: Vec<String>,
    writes: Vec<Arc<Mutex<Vec<u8>>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeRunner {
    pub fn from_transcript(transcript: Transcript) -> FakeRunner {
        FakeRunner {
            state: Mutex::new(State {
                tool: transcript.tool,
                version: transcript.version,
                pending: transcript.calls.into(),
                taken: 0,
                vars: default_vars(),
                failures: Vec::new(),
                writes: Vec::new(),
            }),
        }
    }

    pub fn from_json(text: &str) -> Result<FakeRunner, TranscriptError> {
        Transcript::from_json(text).map(FakeRunner::from_transcript)
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<FakeRunner, TranscriptError> {
        Transcript::from_file(path.as_ref()).map(FakeRunner::from_transcript)
    }

    /// Sets what `{name}` stands for in the transcript. `home` and `tmp`
    /// start as this machine's home and temp folders.
    #[must_use]
    pub fn with_var(self, name: &str, value: impl AsRef<Path>) -> FakeRunner {
        set_var(&mut lock(&self.state).vars, name, &value.as_ref().to_string_lossy());
        self
    }

    /// Fails if calls are still expected, or if any call didn't match.
    pub fn assert_done(&self) {
        let st = lock(&self.state);
        assert!(st.failures.is_empty(), "{} call(s) did not match the {} transcript:\n{}", st.failures.len(), st.tool, st.failures.join("\n"));
        assert!(
            st.pending.is_empty(),
            "{} call(s) of the {} {} transcript never happened. Next expected: {}",
            st.pending.len(),
            st.tool,
            st.version,
            st.pending.front().map(|c| c.argv.join(" ")).unwrap_or_default()
        );
    }

    /// What was written to each `duplex` stream so far, in call order.
    pub fn duplex_writes(&self) -> Vec<Vec<u8>> {
        lock(&self.state).writes.iter().map(|w| lock(w).clone()).collect()
    }

    /// Matches `cmd` against the next call and consumes it. A mismatch leaves
    /// the call in place.
    fn next(&self, cmd: &Command) -> Result<Call, ExecError> {
        let mut st = lock(&self.state);
        let n = st.taken + 1;
        let mut actual = cmd.argv();
        actual[0] = base_name(&actual[0]);
        let Some(expected) = st.pending.front().cloned() else {
            let message = format!("Unexpected call {n} to the {} fake: the transcript has no calls left.\n  actual: {}", st.tool, actual.join(" "));
            st.failures.push(message.clone());
            return Err(ExecError::Unexpected { message });
        };
        let want: Vec<String> = expected.argv.iter().enumerate().map(|(i, a)| if i == 0 { base_name(a) } else { substitute(a, &st.vars) }).collect();
        let stdin_actual = cmd.stdin().map(|b| String::from_utf8_lossy(b).into_owned());
        let stdin_want = expected.stdin.as_ref().map(|s| substitute(s, &st.vars));
        let argv_ok = want.len() == actual.len() && want.iter().zip(&actual).all(|(w, a)| w == "{any}" || w == a);
        let stdin_ok = stdin_want.is_none() || stdin_want == stdin_actual;
        if !(argv_ok && stdin_ok) {
            let message = diff(n, &st.tool, &st.version, &want, &actual, (!stdin_ok).then_some((&stdin_want, &stdin_actual)));
            st.failures.push(message.clone());
            return Err(ExecError::Unexpected { message });
        }
        st.pending.pop_front();
        st.taken += 1;
        let vars = st.vars.clone();
        Ok(Call { argv: want, stdin: stdin_want, stdout: substitute(&expected.stdout, &vars), stderr: substitute(&expected.stderr, &vars), status: expected.status })
    }
}

fn base_name(program: &str) -> String {
    Path::new(program).file_name().map_or_else(|| program.to_owned(), |n| n.to_string_lossy().into_owned())
}

fn diff(n: usize, tool: &str, version: &str, want: &[String], actual: &[String], stdin: Option<(&Option<String>, &Option<String>)>) -> String {
    let mut out = format!("Call {n} to the {tool} {version} fake did not match the transcript.\n  expected: {}\n  actual:   {}", want.join(" "), actual.join(" "));
    if let Some(i) =
        (0..want.len().max(actual.len())).find(|&i| want.get(i).map(String::as_str) != actual.get(i).map(String::as_str) && want.get(i).map(String::as_str) != Some("{any}"))
    {
        let w = want.get(i).map_or("(nothing)", String::as_str);
        let a = actual.get(i).map_or("(nothing)", String::as_str);
        let _ = write!(out, "\n  first difference at position {i}: expected {w:?}, got {a:?}");
    }
    if let Some((w, a)) = stdin {
        let _ = write!(out, "\n  stdin: expected {w:?}, got {a:?}");
    }
    out
}

fn output_of(call: &Call) -> Output {
    Output { status: Some(call.status), stdout: call.stdout.clone().into_bytes(), stderr: call.stderr.clone().into_bytes(), timed_out: false, cancelled: false }
}

#[async_trait]
impl Runner for FakeRunner {
    async fn run(&self, cmd: &Command, _cancel: &Cancel) -> Result<Output, ExecError> {
        Ok(output_of(&self.next(cmd)?))
    }

    async fn stream(&self, cmd: &Command, _cancel: &Cancel, sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        let call = self.next(cmd)?;
        if !call.stdout.is_empty() {
            let _ = sink.send(Chunk::Stdout(call.stdout.clone().into_bytes())).await;
        }
        if !call.stderr.is_empty() {
            let _ = sink.send(Chunk::Stderr(call.stderr.clone().into_bytes())).await;
        }
        Ok(Output { status: Some(call.status), ..Output::default() })
    }

    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError> {
        let call = self.next(cmd)?;
        let written = Arc::new(Mutex::new(Vec::new()));
        lock(&self.state).writes.push(Arc::clone(&written));
        let control = ReplayControl { status: call.status, stderr: call.stderr.clone().into_bytes() };
        Ok(Duplex::from_parts(Box::new(io::Cursor::new(call.stdout.into_bytes())), Box::new(Sink(written)), Box::new(control)))
    }
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl AsyncWrite for Sink {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        lock(&self.0).extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

struct ReplayControl {
    status: i32,
    stderr: Vec<u8>,
}

#[async_trait]
impl DuplexControl for ReplayControl {
    async fn finish(self: Box<Self>) -> Result<Output, ExecError> {
        Ok(Output { status: Some(self.status), stderr: self.stderr, ..Output::default() })
    }

    fn kill(&mut self) {}
}
