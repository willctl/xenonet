//! The ACP client against a fake ACP v1 agent, built from the SDK's Agent
//! role: this test binary, run again as `fake-agent`, over its standard
//! input and output as a real agent is. Its own main, since libtest's would
//! print on the agent's standard output.
//!
//! Each turn's last line says what the fake agent does: `echo` says the
//! turn's text back, `caps` what XenoNet offered it, `env` the names of
//! its environment, `permission` asks to run a command and says the
//! option it got, `fs` asks for a file and a terminal, `wait` waits to be
//! cancelled. A session opened in a folder named `needs-sign-in` asks to
//! be signed in.
#![expect(clippy::unwrap_used, clippy::panic, reason = "test code with a main of its own, which clippy.toml's test allowances don't reach: a failing test says where")]

use futures::future::BoxFuture;
use futures::FutureExt as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Notify};
use xenonet_acp::schema::{ContentBlock, SessionId, SessionUpdate, StopReason};
use xenonet_acp::{AcpError, Answer, Client, Host, Launch, Mcp, Prompt, Session, Turn};
use xenonet_authz::{
    decide, Act, AllowScope, AnswerDecision, AskId, Asked, Attestation, Authority, Authorized, Consent, Decision, Device, DeviceId, DeviceKind, Door, Ground, Information,
    MachineId, NetworkId, PersonId, PersonSigned, Principal, PromptKind, Request, Resource, Role, RootCertificate, Settings, StandingGrant, VerifiedPresence,
};

/// A test, as main runs them.
type Test = fn() -> BoxFuture<'static, ()>;
/// A prompt the host was asked, and where its answer goes.
type Asking = (Prompt, oneshot::Sender<Answer>);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("fake-agent") {
        fake::run();
        return;
    }
    // A process the fake agent starts, which runs until it's stopped.
    if args.get(1).map(String::as_str) == Some("fake-child") {
        std::thread::sleep(Duration::from_secs(20));
        return;
    }
    let filter: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    let tests: &[(&str, Test)] = &[
        ("start_and_prompt_stream_framed_turns", || start_and_prompt_stream_framed_turns().boxed()),
        ("no_file_system_or_terminal_is_offered", || no_file_system_or_terminal_is_offered().boxed()),
        ("the_agent_gets_no_credential_or_token", || the_agent_gets_no_credential_or_token().boxed()),
        ("a_permission_holds_until_an_allow_with_presence", || a_permission_holds_until_an_allow_with_presence().boxed()),
        ("a_deny_rejects_once", || a_deny_rejects_once().boxed()),
        ("an_allow_for_another_act_rejects", || an_allow_for_another_act_rejects().boxed()),
        ("cancel_ends_the_turn_and_its_held_prompt", || cancel_ends_the_turn_and_its_held_prompt().boxed()),
        ("sign_in_is_the_persons", || sign_in_is_the_persons().boxed()),
        ("a_missing_agent_doesnt_start", || a_missing_agent_doesnt_start().boxed()),
        ("close_stops_the_agent_and_what_it_started", || close_stops_the_agent_and_what_it_started().boxed()),
    ];
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, test) in tests {
        if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        ran += 1;
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(async { tokio::time::timeout(Duration::from_secs(30), test()).await.expect("the test finished in 30 s") });
        }))
        .is_ok();
        eprintln!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            failed.push(*name);
        }
    }
    eprintln!("\n{} passed, {} failed", ran - failed.len(), failed.len());
    if !failed.is_empty() {
        std::process::exit(1);
    }
}

// ----- the host the tests stand in for -----

#[derive(Default)]
struct TestHost {
    asks: Mutex<Option<mpsc::UnboundedSender<Asking>>>,
    texts: Mutex<Vec<String>>,
    refused: Mutex<Vec<&'static str>>,
    changed: Notify,
}

impl Host for TestHost {
    fn ask(&self, prompt: Prompt) -> BoxFuture<'static, Answer> {
        let (tx, rx) = oneshot::channel();
        if let Some(asks) = &*self.asks.lock().unwrap() {
            let _ = asks.send((prompt, tx));
        }
        // No answer at all (the host went) is a deny.
        async move { rx.await.unwrap_or(Answer::Deny) }.boxed()
    }

    fn update(&self, _session: &SessionId, update: SessionUpdate) {
        if let SessionUpdate::AgentMessageChunk(c) = update {
            if let ContentBlock::Text(t) = c.content {
                self.texts.lock().unwrap().push(t.text);
                self.changed.notify_waiters();
            }
        }
    }

    fn refused(&self, method: &'static str) {
        self.refused.lock().unwrap().push(method);
    }
}

