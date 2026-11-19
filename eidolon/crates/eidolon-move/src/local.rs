//! What a machine keeps about one eidolon, in `<root>/<eid>/`:
//!
//! - `spec.json`: the eidolon's spec;
//! - `seat.json`: the latest seat this machine knows;
//! - `head`: the opaque signed head (the manifest, to `eidolon-pack`);
//! - `chunks.json`: the chunk names the head needs;
//! - `state.json`: where the eidolon is now;
//! - `pending.json`: only while a move away from here is unfinished.
//!
//! Every file is written whole to a private temp file and renamed over the
//! old one. The seat is written last when a holding is installed, so a
//! holding without a seat is one that never finished arriving and is
//! ignored.

use crate::MoveError;
use eidolon_model::{EidolonId, EidolonSpec, HolderKey, Refusal, RefusalCode, SeatRecord, SignedSeat, State};
use eidolon_policy::{accept_seat, may_hold, may_open};
use eidolon_vault::{ChunkName, KeyRing};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use xenonet_id::sign::Signer;

/// A move away from this machine that was started and not finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub to: HolderKey,
    /// The generation of the next seat, the one that names `to`.
    pub generation: u64,
}

/// One eidolon as this machine holds it.
#[derive(Debug)]
pub struct Holding {
    dir: PathBuf,
    pub spec: EidolonSpec,
    pub seat: SignedSeat,
    pub head: Vec<u8>,
    pub chunks: Vec<ChunkName>,
    pub state: State,
    pub pending: Option<Pending>,
}

/// The first seat of a new eidolon: generation 1, signed by its own holder,
/// which must be on the spec's list.
pub fn seat_first(spec: &EidolonSpec, signer: &dyn Signer) -> Result<SignedSeat, MoveError> {
    let holder = HolderKey::from_verifying_key(&signer.public()).ok_or_else(|| MoveError::Protocol("the machine's key is not an Ed25519 key".into()))?;
    may_hold(spec, &holder)?;
    let record = SeatRecord { eidolon: spec.id, generation: 1, holder, previous: None, at: now() };
    Ok(SignedSeat::sign(record, signer)?)
}

pub(crate) fn now() -> String {
    xenonet_text::time::iso(xenonet_text::time::now_ms())
}

fn private_dir(dir: &Path) -> Result<(), MoveError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(MoveError::disk("create the holding's folder"))
}

fn put(dir: &Path, name: &'static str, bytes: &[u8]) -> Result<(), MoveError> {
    xenonet_os::private_file::write(&dir.join(name), bytes, &[]).map_err(MoveError::disk("write a file of the holding"))
}

fn put_json<T: Serialize>(dir: &Path, name: &'static str, value: &T) -> Result<(), MoveError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| MoveError::BadFile { what: name, why: e.to_string() })?;
    put(dir, name, &bytes)
}

fn get(dir: &Path, name: &'static str) -> Result<Vec<u8>, MoveError> {
    std::fs::read(dir.join(name)).map_err(|e| MoveError::BadFile { what: name, why: e.to_string() })
}

fn get_json<T: DeserializeOwned>(dir: &Path, name: &'static str) -> Result<T, MoveError> {
    serde_json::from_slice(&get(dir, name)?).map_err(|e| MoveError::BadFile { what: name, why: e.to_string() })
}

impl Holding {
    /// Where `eidolon` lives under `root`.
    pub fn dir_of(root: &Path, eidolon: &EidolonId) -> PathBuf {
        root.join(eidolon.to_string())
    }

    /// Whether `root` has a finished holding of `eidolon`.
    pub fn exists(root: &Path, eidolon: &EidolonId) -> bool {
        Self::dir_of(root, eidolon).join("seat.json").is_file()
    }

    /// A new eidolon on this machine, sealed. `seat` must be a valid first
    /// seat (see [`seat_first`]); the head and chunks are what was sealed
    /// into the vault.
    pub fn create(root: &Path, spec: EidolonSpec, seat: SignedSeat, head: Vec<u8>, chunks: Vec<ChunkName>) -> Result<Holding, MoveError> {
        if Self::exists(root, &spec.id) {
            return Err(MoveError::Protocol("this machine already holds that eidolon".into()));
        }
        accept_seat(&spec, None, &seat)?;
        Self::install(root, spec, seat, head, chunks)
    }

