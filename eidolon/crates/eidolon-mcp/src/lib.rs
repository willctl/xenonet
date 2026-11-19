//! The browser facet's tools for an agent, as an MCP server.
//!
//! The browser runs in a cell ([`eidolon_browser`]); these tools drive it
//! over WebDriver BiDi and name what they act on with short uids from a
//! snapshot, as the Firefox MCP server does, so skills written for that
//! server carry over. An agent gets pages, a snapshot, click, fill, type,
//! key presses, page text, screenshots, uploads and downloads. It does not
//! get arbitrary script.
//!
//! - [`BrowserTools`]: the tools, each a method; [`definitions`] lists them
//!   with their JSON Schemas.
//! - [`serve_stdio`] / [`serve_transport`]: the MCP server over `rmcp`.
//! - [`serve_http`]: the same tools on localhost over HTTP, with a token
//!   of their own, for XenoNet to publish as a service.
//! - [`Bidi`]: what the tools need of a browser, implemented for a
//!   supervised [`eidolon_browser::Browser`] and a bare session.
//!
//! Files pass through one host folder. An upload must be inside it once
//! symlinks are resolved ([`files::upload_source`]); a fetched download
//! lands in its `downloads` folder.

mod args;
mod bidi;
mod error;
pub mod files;
mod http;
mod keys;
mod server;
pub mod snapshot;
mod tools;

pub use bidi::Bidi;
pub use error::{Error, Result};
pub use http::{serve_http, HttpServer, MAX_BODY, PATH};
pub use server::{serve_stdio, serve_transport};
pub use tools::{check_url, definitions, BrowserTools, Download, Part, ToolDef};
