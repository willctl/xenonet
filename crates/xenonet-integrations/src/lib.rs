//! XenoNet's integrations: routing suggestions, OpenShell sandboxes and
//! their text reviews, Pi's sessions and exports, and the MCP servers'
//! catalogs (`mcp_info`, `mcp_discovery`), all read-only toward the agents
//! they describe. What goes out is checked and bounded here; the commands,
//! git and the records are their hosts' (`openshell::ShellHost`,
//! `reviews::ReviewHost`), which `runtime` answers on this machine, with
//! the service's Integrations, Decider, OpenRouter and McpInfo cores;
//! `jev` is JevRouter's route() over the decision model's answer.

pub mod b64;
pub mod common;
pub mod index;
pub mod jev;
pub mod mcp_discovery;
pub mod mcp_info;
pub mod openshell;
pub mod pi;
pub mod reviews;
pub mod routing;
pub mod runtime;
