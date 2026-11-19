//! Adytum's isolation rules as pure functions. Each rule takes a spec (or a
//! name) and what already exists, and answers `Ok(())` or a `Refusal` that
//! says what to change. Nothing here touches the filesystem or the network.

mod facts;
mod rules;

pub use facts::{CellFacts, World};
pub use rules::{
    automation_port_kept_inside, check_create, name_free, name_is_ours, no_clone_of_open_eidolon, ports_on_localhost, sealed_has_no_mounts, walls_share_no_clone_source,
    walls_share_no_mount, walls_share_no_network, walls_share_no_secret,
};

#[cfg(test)]
mod tests;
