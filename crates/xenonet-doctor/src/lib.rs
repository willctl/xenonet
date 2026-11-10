//! XenoNet's doctor. Each check reads the machine through its host
//! (`checks::DoctorHost`: files, programs, XenoNet's own state), and its
//! verdict and words are here; `diagnostics` is what XenoNet can tell
//! someone helping you, safe to share.

pub mod checks;
pub mod diagnostics;
#[cfg(test)]
mod fake;
pub mod firewall;
pub mod login;
pub mod run;
