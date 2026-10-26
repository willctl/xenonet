//! core/doctor/ in Rust: XenoNet's doctor. Each check reads the machine
//! through what it's given (the twin's `ctx`: files, processes, XenoNet's
//! own state), and its verdict and words are here.

pub mod checks;
pub mod firewall;
pub mod login;
pub mod run;
