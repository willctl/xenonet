//! A host's keys (docs/design/network.md, section 3), from `xenonet-id`:
//! an Ed25519 key that signs frames and an X25519 key that frames are
//! sealed to, in one owner-only file, `<state>/network/host.key`.

pub use xenonet_id::keys::{write_private, HostKey, PublicKeys};
