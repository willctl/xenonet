//! The service's parts as the HTTP API reaches them (xenonet-http's
//! parts.rs): the actor gateway, the component service, the assistant's
//! endpoint, A2A and the network, each over the part it is here.

use crate::a2a::A2a;
use crate::assistant_endpoint::Endpoint;
use crate::component::Components;
use crate::cores::CoreError;
use crate::gateway::control::GatewayControl;
use crate::gateway::network::NetService;
use async_trait::async_trait;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use xenonet_http::parts::{self, NetChange, NetResult, SvcAnswer, SvcCall};
use xenonet_net::node::Change;
use xenonet_net::{svc, NetError, R};

#[async_trait]
impl parts::Gateway for GatewayControl {
    fn url(&self) -> Value {
        GatewayControl::url(self)
    }
    fn brief(&self) -> Value {
        GatewayControl::brief(self)
    }
    async fn status(&self) -> Result<Value, CoreError> {
        GatewayControl::status(self).await
    }
    async fn set(self: Arc<Self>, on: Option<bool>, pick: Option<Value>, check: bool, repair: bool) -> Result<Value, CoreError> {
        GatewayControl::set(&self, on, pick, check, repair).await
    }
    fn tunnel_view(&self) -> Value {
        self.tunnel().view()
    }
    fn public_host(&self) -> Option<String> {
        self.gateway().public_host()
    }
    fn port(&self) -> u16 {
        self.gateway().port()
    }
}

#[async_trait]
impl parts::Components for Components {
    fn info(&self) -> Value {
        Components::info(self)
    }
    fn notify(&self, note: Value) {
        Components::notify(self, note);
    }
    async fn open(&self, target: &str) -> bool {
        Components::open(self, target).await
    }
    async fn show(&self, select: Option<&str>) -> bool {
        Components::show(self, select).await
    }
    async fn console(&self, body: Value) -> Result<Value, String> {
        Components::console(self, body).await
    }
}

impl parts::Endpoint for Endpoint {
    fn info(&self) -> Value {
        Endpoint::info(self)
    }
}

impl parts::A2a for A2a {
    fn status(&self, address: Option<&str>) -> Value {
        A2a::status(self, address)
    }
    fn set_enabled(&self, on: bool) -> Result<(), String> {
        A2a::set_enabled(self, on)
    }
    fn set_default_target(&self, actor: &str, target: Option<&str>) -> Result<(), String> {
        A2a::set_default_target(self, actor, target)
    }
}

/// A network refusal as the routes answer it: its status and code.
fn refusal(e: NetError) -> CoreError {
    CoreError { status: Some(e.status), code: Some(e.code.to_string()), ..CoreError::new(e.message) }
}

fn answered(r: R<Value>) -> NetResult {
    r.map_err(refusal)
}

#[async_trait]
impl parts::Network for NetService {
    fn state(&self) -> Value {
        NetService::state(self)
    }
    fn network_name(&self) -> Option<String> {
        self.node().shared().network().map(|n| n.name)
    }
    fn host(&self) -> Option<String> {
        self.node().shared().network().map(|n| n.host)
    }
    fn is_gateway(&self) -> bool {
        self.node().shared().is_gateway()
    }
    async fn listen(&self, addr: SocketAddr) -> Result<(), CoreError> {
        NetService::listen(self, addr).await.map_err(refusal)
    }
    fn unlisten(&self) {
        NetService::unlisten(self);
    }
    fn lan(&self) -> Option<SocketAddr> {
        NetService::lan(self)
    }
    fn create(&self, name: &str, host: &str, address: Option<&str>, lan: Option<&str>, at_window: bool) -> NetResult {
        answered(self.node().create(name, host, address, lan, at_window))
    }
    async fn join(&self, address: &str, ticket: &str) -> NetResult {
        answered(self.node().join(address, ticket).await)
    }
    async fn leave(&self, force: bool) -> NetResult {
        answered(self.node().leave(force).await)
    }
    fn tunnel_address(&self, address: Option<&str>) {
        self.node().tunnel_address(address);
    }
    async fn ticket(&self, host: &str, window: bool, at_window: bool) -> NetResult {
        answered(self.node().ticket(host, window, at_window).await)
    }
    async fn remove(&self, host: &str) -> NetResult {
        answered(self.node().remove(host).await)
    }
    async fn replace(&self, host: &str, at_window: bool) -> NetResult {
        answered(self.node().replace(host, at_window).await)
    }
    async fn approve(&self, id: &str, at_window: bool) -> NetResult {
        answered(self.node().approve(id, at_window).await)
    }
    async fn take_gateway(&self, address: Option<&str>, lan: Option<&str>, at_window: bool) -> NetResult {
        answered(self.node().take_gateway(address, lan, at_window).await)
    }
    async fn set_gateway(&self, address: &str) -> NetResult {
        answered(self.node().set_gateway(address).await)
    }
    async fn set_reach(&self, host: &str, start: Option<&str>, wake: Option<&str>, at_window: bool) -> NetResult {
        answered(self.node().set_reach(host, start, wake, at_window).await)
    }
    async fn settings(&self, change: NetChange, at_window: bool) -> NetResult {
        let NetChange { titles_only, share_load, lan, start, wake } = change;
        answered(self.node().settings(Change { titles_only, share_load, lan, start, wake }, at_window).await)
    }
    fn svc_list(&self) -> Value {
        self.node().shared().svc_list()
    }
    fn svc_publish(&self, name: &str, target: &str, paths: &[String], hosts: &[String], at_window: bool) -> NetResult {
        answered(self.node().shared().svc_publish(name, target, paths, hosts, at_window))
    }
    fn revoke_device(&self, device: &str, at_window: bool) -> NetResult {
        answered(self.node().shared().revoke_device(device, at_window))
    }
    fn svc_unpublish(&self, name: &str) -> NetResult {
        answered(self.node().shared().svc_unpublish(name))
    }
    fn unfreeze(&self, author: &str, at_window: bool) -> NetResult {
        answered(self.node().shared().unfreeze(author, at_window))
    }
    fn svc_port(&self, target: &str) -> Option<u16> {
        svc::Target::parse(target).ok().map(|t| t.port)
    }
    async fn svc_call(&self, host: &str, name: &str, call: SvcCall) -> SvcAnswer {
        let SvcCall { method, path, headers, body } = call;
        let a = self.node().shared().svc_call(host, name, svc::Call { method, path, headers, body }).await;
        SvcAnswer { status: a.status, headers: a.headers, body: a.body }
    }
}
