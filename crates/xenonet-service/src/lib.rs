//! The XenoNet service, `xenonet serve` (docs/design/rust-core.md, "The
//! service"): what `node server/index.js` is. The HTTP API on 127.0.0.1
//! with its checks and event stream (http.rs, routes/, events.rs), the
//! component service's socket (component.rs), the actor gateway
//! (gateway/) and the assistant's endpoint, over the cores, each behind a
//! trait until its crate lands (cores/).

// A core's error carries what the JS errors do (a status, a code, extra
// fields), and is rare: boxing it would only make each route wordier.
#![allow(clippy::result_large_err)]
// Hooks are closures shared between tasks, typed where they're kept.
#![allow(clippy::type_complexity)]

pub mod actor_caller;
pub mod assistant_endpoint;
pub mod borrowed;
pub mod buckets;
pub mod component;
pub mod config;
pub mod cores;
pub mod ctx;
pub mod environments;
pub mod events;
pub mod gateway;
pub mod glue;
pub mod http;
pub mod net;
pub mod routes;
pub mod rpc;
pub mod serve;
pub mod static_files;
pub mod upstream;
pub mod window_key;
pub mod wire;

pub use serve::run;
