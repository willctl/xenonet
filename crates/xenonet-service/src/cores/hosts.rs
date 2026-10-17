//! Hosts (core/hosts/): this XenoNet as a host (config.js), and the hub
//! that shows other machines' XenoNets with XENONET_HOSTS=1 (hub.js).

use super::{pending, Pending, R};
use async_trait::async_trait;
use serde_json::Value;

const CONFIG: &str = "core/hosts/config.js";
const HUB: &str = "core/hosts/hub.js";

pub trait HostsConfig: Send + Sync {
    /// `hosts.self()`: its name, its key, the folder ~ stands for.
    fn self_info(&self) -> R<Value> {
        pending(CONFIG)
    }
    fn set_self(&self, _id: &Value) -> R<Value> {
        pending(CONFIG)
    }
    fn ssh_config(&self) -> R<Value> {
        pending(CONFIG)
    }
}

/// A host's answer through its relay: `{ status, body }`.
pub struct HubAnswer {
    pub status: u16,
    pub body: Value,
}

#[async_trait]
pub trait Hub: Send + Sync {
    fn has(&self, _host: &str) -> bool {
        false
    }
    async fn get(&self, _host: &str, _path: &str) -> R<HubAnswer> {
        pending(HUB)
    }
    fn list(&self) -> R<Value> {
        pending(HUB)
    }
    fn add(&self, _body: &Value, _replace: bool) -> R<Value> {
        pending(HUB)
    }
    fn detail(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    fn remove(&self, _id: &str, _forget: bool) -> R<Value> {
        pending(HUB)
    }
    fn reconnect(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    async fn test(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    /// This machine's map with other hosts' in it.
    fn graph(&self, g: Value) -> Value {
        g
    }
    fn live(&self, s: Value) -> Value {
        s
    }
    fn summary(&self, s: Value) -> Value {
        s
    }
    /// The host-… simulations (dev only).
    fn simulate(&self, _kind: &str, _args: &Value) -> R<Option<Value>> {
        pending(HUB)
    }
    fn start(&self) {}
    fn stop(&self) {}
}

impl HostsConfig for Pending {}
impl Hub for Pending {}
