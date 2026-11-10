//! XenoNet's install: the xenonet command on your PATH (`command`), the
//! service's settings (`config`), the instances XenoNet Dev runs as
//! (`instances`) and its version (`version`); and its components, the
//! service and the Tools, released and updated on their own. Their packages are checked against
//! their release's hash and size before anything is unpacked, unpacked
//! only when every entry is a plain file or folder inside the package, and
//! switched to only once the new runtime passes its startup check; a
//! service update that fails is rolled back, or recovered on the next
//! launch. And the service's local connection, starting it at login under
//! the system's supervisor, the macOS tray beside it (tray.rs), and the
//! console the tools reach an installed XenoNet through. What that work
//! waits on is its caller's (`host`).

pub mod command;
pub mod config;
pub mod console;
pub mod console_policy;
pub mod host;
pub mod install;
pub mod instances;
pub mod packages;
pub mod protocol;
pub mod rpc;
pub mod runtime;
pub mod supervisor;
pub mod tar;
pub mod tray;
pub mod updates;
pub mod version;
