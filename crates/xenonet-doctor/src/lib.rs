//! XenoNet's doctor. Each check reads the machine through its host
//! (`checks::DoctorHost`: files, programs, XenoNet's own state), and its
//! verdict and words are here; `diagnostics` is what XenoNet can tell
//! someone helping you, safe to share. `runtime` runs them in the
//! service: the Doctor and Diagnostics cores, over the other cores.

pub mod checks;
pub mod diagnostics;
#[cfg(test)]
mod fake;
pub mod firewall;
pub mod login;
pub mod run;
pub mod runtime;