impl TestHost {
    fn asking() -> (Arc<TestHost>, mpsc::UnboundedReceiver<Asking>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(TestHost::default());
        *host.asks.lock().unwrap() = Some(tx);
        (host, rx)
    }

    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap().clone()
    }

    /// Waits for the agent to say something that starts with `prefix`.
    async fn said(&self, prefix: &str) -> String {
        loop {
            let changed = self.changed.notified();
            if let Some(t) = self.texts().into_iter().find(|t| t.starts_with(prefix)) {
                return t;
            }
            changed.await;
        }
    }
}

fn base_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = ["PATH", "HOME", "TMPDIR", "SystemRoot", "TEMP"].iter().filter_map(|k| Some((k.to_string(), std::env::var(k).ok()?))).collect();
    env.extend(
        [("XENONET_ACTOR_TOKEN", "xenonet_actor_00"), ("XENONET_PORT", "4319"), ("GITHUB_TOKEN", "ghp_x"), ("ACP_TEST_KEPT", "1")].map(|(k, v)| (k.to_string(), v.to_string())),
    );
    env
}

fn fake_launch() -> Launch {
    let me = std::env::current_exe().expect("the test binary");
    Launch::new(me, vec!["fake-agent".into()], std::env::temp_dir(), base_env(), &[])
}

async fn started(host: Arc<TestHost>) -> (Client, Session) {
    let client = Client::start(&fake_launch(), host).await.expect("the fake agent starts");
    assert_eq!(client.info().agent_info.as_ref().map(|i| i.name.as_str()), Some("fake-agent"));
    let mcp = Mcp { name: "xenonet".into(), command: "/opt/xenonet".into(), args: vec!["mcp".into()] };
    let session = client.open(&std::env::temp_dir(), Some(&mcp)).await.expect("a session");
    (client, session)
}

fn turn(what: &str) -> Turn {
    Turn::message("7", "Docs", None, &Information::new(format!("please\n{what}")), false)
}

// ----- the tests -----

async fn start_and_prompt_stream_framed_turns() {
    let host = Arc::new(TestHost::default());
    let (client, session) = started(host.clone()).await;
    assert_eq!(session.prompt(turn("echo")).await.unwrap(), StopReason::EndTurn);
    let said = host.said("echo:").await;
    assert!(said.starts_with("echo:[XenoNet from Docs] (message #7;"), "{said}");
    assert!(said.contains("not your user's instruction"), "{said}");
    // A started session's first prompt says who asked.
    let by = json!({ "kind": "actor", "id": "actor:muse", "title": "Muse" });
    let first = Turn::started(&by, None, &Information::new("echo".to_string()), None);
    session.prompt(first).await.unwrap();
    let said = host.said("echo:[XenoNet] This was asked").await;
    assert!(said.contains("It came from an agent, not from your user."), "{said}");
    client.close().await;
}

async fn no_file_system_or_terminal_is_offered() {
    let host = Arc::new(TestHost::default());
    let (client, session) = started(host.clone()).await;
    session.prompt(turn("caps")).await.unwrap();
    let caps: Value = serde_json::from_str(host.said("{").await.as_str()).unwrap();
    assert_eq!(caps, json!({ "fs_read": false, "fs_write": false, "terminal": false, "mcp": ["xenonet"] }));
    // Asked anyway: each refused, and the host told.
    session.prompt(turn("fs")).await.unwrap();
    let said = host.said("fs:").await;
    assert_eq!(said, "fs:read refused -32601, write refused -32601, terminal refused -32601");
    assert_eq!(*host.refused.lock().unwrap(), ["fs/read_text_file", "fs/write_text_file", "terminal/create"]);
    client.close().await;
}

async fn the_agent_gets_no_credential_or_token() {
    let host = Arc::new(TestHost::default());
    let (client, session) = started(host.clone()).await;
    session.prompt(turn("env")).await.unwrap();
    let names: Vec<String> = serde_json::from_str(host.said("[").await.as_str()).unwrap();
    assert!(names.contains(&"ACP_TEST_KEPT".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("XENONET_") || n.contains("TOKEN")), "{names:?}");
    client.close().await;
}

