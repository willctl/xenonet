//! Frames and messages. A frame is a u32 big-endian length (the type byte
//! and the body), one type byte, then the body: JSON for control messages,
//! raw bytes for chunk names and chunks.

use crate::MoveError;
use eidolon_model::{EidolonId, EidolonSpec, HolderKey, Refusal, SignedSeat};
use eidolon_vault::{ChunkName, WrappedKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use xenonet_id::codec::{b64, unb64, unb64_n};
use xenonet_id::sign::{Purpose, Signer};

pub const PROTOCOL: &str = "eidolon-move/1";

/// The most a frame's length may say.
pub const MAX_FRAME: usize = 2 << 20;

/// The most a head may weigh: its JSON text must fit a frame.
pub const MAX_HEAD_BYTES: usize = 1 << 20;

/// Names in one `Names` frame.
pub const NAMES_PER_FRAME: usize = 8192;

pub mod kind {
    pub const HELLO: u8 = 1;
    pub const IAM: u8 = 2;
    pub const REFUSED: u8 = 3;
    pub const OFFER: u8 = 4;
    pub const NAMES: u8 = 5;
    pub const CHUNK: u8 = 6;
    pub const SENT: u8 = 7;
    pub const SEAT: u8 = 8;
    pub const ACK: u8 = 9;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: String,
    pub eidolon: EidolonId,
    /// base64url of 32 random bytes.
    pub challenge: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Iam {
    pub holder: HolderKey,
    pub proof: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Refused {
    pub refusal: Refusal,
    /// A refusal sent after the seat was handed over carries the
    /// receiver's signature, so the sender can tell it from a forged one
    /// before it opens its own copy again.
    pub proof: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Offer {
    pub spec: EidolonSpec,
    pub seat: SignedSeat,
    /// base64url of the head.
    pub head: String,
    /// How many names follow in `Names` frames.
    pub chunks: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeatMsg {
    pub next: SignedSeat,
    pub key: WrappedKey,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ack {
    pub generation: u64,
    pub proof: String,
}

/// Writes one frame; returns the bytes it put on the wire.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, kind: u8, body: &[u8]) -> Result<u64, MoveError> {
    let len = body.len() + 1;
    if len > MAX_FRAME {
        return Err(MoveError::Protocol(format!("a frame of {len} bytes is over the {MAX_FRAME} limit")));
    }
    let prefix = u32::try_from(len).map_err(|_| MoveError::Protocol("a frame is too long".into()))?;
    let mut buf = Vec::with_capacity(len + 4);
    buf.extend_from_slice(&prefix.to_be_bytes());
    buf.push(kind);
    buf.extend_from_slice(body);
    w.write_all(&buf).await.map_err(MoveError::from_stream)?;
    w.flush().await.map_err(MoveError::from_stream)?;
    Ok(buf.len() as u64)
}

pub async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, kind: u8, value: &T) -> Result<u64, MoveError> {
    let body = serde_json::to_vec(value).map_err(|e| MoveError::Protocol(format!("could not write a message: {e}")))?;
    write_frame(w, kind, &body).await
}

/// One frame: its type, its body and the bytes it took on the wire.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>, u64), MoveError> {
    let mut prefix = [0u8; 4];
    r.read_exact(&mut prefix).await.map_err(MoveError::from_stream)?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(MoveError::Protocol(format!("a frame says it is {len} bytes long")));
    }
    let mut rest = vec![0u8; len];
    r.read_exact(&mut rest).await.map_err(MoveError::from_stream)?;
    let body = rest.split_off(1);
    Ok((rest[0], body, len as u64 + 4))
}

pub fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, MoveError> {
    serde_json::from_slice(body).map_err(|e| MoveError::Protocol(format!("a message did not parse: {e}")))
}

/// The next frame, which must be of type `want`; the other side's refusal
/// comes back as that refusal.
pub async fn read_expect<R: AsyncRead + Unpin>(r: &mut R, want: u8) -> Result<(Vec<u8>, u64), MoveError> {
    let (got, body, wire) = read_frame(r).await?;
    if got == kind::REFUSED {
        return Err(MoveError::RefusedByPeer(parse::<Refused>(&body)?.refusal));
    }
    if got != want {
        return Err(MoveError::Protocol(format!("expected a frame of type {want} and got {got}")));
    }
    Ok((body, wire))
}

/// A list of chunk names as `Names` frames: a byte that is 1 on the last
/// frame, then 32 raw bytes per name. Returns the wire bytes.
pub async fn write_names<W: AsyncWrite + Unpin>(w: &mut W, names: &[ChunkName]) -> Result<u64, MoveError> {
    let mut wire = 0;
    let mut batches = names.chunks(NAMES_PER_FRAME).peekable();
    if batches.peek().is_none() {
        return write_frame(w, kind::NAMES, &[1]).await;
    }
    while let Some(batch) = batches.next() {
        let mut body = Vec::with_capacity(1 + batch.len() * 32);
        body.push(u8::from(batches.peek().is_none()));
        for n in batch {
            body.extend_from_slice(&n.0);
        }
        wire += write_frame(w, kind::NAMES, &body).await?;
    }
    Ok(wire)
}

/// Reads `Names` frames up to the last one, at most `limit` names.
pub async fn read_names<R: AsyncRead + Unpin>(r: &mut R, limit: u64) -> Result<(Vec<ChunkName>, u64), MoveError> {
    let (mut names, mut wire) = (Vec::new(), 0);
    loop {
        let (body, w) = read_expect(r, kind::NAMES).await?;
        wire += w;
        let (&last, rest) = body.split_first().ok_or_else(|| MoveError::Protocol("an empty names frame".into()))?;
        if rest.len() % 32 != 0 {
            return Err(MoveError::Protocol("a names frame is not whole names".into()));
        }
        names.extend(rest.as_chunks::<32>().0.iter().map(|n| ChunkName(*n)));
        if names.len() as u64 > limit {
            return Err(MoveError::Protocol("more names than were announced".into()));
        }
        if last == 1 {
            return Ok((names, wire));
        }
        if last != 0 {
            return Err(MoveError::Protocol("a names frame has a bad last flag".into()));
        }
    }
}

/// What a machine signs to say who it is, or that it took or refused the
/// seat of a given generation. The challenge is the sender's, new each run,
/// so no signature is good for another run.
pub fn attestation(challenge: &[u8; 32], eidolon: &EidolonId, tag: &str, generation: u64) -> Vec<u8> {
    let mut msg = Vec::new();
    msg.extend_from_slice(challenge);
    msg.extend_from_slice(eidolon.to_string().as_bytes());
    msg.push(0);
    msg.extend_from_slice(tag.as_bytes());
    msg.extend_from_slice(&generation.to_be_bytes());
    msg
}

pub fn attest(signer: &dyn Signer, challenge: &[u8; 32], eidolon: &EidolonId, tag: &str, generation: u64) -> Result<String, MoveError> {
    let sig = signer.sign(Purpose::EIDOLON_MOVE, &attestation(challenge, eidolon, tag, generation)).map_err(|e| eidolon_model::ModelError::Sign(e.message))?;
    Ok(b64(&sig))
}

pub fn attestation_holds(key: &HolderKey, proof: &str, challenge: &[u8; 32], eidolon: &EidolonId, tag: &str, generation: u64) -> bool {
    unb64(proof).is_some_and(|sig| key.verifying_key().verify(Purpose::EIDOLON_MOVE, &attestation(challenge, eidolon, tag, generation), &sig))
}

pub fn challenge_of(hello: &Hello) -> Result<[u8; 32], MoveError> {
    unb64_n::<32>(&hello.challenge).ok_or_else(|| MoveError::Protocol("the challenge is not 32 bytes".into()))
}

/// The key wrapping binds the eidolon and the generation.
pub fn wrap_context(eidolon: &EidolonId, generation: u64) -> Vec<u8> {
    let mut c = eidolon.to_string().into_bytes();
    c.extend_from_slice(&generation.to_be_bytes());
    c
}
