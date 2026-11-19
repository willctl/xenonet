use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{ExecError, Output};

/// What a duplex needs from whoever made it, once the bytes are done: wait
/// for the child (or its replay) and stop it early.
#[async_trait]
pub trait DuplexControl: Send {
    /// Waits for the child to end. The reader and writer are already closed.
    async fn finish(self: Box<Self>) -> Result<Output, ExecError>;

    /// Stops the child (and its process group on unix) now.
    fn kill(&mut self);
}

/// A child's stdin and stdout as one byte stream.
///
/// Writes go to stdin, reads come from stdout. `poll_shutdown` (and
/// `AsyncWriteExt::shutdown`) closes stdin, so the child sees end of input
/// while its output can still be read. Split it with `tokio::io::split` to
/// read and write at once, and `unsplit` before [`Duplex::finish`].
pub struct Duplex {
    reader: Box<dyn AsyncRead + Send + Unpin>,
    writer: Option<Box<dyn AsyncWrite + Send + Unpin>>,
    control: Box<dyn DuplexControl>,
}

impl Duplex {
    pub fn from_parts(reader: Box<dyn AsyncRead + Send + Unpin>, writer: Box<dyn AsyncWrite + Send + Unpin>, control: Box<dyn DuplexControl>) -> Duplex {
        Duplex { reader, writer: Some(writer), control }
    }

    /// Closes stdin and stdout, then waits for the child. Closing stdout
    /// means a child still writing gets a broken pipe rather than blocking
    /// this call forever. A command deadline applies from here.
    pub async fn finish(self) -> Result<Output, ExecError> {
        let Duplex { reader, writer, control } = self;
        drop(writer);
        drop(reader);
        control.finish().await
    }

    /// Stops the child now.
    pub fn kill(&mut self) {
        self.control.kill();
    }
}

impl std::fmt::Debug for Duplex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Duplex").field("stdin_open", &self.writer.is_some()).finish_non_exhaustive()
    }
}

impl AsyncRead for Duplex {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the child's stdin is closed")
}

impl AsyncWrite for Duplex {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.writer.as_mut() {
            Some(w) => Pin::new(w).poll_write(cx, buf),
            None => Poll::Ready(Err(closed())),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.writer.as_mut() {
            Some(w) => Pin::new(w).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(w) = self.writer.as_mut() {
            match Pin::new(w).poll_shutdown(cx) {
                Poll::Ready(Ok(())) => {}
                other => return other,
            }
        }
        // Dropping the pipe is what closes it; a child pipe's own shutdown
        // does nothing.
        self.writer = None;
        Poll::Ready(Ok(()))
    }
}