/// Starts a `permission` turn and takes the prompt it asks: the turn
/// hasn't ended, and the agent has had no answer.
async fn held(
    host: &Arc<TestHost>,
    rx: &mut mpsc::UnboundedReceiver<Asking>,
    session: Arc<Session>,
) -> (Prompt, oneshot::Sender<Answer>, tokio::task::JoinHandle<Result<StopReason, AcpError>>) {
    let turn_done = tokio::spawn(async move { session.prompt(turn("permission")).await });
    let (prompt, answer) = rx.recv().await.expect("a prompt");
    assert_eq!(prompt.kind, PromptKind::Command);
    assert_eq!(prompt.title.read(), "run ls");
    assert_eq!(prompt.input.read(), &json!({ "command": "ls" }));
    assert!(prompt.ask.as_str().starts_with("ask_"));
    // Held: nothing answers it while the host doesn't.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!turn_done.is_finished(), "the turn waits on the prompt");
    assert!(!host.texts().iter().any(|t| t.starts_with("outcome:")), "no answer went to the agent");
    (prompt, answer, turn_done)
}

async fn a_permission_holds_until_an_allow_with_presence() {
    let (host, mut rx) = TestHost::asking();
    let (client, session) = started(host.clone()).await;
    let (prompt, answer, turn_done) = held(&host, &mut rx, Arc::new(session)).await;
    let ok = person::allow(&prompt.ask, AnswerDecision::Allow);
    assert!(matches!(ok.authority(), Authority::Presence { .. }));
    answer.send(Answer::Allow(ok)).unwrap();
    assert_eq!(turn_done.await.unwrap().unwrap(), StopReason::EndTurn);
    // Allow-once, though allow-always came first in its options.
    assert_eq!(host.said("outcome:").await, "outcome:allow-once");
    client.close().await;
}

async fn a_deny_rejects_once() {
    let (host, mut rx) = TestHost::asking();
    let (client, session) = started(host.clone()).await;
    let (_, answer, turn_done) = held(&host, &mut rx, Arc::new(session)).await;
    answer.send(Answer::Deny).unwrap();
    assert_eq!(turn_done.await.unwrap().unwrap(), StopReason::EndTurn);
    assert_eq!(host.said("outcome:").await, "outcome:reject-once");
    client.close().await;
}

async fn an_allow_for_another_act_rejects() {
    let (host, mut rx) = TestHost::asking();
    let (client, session) = started(host.clone()).await;
    let session = Arc::new(session);
    // Allowed, but for another ask.
    let (_, answer, turn_done) = held(&host, &mut rx, session.clone()).await;
    answer.send(Answer::Allow(person::allow(&AskId::new("ask_other").unwrap(), AnswerDecision::Allow))).unwrap();
    turn_done.await.unwrap().unwrap();
    assert_eq!(host.said("outcome:").await, "outcome:reject-once");
    // A Deny decided for this ask, offered as an Allow.
    host.texts.lock().unwrap().clear();
    let (prompt, answer, turn_done) = held(&host, &mut rx, session).await;
    answer.send(Answer::Allow(person::allow(&prompt.ask, AnswerDecision::Deny))).unwrap();
    turn_done.await.unwrap().unwrap();
    assert_eq!(host.said("outcome:").await, "outcome:reject-once");
    client.close().await;
}

async fn cancel_ends_the_turn_and_its_held_prompt() {
    let (host, mut rx) = TestHost::asking();
    let (client, session) = started(host.clone()).await;
    let session = Arc::new(session);
    // A turn at work.
    let s = session.clone();
    let turn_done = tokio::spawn(async move { s.prompt(turn("wait")).await });
    host.said("waiting").await;
    session.cancel().unwrap();
    assert_eq!(turn_done.await.unwrap().unwrap(), StopReason::Cancelled);
    // A turn waiting on a prompt: the prompt is answered cancelled, not
    // allowed, and the host's answer is no longer waited for.
    let (_, answer, turn_done) = held(&host, &mut rx, session.clone()).await;
    session.cancel().unwrap();
    assert_eq!(turn_done.await.unwrap().unwrap(), StopReason::Cancelled);
    assert_eq!(host.said("outcome:").await, "outcome:cancelled");
    assert!(answer.is_closed(), "the prompt's wait was dropped");
    client.close().await;
}

