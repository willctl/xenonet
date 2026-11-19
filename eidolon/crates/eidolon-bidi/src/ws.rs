//! WebSocket messages over any byte stream: reassembling fragments on the
//! way in, masking on the way out. Used by the client and, with the other
//! role, by the fake server in `eidolon-testkit`.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::Error;
use crate::frame::{self, Frame, Opcode};

/// Which end of the connection this is. A client masks what it sends and
/// refuses masked frames; a server does the reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// A whole message, or a control frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close { code: Option<u16>, reason: String },
}

/// The read half.
pub struct Reader {
    r: BoxRead,
    buf: Vec<u8>,
    role: Role,
    max_message: usize,
    partial: Option<(Opcode, Vec<u8>)>,
}

impl Reader {
    /// `leftover` is what the handshake read past the HTTP head.
    pub fn new(r: BoxRead, role: Role, leftover: Vec<u8>, max_message: usize) -> Self {
        Self { r, buf: leftover, role, max_message, partial: None }
    }

    /// The next message. Fragments are joined; control frames come out as
    /// they arrive, even between fragments.
    pub async fn next(&mut self) -> Result<Message, Error> {
        loop {
            match frame::parse_with_limit(&self.buf, self.max_message)? {
                Some((frame, used)) => {
                    self.buf.drain(..used);
                    if let Some(message) = self.accept(frame)? {
                        return Ok(message);
                    }
                }
                None => {
                    self.buf.reserve(16 * 1024);
                    if self.r.read_buf(&mut self.buf).await? == 0 {
                        return Err(Error::Closed("the stream ended without a close message".into()));
                    }
                }
            }
        }
    }

    fn accept(&mut self, frame: Frame) -> Result<Option<Message>, Error> {
        match (self.role, frame.masked) {
            (Role::Client, true) => return Err(Error::Protocol("the server sent a masked frame".into())),
            (Role::Server, false) => return Err(Error::Protocol("the client sent an unmasked frame".into())),
            _ => {}
        }
        match frame.opcode {
            Opcode::Ping => Ok(Some(Message::Ping(frame.payload))),
            Opcode::Pong => Ok(Some(Message::Pong(frame.payload))),
            Opcode::Close => Ok(Some(close_message(&frame.payload)?)),
            Opcode::Text | Opcode::Binary => {
                if self.partial.is_some() {
                    return Err(Error::Protocol("a new message began before the last one ended".into()));
                }
                if frame.fin {
                    finish(frame.opcode, frame.payload).map(Some)
                } else {
                    self.partial = Some((frame.opcode, frame.payload));
                    Ok(None)
                }
            }
            Opcode::Continuation => {
                let Some((opcode, mut data)) = self.partial.take() else {
                    return Err(Error::Protocol("a continuation frame arrived with nothing to continue".into()));
                };
                if data.len() + frame.payload.len() > self.max_message {
                    return Err(Error::MessageTooLarge { limit: self.max_message });
                }
                data.extend_from_slice(&frame.payload);
                if frame.fin {
                    finish(opcode, data).map(Some)
                } else {
                    self.partial = Some((opcode, data));
                    Ok(None)
                }
            }
        }
    }
}

fn finish(opcode: Opcode, data: Vec<u8>) -> Result<Message, Error> {
    if opcode == Opcode::Text {
        let text = String::from_utf8(data).map_err(|_| Error::Protocol("a text message is not valid UTF-8".into()))?;
        Ok(Message::Text(text))
    } else {
        Ok(Message::Binary(data))
    }
}

fn close_message(payload: &[u8]) -> Result<Message, Error> {
    match payload {
        [] => Ok(Message::Close { code: None, reason: String::new() }),
        [_] => Err(Error::Protocol("a close frame has a one-byte payload".into())),
        [a, b, reason @ ..] => Ok(Message::Close { code: Some(u16::from_be_bytes([*a, *b])), reason: String::from_utf8_lossy(reason).into_owned() }),
    }
}

/// The write half.
pub struct Writer {
    w: BoxWrite,
    role: Role,
}

impl Writer {
    pub fn new(w: BoxWrite, role: Role) -> Self {
        Self { w, role }
    }

    /// Sends one frame as given (masked when this is a client).
    pub async fn send_frame(&mut self, fin: bool, opcode: Opcode, payload: &[u8]) -> Result<(), Error> {
        let mask = match self.role {
            Role::Client => {
                let mut m = [0u8; 4];
                getrandom::fill(&mut m).map_err(|e| Error::Io(format!("no random bytes for the mask: {e}")))?;
                Some(m)
            }
            Role::Server => None,
        };
        let bytes = frame::encode(fin, opcode, payload, mask);
        self.write_raw(&bytes).await
    }

    /// Writes bytes as they are. For tests that send what a correct peer
    /// would not.
    pub async fn write_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.w.write_all(bytes).await?;
        self.w.flush().await?;
        Ok(())
    }

    pub async fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.send_frame(true, Opcode::Text, text.as_bytes()).await
    }

    pub async fn send_pong(&mut self, payload: &[u8]) -> Result<(), Error> {
        self.send_frame(true, Opcode::Pong, payload).await
    }

    pub async fn send_close(&mut self, code: u16, reason: &str) -> Result<(), Error> {
        let mut payload = code.to_be_bytes().to_vec();
        // A control frame holds 125 bytes at most.
        let mut cut = reason.len().min(123);
        while !reason.is_char_boundary(cut) {
            cut -= 1;
        }
        payload.extend_from_slice(&reason.as_bytes()[..cut]);
        self.send_frame(true, Opcode::Close, &payload).await
    }

    /// Ends the write side of the stream.
    pub async fn shutdown(&mut self) {
        // The peer may be gone already; there is nothing to do about that.
        let _ = self.w.shutdown().await;
    }
}