    /// Writes a holding, sealed, replacing any there. The seat goes last.
    pub(crate) fn install(root: &Path, spec: EidolonSpec, seat: SignedSeat, head: Vec<u8>, chunks: Vec<ChunkName>) -> Result<Holding, MoveError> {
        let dir = Self::dir_of(root, &spec.id);
        private_dir(&dir)?;
        let state = State::Sealed;
        put_json(&dir, "spec.json", &spec)?;
        put(&dir, "head", &head)?;
        put_json(&dir, "chunks.json", &chunks)?;
        put_json(&dir, "state.json", &state)?;
        if let Err(e) = std::fs::remove_file(dir.join("pending.json")) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(MoveError::Disk { what: "remove an old pending move", source: e });
            }
        }
        put_json(&dir, "seat.json", &seat)?;
        Ok(Holding { dir, spec, seat, head, chunks, state, pending: None })
    }

    pub fn load(root: &Path, eidolon: &EidolonId) -> Result<Holding, MoveError> {
        let dir = Self::dir_of(root, eidolon);
        let pending = match std::fs::read(dir.join("pending.json")) {
            Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(|e| MoveError::BadFile { what: "pending.json", why: e.to_string() })?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(MoveError::BadFile { what: "pending.json", why: e.to_string() }),
        };
        Ok(Holding {
            spec: get_json(&dir, "spec.json")?,
            seat: get_json(&dir, "seat.json")?,
            head: get(&dir, "head")?,
            chunks: get_json(&dir, "chunks.json")?,
            state: get_json(&dir, "state.json")?,
            pending,
            dir,
        })
    }

    /// Whether `machine` may open this eidolon now. A move away that was
    /// started and not finished counts as open elsewhere: this copy stays
    /// closed until the move is settled.
    pub fn may_open(&self, machine: &HolderKey) -> Result<(), Refusal> {
        may_open(&self.state, &self.seat, machine, self.pending.is_some())
    }

    /// A new head and the chunks it needs, after the eidolon was changed
    /// and resealed here.
    pub fn record_head(&mut self, head: Vec<u8>, chunks: Vec<ChunkName>) -> Result<(), MoveError> {
        put(&self.dir, "head", &head)?;
        put_json(&self.dir, "chunks.json", &chunks)?;
        self.head = head;
        self.chunks = chunks;
        Ok(())
    }

    pub fn set_state(&mut self, state: State) -> Result<(), MoveError> {
        put_json(&self.dir, "state.json", &state)?;
        self.state = state;
        Ok(())
    }

    /// Written before anything that hands over the seat is sent.
    pub(crate) fn begin_move(&mut self, pending: Pending) -> Result<(), MoveError> {
        put_json(&self.dir, "pending.json", &pending)?;
        self.pending = Some(pending);
        Ok(())
    }

    fn clear_pending(&mut self) -> Result<(), MoveError> {
        match std::fs::remove_file(self.dir.join("pending.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(MoveError::Disk { what: "remove the pending move", source: e }),
        }
        self.pending = None;
        Ok(())
    }

    /// The person says the other machine never got the eidolon: this copy
    /// opens again. Only say so when you know it did not arrive; if it did,
    /// two machines could open it.
    pub fn abandon_pending(&mut self) -> Result<(), MoveError> {
        if self.pending.is_none() {
            return Err(MoveError::NoPending);
        }
        self.clear_pending()
    }

    /// The other machine shows the seat it took: the move is done. Checked
    /// like any seat; then this copy retires.
    pub fn confirm_moved(&mut self, keyring: &KeyRing<'_>, taken: &SignedSeat) -> Result<(), MoveError> {
        let pending = self.pending.ok_or(MoveError::NoPending)?;
        if taken.record.generation != pending.generation || taken.record.holder != pending.to {
            return Err(MoveError::Refused(Refusal::new(
                RefusalCode::StaleGeneration,
                "That seat is not the one this machine handed over.",
                "Show the seat the other machine holds after the move.",
            )));
        }
        accept_seat(&self.spec, Some(&self.seat), taken)?;
        self.retire(keyring, taken)
    }

    /// This machine no longer holds the eidolon: the seat is the receiver's,
    /// the state is retired, the key is gone. The seat and state are written
    /// first, so a stop partway still leaves this copy closed.
    pub(crate) fn retire(&mut self, keyring: &KeyRing<'_>, taken: &SignedSeat) -> Result<(), MoveError> {
        put_json(&self.dir, "seat.json", taken)?;
        self.seat = taken.clone();
        self.set_state(State::Retired)?;
        keyring.delete(&self.spec.id.to_string())?;
        self.clear_pending()
    }
}
