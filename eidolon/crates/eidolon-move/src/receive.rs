//! The receiving side of a move.

use crate::local::Holding;
use crate::wire::{self, kind, Ack, Hello, Iam, Offer, Refused, SeatMsg};
use crate::{MoveError, MoveStats};
use eidolon_model::{verify, EidolonId, HolderKey, Refusal, RefusalCode, SignedSeat};
use eidolon_policy::{accept_seat, may_hold};
use eidolon_vault::{unwrap_key, ChunkName, KeyRing, Keys, Vault, VaultError};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite};
use x25519_dalek::StaticSecret;
use xenonet_id::codec::unb64;
use xenonet_id::sign::Signer;

/// Takes an eidolon from the machine at the other end of `stream` into
/// `root`.
///
/// `me` is this machine's signing key and `seal_secret` the secret of the
/// X25519 key listed beside it in the eidolon's holder list. Chunks are kept
/// as they arrive, unchecked, because the key comes last; a chunk that then
/// fails to open is removed again and the move is refused, with no key and no
/// seat kept. A broken transfer resumes: the next run asks only for what the
/// vault still lacks.
pub async fn receive<S>(
    mut stream: S,
    root: &Path,
    vault: &Vault,
    keyring: &KeyRing<'_>,
    signer: &dyn Signer,
    me: &HolderKey,
    seal_secret: &StaticSecret,
) -> Result<MoveStats, MoveError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let started = Instant::now();
    let mut stats = MoveStats::default();
    if HolderKey::from_verifying_key(&signer.public()) != Some(*me) {
        return Err(MoveError::Protocol("the signer is not the key this machine is listed under".into()));
    }

    let (body, wire_in) = wire::read_expect(&mut stream, kind::HELLO).await?;
    stats.bytes_received += wire_in;
    let hello: Hello = wire::parse(&body)?;
    if hello.protocol != wire::PROTOCOL {
        return Err(MoveError::Protocol(format!("the sender speaks {:?} and this machine speaks {:?}", hello.protocol, wire::PROTOCOL)));
    }
    let challenge = wire::challenge_of(&hello)?;
    let eid = hello.eidolon;
    let proof = wire::attest(signer, &challenge, &eid, "iam", 0)?;
    stats.bytes_sent += wire::write_json(&mut stream, kind::IAM, &Iam { holder: *me, proof }).await?;

    let (body, wire_in) = wire::read_expect(&mut stream, kind::OFFER).await?;
    stats.bytes_received += wire_in;
    let offer: Offer = wire::parse(&body)?;
    let head = unb64(&offer.head).ok_or_else(|| MoveError::Protocol("the head is not base64url".into()))?;

    let current = match check_offer(&offer, &eid, me, root) {
        Ok(current) => current,
        Err(MoveError::Refused(r)) => return Err(refuse(&mut stream, r, None).await),
        Err(e) => return Err(e),
    };

    let (names, wire_in) = wire::read_names(&mut stream, offer.chunks).await?;
    stats.bytes_received += wire_in;
    if names.len() as u64 != offer.chunks {
        return Err(MoveError::Protocol("fewer names than were announced".into()));
    }
    let names: Vec<ChunkName> = names.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    stats.chunks_offered = names.len() as u64;
    let want: Vec<ChunkName> = names.iter().copied().filter(|n| !vault.has(n)).collect();
    stats.bytes_sent += wire::write_names(&mut stream, &want).await?;

    let mut missing: BTreeSet<ChunkName> = want.into_iter().collect();
    loop {
        let (got, body, wire_in) = wire::read_frame(&mut stream).await?;
        stats.bytes_received += wire_in;
        match got {
            kind::CHUNK => {
                let (raw, sealed) = body.split_at_checked(32).ok_or_else(|| MoveError::Protocol("a chunk frame is shorter than a name".into()))?;
                let mut name = [0u8; 32];
                name.copy_from_slice(raw);
                if !missing.remove(&ChunkName(name)) {
                    return Err(MoveError::Protocol("a chunk arrived that was not asked for".into()));
                }
                vault.put(&ChunkName(name), sealed)?;
                stats.chunks_sent += 1;
            }
            kind::SENT => break,
            kind::REFUSED => return Err(MoveError::RefusedByPeer(wire::parse::<Refused>(&body)?.refusal)),
            other => return Err(MoveError::Protocol(format!("expected a chunk and got a frame of type {other}"))),
        }
    }
    if !missing.is_empty() {
        return Err(MoveError::Protocol("the sender stopped sending before all the asked-for chunks came".into()));
    }

    let (body, wire_in) = wire::read_expect(&mut stream, kind::SEAT).await?;
    stats.bytes_received += wire_in;
    let seat_msg: SeatMsg = wire::parse(&body)?;
    let generation = seat_msg.next.record.generation;

    // From here a refusal is signed: the sender has written its pending
    // move and can tell this one is the receiver's, and that it kept
    // nothing. A machine that already holds this generation or a later one
    // does not sign, so a repeated move cannot undo itself.
    let holds_it = Holding::exists(root, &eid) && Holding::load(root, &eid)?.seat.record.generation >= generation;
    let outcome = take(&offer, &head, &names, &current, &seat_msg, root, vault, keyring, seal_secret, me);
    match outcome {
        Ok(()) => {
            let proof = wire::attest(signer, &challenge, &eid, "ack", generation)?;
            stats.bytes_sent += wire::write_json(&mut stream, kind::ACK, &Ack { generation, proof }).await?;
        }
        Err(MoveError::Refused(r)) => {
            let proof = if holds_it { None } else { Some(wire::attest(signer, &challenge, &eid, "refused", generation)?) };
            return Err(refuse(&mut stream, r, proof).await);
        }
        Err(e) => return Err(e),
    }
    stats.elapsed = started.elapsed();
    Ok(stats)
}

