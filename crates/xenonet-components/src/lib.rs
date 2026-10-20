//! XenoNet's components (core/components/): the service and the Tools,
//! released and updated on their own. Their packages are checked against
//! their release's hash and size before anything is unpacked, unpacked
//! only when every entry is a plain file or folder inside the package, and
//! switched to only once the new runtime passes its startup check; a
//! service update that fails is rolled back, or recovered on the next
//! launch. And the service's local connection, starting it at login under
//! the system's supervisor, and the console the tools reach an installed
//! XenoNet through.

pub mod console;
pub mod console_policy;
pub mod install;
pub mod packages;
pub mod protocol;
pub mod rpc;
pub mod runtime;
pub mod supervisor;
pub mod tar;
pub mod updates;
