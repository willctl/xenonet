//! The sending side of a move.

use crate::local::{now, Holding, Pending};
use crate::wire::{self, kind, Ack, Hello, Iam, Offer, Refused, SeatMsg};
use crate::{MoveError, MoveStats};
use eidolon_model::{HolderKey, Refusal, RefusalCode, SeatRecord, SignedSeat};
use eidolon_policy::{may_hold, may_open};
use eidolon_vault::{wrap_key, ChunkName, KeyRing, Vault};
use std::collections::BTreeSet;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite};
use xenonet_id::codec::{b64, random};
use xenonet_id::sign::Signer;

/// Moves the eidolon in `holding` to the machine at the other end of
/// `stream`.
///
/// The receiver must be on the spec's holder list and prove it holds its
/// key. Its chunks it lacks are sent as they sit in `vault`. Then
/// `pending.json` is written, and only then the seat and the key. On a valid
/// acknowledgement this machine's key is deleted and it retires.
///
/// If anything fails after the seat was sent, `holding.pending` stays set and
/// this copy stays closed: run the move again to the same machine, or settle
/// it with [`Holding::abandon_pending`] or [`Holding::confirm_moved`]. The one
/// exception is a refusal the receiver signed: it kept nothing, so the move
/// is undone here too.
pub async fn send<S>(mut stream: S, holding: &mut Holding, vault: &Vault, keyring: &KeyRing<'_>, signer: &dyn Signer) -> Result<MoveStats, MoveError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let started = Instant::now();
    let mut stats = MoveStats::default();
    let me = HolderKey::from_verifying_key(&signer.public()).ok_or_else(|| MoveError::Protocol("the machine's key is not an Ed25519 key".into()))?;
    let eid = holding.spec.id;

    // A pending move only continues to the machine it was for, and the
    // seat check ignores it: it is this move.
    may_open(&holding.state, &holding.seat, &me, false)?;

    let challenge = random::<32>().map_err(|e| MoveError::Protocol(e.message))?;
    stats.bytes_sent += wire::write_json(&mut stream, kind::HELLO, &Hello { protocol: wire::PROTOCOL.into(), eidolon: eid, challenge: b64(&challenge) }).await?;

    let (body, wire_in) = wire::read_expect(&mut stream, kind::IAM).await?;
    stats.bytes_received += wire_in;
    let iam: Iam = wire::parse(&body)?;
    let receiver = iam.holder;
    // The receiver is checked against this machine's own spec. A key it
    // sends for itself is never believed.
    let checked = may_hold(&holding.spec, &receiver).map_err(MoveError::Refused).and_then(|()| {
        if receiver == me {
            Err(MoveError::Refused(Refusal::new(RefusalCode::SecondSeat, "That is this machine.", "Move the eidolon to another machine on its holder list.")))
        } else if !wire::attestation_holds(&receiver, &iam.proof, &challenge, &eid, "iam", 0) {
            Err(MoveError::Refused(Refusal::new(RefusalCode::NotAHolder, "That machine did not prove it holds the key it named.", "Check which machine is on the other end.")))
        } else {
            Ok(())
        }
    });
    if let Err(MoveError::Refused(r)) = &checked {
        stats.bytes_sent += wire::write_json(&mut stream, kind::REFUSED, &Refused { refusal: r.clone(), proof: None }).await.unwrap_or(0);
    }
    checked?;
    if let Some(p) = holding.pending {
        if p.to != receiver {
            return Err(MoveError::MovePending { to: p.to, generation: p.generation });
        }
    }
    let seal = holding.spec.holder_by_key(&receiver).map(|h| h.seal).ok_or(MoveError::Protocol("the receiver left the holder list".into()))?;

    // What the receiver needs to see before it asks for chunks.
    if holding.head.len() > wire::MAX_HEAD_BYTES {
        return Err(MoveError::Protocol(format!("the head is {} bytes and the limit is {}", holding.head.len(), wire::MAX_HEAD_BYTES)));
    }
    let offered: Vec<ChunkName> = holding.chunks.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    stats.chunks_offered = offered.len() as u64;
    let offer = Offer { spec: holding.spec.clone(), seat: holding.seat.clone(), head: b64(&holding.head), chunks: offered.len() as u64 };
    stats.bytes_sent += wire::write_json(&mut stream, kind::OFFER, &offer).await?;
    stats.bytes_sent += wire::write_names(&mut stream, &offered).await?;

    let (want, wire_in) = wire::read_names(&mut stream, offered.len() as u64).await?;
    stats.bytes_received += wire_in;
    let offered_set: BTreeSet<ChunkName> = offered.iter().copied().collect();
    if want.iter().any(|n| !offered_set.contains(n)) {
        return Err(MoveError::Protocol("the receiver asked for a chunk that was not offered".into()));
    }
    let want: Vec<ChunkName> = want.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    for name in &want {
        let sealed = vault.get(name)?;
        let mut body = Vec::with_capacity(32 + sealed.len());
        body.extend_from_slice(&name.0);
        body.extend_from_slice(&sealed);
        stats.bytes_sent += wire::write_frame(&mut stream, kind::CHUNK, &body).await?;
        stats.chunks_sent += 1;
    }
    stats.bytes_sent += wire::write_frame(&mut stream, kind::SENT, &[]).await?;

    // From here the seat can be taken. Nothing is sent before the pending
    // move is on disk.
    let key = keyring.get(&eid.to_string())?.ok_or(MoveError::NoKey)?;
    let generation = holding.seat.record.generation + 1;
    let next = SignedSeat::sign(SeatRecord { eidolon: eid, generation, holder: receiver, previous: Some(me), at: now() }, signer)?;
    let wrapped = wrap_key(&key, seal, &wire::wrap_context(&eid, generation))?;
    holding.begin_move(Pending { to: receiver, generation })?;
    stats.bytes_sent += wire::write_json(&mut stream, kind::SEAT, &SeatMsg { next: next.clone(), key: wrapped }).await?;

    let (got, body, wire_in) = wire::read_frame(&mut stream).await?;
    stats.bytes_received += wire_in;
    match got {
        kind::ACK => {
            let ack: Ack = wire::parse(&body)?;
            if ack.generation != generation || !wire::attestation_holds(&receiver, &ack.proof, &challenge, &eid, "ack", generation) {
                return Err(MoveError::Protocol("the acknowledgement is not the receiver's for this move".into()));
            }
            holding.retire(keyring, &next)?;
        }
        kind::REFUSED => {
            let refused: Refused = wire::parse(&body)?;
            // Only the receiver's own signature clears the pending move.
            if refused.proof.as_deref().is_some_and(|p| wire::attestation_holds(&receiver, p, &challenge, &eid, "refused", generation)) {
                holding.abandon_pending()?;
            }
            return Err(MoveError::RefusedByPeer(refused.refusal));
        }
        other => return Err(MoveError::Protocol(format!("expected an acknowledgement and got a frame of type {other}"))),
    }
    stats.elapsed = started.elapsed();
    Ok(stats)
}
