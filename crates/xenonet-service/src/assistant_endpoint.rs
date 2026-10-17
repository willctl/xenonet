//! The private endpoint remote actors' adapters reach XenoNet through
//! (server/assistant-endpoint.js).

use serde_json::{json, Value};

pub struct Endpoint {}

impl Endpoint {
    /// How many adapters are connected, and where it listens.
    pub fn info(&self) -> Value {
        json!({ "listening": false, "port": null, "sessions": 0 })
    }
}