async fn refuse<S: AsyncWrite + Unpin>(stream: &mut S, refusal: Refusal, proof: Option<String>) -> MoveError {
    // The sender may already be gone; the refusal is returned either way.
    let _ = wire::write_json(stream, kind::REFUSED, &Refused { refusal: refusal.clone(), proof }).await;
    MoveError::Refused(refusal)
}

fn refusal(code: RefusalCode, message: &str, fix: &str) -> MoveError {
    MoveError::Refused(Refusal::new(code, message, fix))
}

/// The offer's spec and seat against this machine's rules and what it
/// already holds. Returns the seat the next one must follow.
fn check_offer(offer: &Offer, eid: &EidolonId, me: &HolderKey, root: &Path) -> Result<SignedSeat, MoveError> {
    let spec = &offer.spec;
    if spec.id != *eid || offer.seat.record.eidolon != *eid {
        return Err(MoveError::Protocol("the offer is for another eidolon than the hello named".into()));
    }
    spec.validate()?;
    may_hold(spec, me)?;

    let local = if Holding::exists(root, eid) { Some(Holding::load(root, eid)?) } else { None };
    if let Some(local) = &local {
        if local.spec.wall != spec.wall {
            return Err(refusal(
                RefusalCode::WrongWall,
                "This machine holds that eidolon behind another wall.",
                "Check which eidolon you are moving; the id is already used here for another.",
            ));
        }
        if local.spec.holders != spec.holders {
            return Err(refusal(
                RefusalCode::NotAHolder,
                "The holder list differs from the one this machine holds.",
                "Update the holder list on both machines the same way, then move again.",
            ));
        }
        if !matches!(local.state, eidolon_model::State::Sealed | eidolon_model::State::Retired) {
            return Err(refusal(RefusalCode::SecondSeat, "That eidolon is open on this machine.", "Close it here first."));
        }
    }

    // A seat the sender shows is believed only as far as its signature:
    // by the machine it names as previous, or by its own holder at
    // generation 1; both on the holder list.
    let offered = &offer.seat;
    let signer = offered.record.previous.unwrap_or(offered.record.holder);
    may_hold(spec, &signer)?;
    may_hold(spec, &offered.record.holder)?;
    if !verify(offered, &signer) {
        return Err(refusal(
            RefusalCode::SeatNotSignedByHolder,
            "The seat the sender showed is not signed by a machine on the holder list.",
            "Move the eidolon from the machine that holds it.",
        ));
    }
    match local {
        Some(local) if local.seat.record.generation > offered.record.generation => Ok(local.seat),
        Some(local) if local.seat.record.generation == offered.record.generation => {
            if local.seat.record == offered.record {
                Ok(local.seat)
            } else {
                Err(refusal(
                    RefusalCode::SecondSeat,
                    "The sender's seat and this machine's differ at the same generation.",
                    "Two machines claim the same seat; decide which one holds the eidolon and fix the other.",
                ))
            }
        }
        _ => Ok(offered.clone()),
    }
}

/// Checks the seat, opens the key and every chunk, and only then keeps
/// anything.
#[expect(clippy::too_many_arguments, reason = "each is a separate thing the move has in hand")]
fn take(
    offer: &Offer,
    head: &[u8],
    names: &[ChunkName],
    current: &SignedSeat,
    msg: &SeatMsg,
    root: &Path,
    vault: &Vault,
    keyring: &KeyRing<'_>,
    seal_secret: &StaticSecret,
    me: &HolderKey,
) -> Result<(), MoveError> {
    let next = &msg.next;
    if next.record.holder != *me {
        return Err(refusal(RefusalCode::NotAHolder, "That seat names another machine.", "Move the eidolon to the machine that is meant to take it."));
    }
    accept_seat(&offer.spec, Some(current), next)?;
    let key = unwrap_key(&msg.key, seal_secret, &wire::wrap_context(&offer.spec.id, next.record.generation))
        .map_err(|_| refusal(RefusalCode::NotAHolder, "The key was not wrapped for this machine.", "Check that the holder list has this machine's current seal key."))?;
    let keys = Keys::new(&key)?;
    let mut changed = Vec::new();
    for name in names {
        match vault.get(name).and_then(|sealed| keys.open(name, &sealed)) {
            Ok(_) => {}
            Err(VaultError::ChangedChunk | VaultError::ChunkTooLarge | VaultError::MissingChunk(_)) => changed.push(*name),
            Err(e) => return Err(e.into()),
        }
    }
    if !changed.is_empty() {
        // Gone, so the next run fetches them again instead of trusting them.
        for name in &changed {
            vault.remove(name)?;
        }
        return Err(refusal(
            RefusalCode::ChangedChunk,
            &format!("{} chunk(s) did not open or did not match their names.", changed.len()),
            "Run the move again; the damaged chunks were dropped and will be sent again.",
        ));
    }
    keyring.put(&offer.spec.id.to_string(), &key)?;
    Holding::install(root, offer.spec.clone(), next.clone(), head.to_vec(), names.to_vec())?;
    Ok(())
}