async fn sign_in_is_the_persons() {
    let host = Arc::new(TestHost::default());
    let client = Client::start(&fake_launch(), host).await.unwrap();
    let dir = std::env::temp_dir().join("needs-sign-in");
    match client.open(&dir, None).await {
        Err(AcpError::SignIn(_)) => {}
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened without signing in"),
    }
    client.close().await;
}

async fn a_missing_agent_doesnt_start() {
    let launch = Launch::new(PathBuf::from("/nonexistent/acp-agent"), vec![], std::env::temp_dir(), base_env(), &[]);
    assert!(matches!(Client::start(&launch, Arc::new(TestHost::default())).await, Err(AcpError::Start(_))));
}

// ----- an answer, as decide makes one -----

mod person {
    use super::*;
    use std::collections::HashMap;
    use xenonet_id::presence::{prove, FixedCheck, PresenceCheck};
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

    const NOW: i64 = 1_800_000_000_000;

    /// This machine (m1), the person and their biometric device, and the
    /// ask waiting: a command, from session s1's checked agent.
    struct World {
        own: MachineId,
        device: SeedSigner,
        asks: HashMap<AskId, Asked>,
    }

    fn person() -> PersonId {
        PersonId::new("prs_owner").unwrap()
    }

    fn device_id(s: &dyn Signer) -> DeviceId {
        DeviceId::new(&s.public().id()).unwrap()
    }

    impl Ground for World {
        fn own_machine(&self) -> &MachineId {
            &self.own
        }
        fn role(&self, _: &NetworkId, _: &MachineId) -> Option<Role> {
            None
        }
        fn settings(&self, _: Option<&NetworkId>) -> Settings {
            Settings::default()
        }
        fn own_person(&self) -> Option<PersonId> {
            Some(person())
        }
        fn device(&self, p: &PersonId, d: &DeviceId) -> Option<Device> {
            (*p == person() && *d == device_id(&self.device)).then(|| Device { key: self.device.public(), kind: DeviceKind::Biometric })
        }
        fn revoked(&self, _: &[u8]) -> bool {
            false
        }
        fn token_roots(&self, _: u32) -> Vec<RootCertificate> {
            Vec::new()
        }
        fn signing_key(&self, _: &MachineId) -> Option<xenonet_id::sign::VerifyingKey> {
            None
        }
        fn consent(&self, _: &xenonet_authz::SessionId) -> Consent {
            Consent::default()
        }
        fn device_of(&self, _: &MachineId) -> Option<DeviceId> {
            Some(device_id(&self.device))
        }
        fn delegations(&self) -> Vec<StandingGrant> {
            Vec::new()
        }
        fn allow_scope(&self, _: &[u8]) -> Option<AllowScope> {
            None
        }
        fn ask(&self, ask: &AskId) -> Option<Asked> {
            self.asks.get(ask).cloned()
        }
        fn governance_synced_at(&self) -> Option<i64> {
            None
        }
    }

    /// What decide allows when the person answers `ask` at this machine's
    /// window with `decision`, Touch ID and all.
    pub fn allow(ask: &AskId, decision: AnswerDecision) -> Box<Authorized> {
        let session = xenonet_authz::SessionId::new("s1").unwrap();
        let asked = Asked { session: session.clone(), by: Attestation::HarnessSigned, kind: PromptKind::Command, delegated_by: None };
        let world = World { own: MachineId::new("m1").unwrap(), device: SeedSigner::from_seed(&[21; 32], KeyHome::File).unwrap(), asks: HashMap::from([(ask.clone(), asked)]) };
        let resource = Resource::Session(session);
        let act = Act::Answer { ask: ask.clone(), decision };
        let sig = PersonSigned::sign(&world.device, &resource, &act).unwrap();
        let signed = PersonSigned::verify(&world, &person(), &device_id(&world.device), &resource, &act, &sig).unwrap();
        let request = Request::new(Principal::Person(signed), resource, act, Door::Local, NOW, None);
        let presence = request.act().presence_what().map(|what| {
            let block = prove(&world.device, Some(&FixedCheck(true)), what, &request.act_hash(), "test", NOW).unwrap();
            VerifiedPresence::check(&mut PresenceCheck::new(), &block, &world, &request).unwrap()
        });
        match decide(None, &request, &world, presence.as_ref()) {
            Decision::Allow(ok) => ok,
            d => panic!("{:?}", d.reason()),
        }
    }
}

