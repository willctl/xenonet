//! The generic ACP client (docs/design/direction.md, "Adapters and
//! surfaces"): XenoNet starts and drives a session of any agent that
//! speaks the Agent Client Protocol, v1, through this one crate. Which
//! apps start this way, and with which command, is the registry's to say;
//! nothing here names one.
//!
//! - `launch`: the agent's command, run with no window, in its own process
//!   tree, with an environment that holds none of XenoNet's credentials or
//!   any token (`agent_env`).
//! - `client`: the connection. `Client::start` starts the agent and
//!   initializes it, `Client::open` opens a session in a folder, and
//!   `Session::prompt` sends a turn and waits for it to end while the
//!   session's updates stream to the `Host`; `Session::cancel` cancels it.
//! - `permission`: what a permission request becomes. Each is a XenoNet
//!   prompt (`Prompt`), held until the host answers it. An Allow is an
//!   `Authorized` that `xenonet_authz::decide` made for that very ask (the
//!   person's presence, an actor's allow scope, or the person's
//!   delegation); it picks the agent's allow-once option and nothing
//!   wider. A Deny picks reject-once.
//!
//! What XenoNet never does as an ACP client (revision 25 of phase 6):
//!
//! - offer the agent the file system or a terminal. Its client
//!   capabilities say no to both, and such a request is refused, so the
//!   agent reads, writes and runs only through its own tools, under its
//!   own sandbox and prompts;
//! - pick an `allow_always` or `reject_always` option, which an agent keeps
//!   in its own settings: one answer covers one call;
//! - send text the agent would read as its user's: a turn is a `Turn`,
//!   which only XenoNet's framing makes (who asked, or who a message is
//!   from, and that it isn't the user's instruction);
//! - answer `authenticate`: the person signs in to the agent themselves;
//! - name an MCP server in `session/new` but its own.
//!
//! The SDK's own process launcher (`AcpAgent`, its `process` feature) adds
//! to the environment XenoNet's service runs with and can't take a
//! variable out, and it brings a second I/O reactor beside tokio's; so
//! XenoNet starts the agent the way it starts its other commands and hands
//! the SDK the agent's standard input and output as lines.

mod client;
mod launch;
mod permission;

pub use client::{AcpError, Client, Host, Session, Turn};
pub use launch::{agent_env, holds_secret, Launch, Mcp};
pub use permission::{allows, choose, prompt_kind, Answer, Prompt};

/// ACP v1's own types, as this crate speaks them: what a session's updates
/// and a turn's end are.
pub use agent_client_protocol::v1::schema;
