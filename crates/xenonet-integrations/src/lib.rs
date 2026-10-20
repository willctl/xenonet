//! core/integrations/ in Rust: routing suggestions, OpenShell sandboxes and
//! their text reviews, and Pi's sessions and exports, all read-only toward
//! the agents they describe. What goes out is checked and bounded here;
//! the processes, git and the JevRouter library stay with the twins.

pub mod b64;
pub mod common;
pub mod index;
pub mod openshell;
pub mod pi;
pub mod reviews;
pub mod routing;
