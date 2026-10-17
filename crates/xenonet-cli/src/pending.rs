//! The parts of the core the command calls that aren't in Rust yet. Each
//! is a function with the JS function's shape (JSON in and out where the
//! JS passes objects), which says what it needs and exits 1. The commands
//! around them are whole, so wiring the real core in is a change here.
//!
//! ```text
//! function                 JS                                         commands
//! create_world             core/demo.js createWorld                   drift (no XenoNet, --demo)
//! ```

use crate::out::Fail;

fn needs(module: &str) -> Fail {
    Fail::new(format!("this needs {module}, which isn't in the Rust xenonet yet"), 1)
}

// ---------- the demo ----------

/// createWorld(): the demo's made-up world, in a temporary folder.
pub fn create_world() -> Fail {
    needs("core/demo.js")
}
