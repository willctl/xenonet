//! The service's parts as the HTTP API reaches them (xenonet-http's
//! parts.rs): the actor gateway, the component service, the assistant's
//! endpoint, A2A and the network, each over the part it is here.

use crate::a2a::outbound::{Caller, Outbound, Register};
use crate::a2a::A2a;
use crate::assistant_endpoint::Endpoint;
use crate::component::Components;
use crate::cores::door::Attestation as DoorAttestation;
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

#[async_trait]
impl parts::A2aOut for Outbound {
    fn status(&self) -> Value {
        Outbound::status(self)
    }
    async fn preview(&self, card: &str, private_host: Option<&str>) -> Result<Value, String> {
        Outbound::preview(self, card, private_host).await
    }
    async fn register(&self, body: &Value) -> Result<Value, String> {
        let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
        let r = Register {
            slug: text("slug").ok_or("name it: { slug }")?,
            card: text("card").ok_or("send its card: { card }")?,
            hash: text("hash").ok_or("send the pin the window showed: { hash }")?,
            approve_host: text("approveHost"),
            private_host: text("privateHost"),
            agents: body.get("agents").cloned().unwrap_or(Value::Null),
            projects: body.get("projects").cloned().unwrap_or(Value::Null),
            days: body.get("days").and_then(Value::as_f64).unwrap_or(30.0),
            token: text("token"),
        };
        Outbound::register(self, r).await
    }
    async fn remove(&self, actor: &str) -> Result<bool, String> {
        Outbound::remove(self, actor).await
    }
    async fn delegate(&self, agent: &str, attestation: DoorAttestation, project: Option<String>, actor: &str, text: Option<&str>, task: Option<&str>) -> Result<Value, String> {
        let attestation = match attestation {
            DoorAttestation::Unchecked => xenonet_authz::Attestation::Unchecked,
            DoorAttestation::ProcessChecked => xenonet_authz::Attestation::ProcessChecked,
        };
        // The routes hold it as a trait object; the stream it starts holds
        // its own reference.
        let me = self.me().ok_or("A2A out is shutting down")?;
        me.delegate(&Caller { agent: agent.to_string(), attestation, project }, actor, text, task).await
    }
}

/// A network refusal as the routes answer it: its status and code.
fn refusal(e: NetError) -> CoreError {
    CoreError { status: Some(e.status), code: Some(e.code.to_string()), ..CoreError::new(e.message) }
}

fn answered(r: R<Value>) -> NetResult {
    r.map_err(refusal)
}

impl NetService {
    // The node of the network a call names, or the one this machine is in.
    fn on(&self, network: Option<&str>) -> Result<Arc<xenonet_net::node::Node>, CoreError> {
        self.nodes().pick(network).map_err(refusal)
    }
}