/// Closing the client stops the agent, and on Windows (its job object,
/// which CI's Windows job runs this on) what the agent started too. The
/// agent starts with CREATE_NO_WINDOW there, which this run goes through
/// but can't observe.
async fn close_stops_the_agent_and_what_it_started() {
    let host = Arc::new(TestHost::default());
    let (client, session) = started(host.clone()).await;
    let agent = client.pid().expect("the agent's pid");
    session.prompt(turn("child")).await.unwrap();
    let child: u32 = host.said("child:").await["child:".len()..].parse().unwrap();
    assert!(xenonet_os::process::running(child, false), "the agent's child runs");
    client.close().await;
    let gone = |pid: u32| async move {
        for _ in 0..50 {
            if !xenonet_os::process::running(pid, false) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    };
    assert!(gone(agent).await, "the agent stopped");
    if cfg!(windows) {
        assert!(gone(child).await, "its job object stopped what it started");
    }
}

// ----- the fake agent -----

mod fake {
    use super::*;
    use agent_client_protocol::v1::schema::{
        AgentCapabilities, CancelNotification, ContentChunk, CreateTerminalRequest, InitializeRequest, InitializeResponse, McpServer, NewSessionRequest, NewSessionResponse,
        PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse, ReadTextFileRequest, RequestPermissionOutcome, RequestPermissionRequest, SessionNotification,
        TextContent, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
    };
    use agent_client_protocol::v1::ConnectionTo;
    use agent_client_protocol::{Agent, Lines};
    use std::sync::OnceLock;
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    use tokio::sync::watch;

    #[derive(Default)]
    struct State {
        caps: Value,
        mcp: HashMap<String, Vec<String>>,
        sessions: usize,
    }

    fn state() -> &'static Mutex<State> {
        static S: OnceLock<Mutex<State>> = OnceLock::new();
        S.get_or_init(Mutex::default)
    }

    fn cancels() -> &'static watch::Sender<u64> {
        static C: OnceLock<watch::Sender<u64>> = OnceLock::new();
        C.get_or_init(|| watch::channel(0).0)
    }

    fn say(cx: &ConnectionTo<agent_client_protocol::Client>, session: &SessionId, text: String) -> Result<(), agent_client_protocol::Error> {
        let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text)));
        cx.send_notification(SessionNotification::new(session.clone(), SessionUpdate::AgentMessageChunk(chunk)))
    }

    fn refused<T>(r: Result<T, agent_client_protocol::Error>) -> String {
        match r {
            Ok(_) => "served".into(),
            Err(e) => format!("refused {}", i32::from(e.code)),
        }
    }

    async fn turn(cx: ConnectionTo<agent_client_protocol::Client>, req: PromptRequest) -> Result<StopReason, agent_client_protocol::Error> {
        let text: String = req.prompt.iter().filter_map(|b| if let ContentBlock::Text(t) = b { Some(t.text.clone()) } else { None }).collect();
        let s = req.session_id;
        match text.lines().last().unwrap_or("") {
            "echo" => say(&cx, &s, format!("echo:{text}"))?,
            "caps" => {
                let st = state().lock().unwrap();
                let mut caps = st.caps.clone();
                caps["mcp"] = json!(st.mcp.get(&*s.0).cloned().unwrap_or_default());
                say(&cx, &s, caps.to_string())?;
            }
            "env" => say(&cx, &s, json!(std::env::vars().map(|(k, _)| k).collect::<Vec<_>>()).to_string())?,
            "fs" => {
                let read = refused(cx.send_request(ReadTextFileRequest::new(s.clone(), "/etc/hosts")).block_task().await);
                let write = refused(cx.send_request(WriteTextFileRequest::new(s.clone(), "/tmp/x", "x")).block_task().await);
                let term = refused(cx.send_request(CreateTerminalRequest::new(s.clone(), "ls")).block_task().await);
                say(&cx, &s, format!("fs:read {read}, write {write}, terminal {term}"))?;
            }
            "permission" => {
                let fields = ToolCallUpdateFields::new().kind(ToolKind::Execute).title("run ls".to_string()).raw_input(json!({ "command": "ls" }));
                let options = [
                    ("allow-always", PermissionOptionKind::AllowAlways),
                    ("reject-always", PermissionOptionKind::RejectAlways),
                    ("allow-once", PermissionOptionKind::AllowOnce),
                    ("reject-once", PermissionOptionKind::RejectOnce),
                ]
                .map(|(id, k)| PermissionOption::new(id, id, k))
                .to_vec();
                let req = RequestPermissionRequest::new(s.clone(), ToolCallUpdate::new("t1", fields), options);
                let outcome = cx.send_request(req).block_task().await?.outcome;
                let (said, end) = match outcome {
                    RequestPermissionOutcome::Selected(o) => (o.option_id.0.to_string(), StopReason::EndTurn),
                    _ => ("cancelled".to_string(), StopReason::Cancelled),
                };
                say(&cx, &s, format!("outcome:{said}"))?;
                return Ok(end);
            }
            "child" => {
                let me = std::env::current_exe().unwrap();
                let mut child = std::process::Command::new(me).arg("fake-child").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).spawn().unwrap();
                let pid = child.id();
                // Left running, reaped when it ends: the client's close
                // must stop it.
                std::thread::spawn(move || child.wait());
                say(&cx, &s, format!("child:{pid}"))?;
            }
            "wait" => {
                let mut c = cancels().subscribe();
                say(&cx, &s, "waiting".into())?;
                let _ = c.changed().await;
                return Ok(StopReason::Cancelled);
            }
            other => say(&cx, &s, format!("unknown:{other}"))?,
        }
        Ok(StopReason::EndTurn)
    }

    pub fn run() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let out = Box::pin(futures::sink::unfold(tokio::io::stdout(), async |mut w, line: String| {
                w.write_all(line.as_bytes()).await?;
                w.write_all(b"\n").await?;
                w.flush().await?;
                Ok::<_, std::io::Error>(w)
            }));
            let input = Box::pin(futures::stream::unfold(tokio::io::BufReader::new(tokio::io::stdin()).lines(), async |mut l| match l.next_line().await {
                Ok(Some(line)) => Some((Ok(line), l)),
                Ok(None) => None,
                Err(e) => Some((Err(e), l)),
            }));
            let _ = Agent
                .v1()
                .name("fake-agent")
                .on_receive_request(
                    async move |req: InitializeRequest, responder, _cx: ConnectionTo<agent_client_protocol::Client>| {
                        let c = &req.client_capabilities;
                        state().lock().unwrap().caps = json!({ "fs_read": c.fs.read_text_file, "fs_write": c.fs.write_text_file, "terminal": c.terminal });
                        responder.respond(
                            InitializeResponse::new(req.protocol_version)
                                .agent_capabilities(AgentCapabilities::default())
                                .agent_info(agent_client_protocol::v1::schema::Implementation::new("fake-agent", "1.0")),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |req: NewSessionRequest, responder, _cx: ConnectionTo<agent_client_protocol::Client>| {
                        if req.cwd.ends_with("needs-sign-in") {
                            return responder.respond_with_error(agent_client_protocol::Error::auth_required());
                        }
                        let mut st = state().lock().unwrap();
                        st.sessions += 1;
                        let id = format!("s{}", st.sessions);
                        let names = req.mcp_servers.iter().map(|m| if let McpServer::Stdio(s) = m { s.name.clone() } else { "other".into() }).collect();
                        st.mcp.insert(id.clone(), names);
                        responder.respond(NewSessionResponse::new(id))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |req: PromptRequest, responder, cx: ConnectionTo<agent_client_protocol::Client>| {
                        let c = cx.clone();
                        cx.spawn(async move {
                            match turn(c, req).await {
                                Ok(stop) => responder.respond(PromptResponse::new(stop)),
                                Err(e) => responder.respond_with_error(e),
                            }
                        })
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_n: CancelNotification, _cx: ConnectionTo<agent_client_protocol::Client>| {
                        cancels().send_modify(|n| *n += 1);
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .connect_to(Lines::new(out, input))
                .await;
        });
    }
}
