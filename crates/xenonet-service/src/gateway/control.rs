//! The actor gateway's switch, its tunnel and how it stands
//! (server/gateway-control.js). Kept in the engine's own database, so it
//! comes back as it was when XenoNet starts again.

use crate::cores::{CoreError, R};
use serde_json::Value;

pub struct GatewayControl {}

impl GatewayControl {
    /// The gateway's public address, or null.
    pub fn url(&self) -> Value {
        Value::Null
    }

    pub async fn status(&self) -> R<Value> {
        Err(CoreError::pending("server/gateway-control.js"))
    }

    /// `set({ on, tunnel, check, repair })`: `tunnel` None leaves it,
    /// Some(null) takes it away.
    pub async fn set(&self, _on: Option<bool>, _tunnel: Option<Value>, _check: bool, _repair: bool) -> R<Value> {
        Err(CoreError::pending("server/gateway-control.js"))
    }
}