#[async_trait]
impl parts::Network for NetService {
    fn state(&self) -> Value {
        NetService::state(self)
    }
    fn network_name(&self) -> Option<String> {
        self.node().shared().network().map(|n| n.name)
    }
    fn host(&self, network: Option<&str>) -> Option<String> {
        self.on(network).ok()?.shared().network().map(|n| n.host)
    }
    fn is_gateway(&self, network: Option<&str>) -> bool {
        self.on(network).is_ok_and(|n| n.shared().is_gateway())
    }
    async fn listen(&self, addr: SocketAddr) -> Result<(), CoreError> {
        NetService::listen(self, addr).await.map_err(refusal)
    }
    fn unlisten(&self) {
        NetService::unlisten(self);
    }
    fn lans(&self) -> Vec<SocketAddr> {
        NetService::lans(self)
    }
    async fn create(&self, name: &str, host: &str, address: Option<&str>, lan: Option<&str>, relay: Option<&str>, at_window: bool) -> NetResult {
        let r = self.nodes().create(name, host, address, lan, relay, at_window).await;
        self.networks_changed().await;
        answered(r)
    }
    async fn join(&self, address: &str, ticket: &str) -> NetResult {
        let r = self.nodes().join(address, ticket).await;
        self.networks_changed().await;
        answered(r)
    }
    async fn join_invite(&self, invite: &str, code: &str, at: Option<&str>, lan: bool) -> NetResult {
        let r = self.nodes().join_invite(invite, code, at, lan).await;
        self.networks_changed().await;
        answered(r)
    }
    async fn leave(&self, network: Option<&str>, force: bool) -> NetResult {
        let r = self.nodes().leave(network, force).await;
        self.networks_changed().await;
        answered(r)
    }
    fn tunnel_address(&self, address: Option<&str>) {
        for n in self.nodes().nodes() {
            n.tunnel_address(address);
        }
    }
    async fn ticket(&self, network: Option<&str>, host: &str, window: bool, at_window: bool) -> NetResult {
        answered(self.on(network)?.ticket(host, window, at_window).await)
    }
    async fn invite(&self, network: Option<&str>, host: &str, window: bool, lan: bool, at_window: bool) -> NetResult {
        answered(self.on(network)?.invite(host, window, lan, at_window).await)
    }
    async fn remove(&self, network: Option<&str>, host: &str) -> NetResult {
        answered(self.on(network)?.remove(host).await)
    }
    async fn replace(&self, network: Option<&str>, host: &str, at_window: bool) -> NetResult {
        answered(self.on(network)?.replace(host, at_window).await)
    }
    async fn approve(&self, network: Option<&str>, id: &str, at_window: bool) -> NetResult {
        answered(self.on(network)?.approve(id, at_window).await)
    }
    async fn take_gateway(&self, network: Option<&str>, address: Option<&str>, lan: Option<&str>, at_window: bool) -> NetResult {
        let r = self.on(network)?.take_gateway(address, lan, at_window).await;
        self.networks_changed().await;
        answered(r)
    }
    async fn set_gateway(&self, network: Option<&str>, address: &str) -> NetResult {
        answered(self.on(network)?.set_gateway(address).await)
    }
    async fn set_reach(&self, network: Option<&str>, host: &str, start: Option<&str>, wake: Option<&str>, at_window: bool) -> NetResult {
        answered(self.on(network)?.set_reach(host, start, wake, at_window).await)
    }
    async fn settings(&self, network: Option<&str>, change: NetChange, at_window: bool) -> NetResult {
        let NetChange { titles_only, share_load, lan, start, wake } = change;
        answered(self.on(network)?.settings(Change { titles_only, share_load, lan, start, wake }, at_window).await)
    }
    fn share(&self, network: Option<&str>, sharing: &Value, at_window: bool) -> NetResult {
        answered(self.nodes().share(network, sharing, at_window))
    }
    async fn relay(&self, network: Option<&str>, url: &str, add: bool, mine: bool, at_window: bool) -> NetResult {
        answered(self.nodes().relay(network, url, add, mine, at_window).await)
    }
    fn svc_list(&self, network: Option<&str>) -> NetResult {
        Ok(self.on(network)?.shared().svc_list())
    }
    fn svc_publish(&self, network: Option<&str>, name: &str, target: &str, paths: &[String], hosts: &[String], at_window: bool) -> NetResult {
        answered(self.on(network)?.shared().svc_publish(name, target, paths, hosts, at_window))
    }
    // The person's chain is the machine's, whichever network carries it.
    fn revoke_device(&self, device: &str, at_window: bool) -> NetResult {
        answered(self.node().shared().revoke_device(device, at_window))
    }
    fn svc_unpublish(&self, network: Option<&str>, name: &str) -> NetResult {
        answered(self.on(network)?.shared().svc_unpublish(name))
    }
    fn unfreeze(&self, network: Option<&str>, author: &str, at_window: bool) -> NetResult {
        answered(self.on(network)?.shared().unfreeze(author, at_window))
    }
    fn svc_port(&self, target: &str) -> Option<u16> {
        svc::Target::parse(target).ok().map(|t| t.port)
    }
    // In the network `host` is a member of: a host id that names members of
    // two networks is refused rather than taken for either.
    async fn svc_call(&self, host: &str, name: &str, call: SvcCall) -> SvcAnswer {
        let SvcCall { method, path, headers, body } = call;
        let holding: Vec<_> = self.nodes().nodes().into_iter().filter(|n| n.shared().roster().view_ref().is_some_and(|v| v.member(host).is_some())).collect();
        let node = match holding.as_slice() {
            [one] => one.clone(),
            [] => return SvcAnswer::refused(404, "no-host", &format!("{host} isn't in a network this machine is in")),
            _ => return SvcAnswer::refused(409, "which-network", &format!("{host} names a machine in more than one of this machine's networks")),
        };
        let a = node.shared().svc_call(host, name, svc::Call { method, path, headers, body }).await;
        SvcAnswer { status: a.status, headers: a.headers, body: a.body }
    }
}
