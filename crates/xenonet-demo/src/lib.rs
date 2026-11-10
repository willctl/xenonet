//! XenoNet's demo: a made-up workspace that runs through XenoNet's real
//! pipeline in place of your own. It writes projects (real git repos),
//! transcripts, rollouts, sessions and a small shared context into a
//! temporary folder, lists the sessions the way the adapters would, and
//! keeps appending lines so the map stays busy. Your real projects and
//! sessions are never read while it runs.
//!
//! - the world itself (`Demo`), and what it asks of the rest of XenoNet
//!   (`Around`): whether git runs, the accounts' ids, the clock;
//! - `root`: a fresh world beside an old one a Windows program still holds.
//!
//! Only the composition root and the map's demo driver use it; the rest of
//! XenoNet reaches the demo through the cores it wires.

pub mod root;
mod world;

pub use world::*;

/// The made-up world's sessions, by title and id, as its actors name them.
pub fn session_titles() -> Vec<(String, String)> {
    let list = Demo::new("", "0", "1970-01-01").session_list();
    list.iter().filter_map(|s| Some((s.get("title")?.as_str()?.to_string(), s.get("id")?.as_str()?.to_string()))).collect()
}
