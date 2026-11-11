//! The connection to one agent: started, initialized as a client that
//! offers nothing (no file system, no terminal), its sessions opened in a
//! folder, prompted and cancelled, its updates passed to the host and its
//! permission requests held as XenoNet's prompts.

use crate::launch::{self, Launch, Mcp, Tail};
use crate::permission::{self, Answer, Prompt};
use crate::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, CreateTerminalRequest, Implementation, InitializeRequest, InitializeResponse, KillTerminalRequest, McpServer,
    McpServerStdio, NewSessionRequest, PromptRequest, ReadTextFileRequest, ReleaseTerminalRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionId, SessionNotification, SessionUpdate, StopReason, TerminalOutputRequest, TextContent, WaitForTerminalExitRequest, WriteTextFileRequest,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{Agent, ErrorCode};
use futures::future::BoxFuture;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use xenonet_authz::Information;
use xenonet_os::process::Tree;
use xenonet_os::sync::Locked as _;

/// How long a closed agent has to exit by itself, once its input is
/// closed, before its tree is stopped.
const EXIT_GRACE: Duration = Duration::from_secs(1);

/// What the client asks of the XenoNet it runs in.
pub trait Host: Send + Sync + 'static {
    /// A permission prompt from a session: held (shown where the person or
    /// an actor answers, kept in the ground `decide` reads) until it's
    /// answered, and denied by the host when nobody answers by the hold's
    /// end. Dropped unanswered when the turn is cancelled.
    fn ask(&self, prompt: Prompt) -> BoxFuture<'static, Answer>;

    /// An update from a session XenoNet opened, as the agent sent it.
    fn update(&self, session: &SessionId, update: SessionUpdate);

    /// A request XenoNet never serves (the file system, a terminal), which
    /// it refused, by its method.
    fn refused(&self, _method: &'static str) {}
}

/// Why the client couldn't do something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpError {
    /// The agent didn't start, or stopped before it answered.
    Start(String),
    /// The agent asks to be signed in, which is the person's to do
    /// themselves: XenoNet never answers `authenticate`.
    SignIn(String),
    /// The agent answered with an error, or not in ACP v1.
    Agent { code: i32, message: String },
}

impl fmt::Display for AcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcpError::Start(m) => write!(f, "the agent didn't start: {m}"),
            AcpError::SignIn(m) => write!(f, "the agent isn't signed in ({m}): sign in to it yourself, then start it again"),
            AcpError::Agent { code, message } => write!(f, "the agent answered {code}: {message}"),
        }
    }
}

impl std::error::Error for AcpError {}

fn agent_error(e: agent_client_protocol::Error) -> AcpError {
    if e.code == ErrorCode::AUTH_REQUIRED {
        return AcpError::SignIn(e.message);
    }
    AcpError::Agent { code: i32::from(e.code), message: e.message }
}

/// A turn's text: only XenoNet's framing makes one, so nothing reaches an
/// agent as its user's words unless XenoNet's window wrote them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn(String);

impl Turn {
    /// A started session's first prompt, after who asked for it (`by` is
    /// `{ kind, id, title }`; `from` the machine it came from, None for
    /// this one; `delegation` the machine and day of the delegation it
    /// runs under).
    pub fn started(by: &Value, from: Option<&str>, prompt: &Information<String>, delegation: Option<(&str, &str)>) -> Turn {
        Turn(xenonet_engine::delivery::starts::framed(by, from, prompt.read(), delegation))
    }

    /// A delivered message, as hook delivery frames it: who it's from, how
    /// XenoNet knows (`verified`), and that it's another agent's message,
    /// not the user's instruction. `user` is a message written in
    /// XenoNet's window, which the router alone says.
    pub fn message(id: &str, from: &str, verified: Option<&str>, text: &Information<String>, user: bool) -> Turn {
        Turn(xenonet_engine::delivery::policy::framed(id, from, verified, text.read(), user))
    }

    pub fn text(&self) -> &str {
        &self.0
    }
}

/// What the connection's handlers share with the client: the host, and
/// the sessions XenoNet opened, each with its turn's cancel signal.
struct Shared {
    host: Arc<dyn Host>,
    sessions: Mutex<HashMap<SessionId, watch::Sender<u64>>>,
}

impl Shared {
    fn cancelled(&self, session: &SessionId) -> Option<watch::Receiver<u64>> {
        self.sessions.locked().get(session).map(watch::Sender::subscribe)
    }

    fn knows(&self, session: &SessionId) -> bool {
        self.sessions.locked().contains_key(session)
    }

    /// A permission request, held as a prompt until the host answers or
    /// the turn is cancelled (ACP: a cancelled turn's pending requests are
    /// answered cancelled). A session XenoNet didn't open asks nothing.
    async fn answer(&self, req: &RequestPermissionRequest) -> RequestPermissionOutcome {
        let Some(mut cancelled) = self.cancelled(&req.session_id) else { return RequestPermissionOutcome::Cancelled };
        let prompt = Prompt::of(req);
        let ask = prompt.ask.clone();
        let asked = self.host.ask(prompt);
        tokio::select! {
            answer = asked => permission::choose(&req.options, &ask, &answer),
            _ = cancelled.changed() => RequestPermissionOutcome::Cancelled,
        }
    }
}

/// The agent's process, until the client closes.
struct Process {
    child: Child,
    tree: Tree,
}

impl Process {
    fn stop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            self.tree.kill(&mut self.child);
            let _ = self.child.wait();
        }
    }
}

/// A started, initialized agent.
pub struct Client {
    cx: ConnectionTo<Agent>,
    shared: Arc<Shared>,
    info: InitializeResponse,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), agent_client_protocol::Error>>>,
    process: Option<Process>,
    stderr: Tail,
}

/// Registers a refusal for each request of the file system and the
/// terminal, which XenoNet's capabilities never offered.
macro_rules! refuse {
    ($b:expr, $shared:expr, $($t:ty => $method:literal),+ $(,)?) => {
        $b$(.on_receive_request(
            {
                let s = $shared.clone();
                async move |_req: $t, responder, _cx: ConnectionTo<Agent>| {
                    s.host.refused($method);
                    responder.respond_with_error(agent_client_protocol::Error::method_not_found().data(Value::String(format!("XenoNet offers no {}", $method))))
                }
            },
            agent_client_protocol::on_receive_request!(),
        ))+
    };
}

impl Client {
    /// Starts the agent `launch` names and initializes it in ACP v1, as a
    /// client offering neither the file system nor a terminal.
    pub async fn start(launch: &Launch, host: Arc<dyn Host>) -> Result<Client, AcpError> {
        let running = launch.spawn().map_err(|e| AcpError::Start(e.to_string()))?;
        let launch::Running { child, tree, stdin, stdout, stderr } = running;
        let mut process = Process { child, tree };
        let shared = Arc::new(Shared { host, sessions: Mutex::new(HashMap::new()) });
        let (ready, readied) = oneshot::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let builder = agent_client_protocol::Client
            .v1()
            .name("xenonet")
            .on_receive_request(
                {
                    let s = shared.clone();
                    async move |req: RequestPermissionRequest, responder, cx: ConnectionTo<Agent>| {
                        let s = s.clone();
                        // Off the dispatch loop: the answer can take as
                        // long as the hold, and other messages go on.
                        cx.spawn(async move {
                            let outcome = s.answer(&req).await;
                            responder.respond(RequestPermissionResponse::new(outcome))
                        })
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                {
                    let s = shared.clone();
                    async move |n: SessionNotification, _cx: ConnectionTo<Agent>| {
                        if s.knows(&n.session_id) {
                            s.host.update(&n.session_id, n.update);
                        }
                        Ok(())
                    }
                },
                agent_client_protocol::on_receive_notification!(),
            );
        let builder = refuse!(builder, shared,
            ReadTextFileRequest => "fs/read_text_file",
            WriteTextFileRequest => "fs/write_text_file",
            CreateTerminalRequest => "terminal/create",
            TerminalOutputRequest => "terminal/output",
            ReleaseTerminalRequest => "terminal/release",
            WaitForTerminalExitRequest => "terminal/wait_for_exit",
            KillTerminalRequest => "terminal/kill",
        );
        let transport = launch::lines(stdin, stdout);
        let task = tokio::spawn(builder.connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            let _ = ready.send(cx);
            let _ = stopped.await;
            Ok(())
        }));
        let Ok(cx) = readied.await else {
            process.stop();
            let why = match task.await {
                Ok(Err(e)) => e.message,
                _ => "it closed".into(),
            };
            return Err(AcpError::Start(with_stderr(why, &stderr)));
        };
        let init =
            InitializeRequest::new(ProtocolVersion::V1).client_capabilities(ClientCapabilities::default()).client_info(Implementation::new("xenonet", env!("CARGO_PKG_VERSION")));
        let mut client = Client { cx, shared, info: InitializeResponse::new(ProtocolVersion::V1), stop: Some(stop), task: Some(task), process: Some(process), stderr };
        match client.cx.send_request(init).block_task().await {
            Ok(info) if info.protocol_version == ProtocolVersion::V1 => {
                client.info = info;
                Ok(client)
            }
            Ok(info) => {
                client.close().await;
                Err(AcpError::Agent { code: 0, message: format!("it speaks ACP {}, not 1", info.protocol_version) })
            }
            Err(e) => {
                let e = AcpError::Start(with_stderr(e.message, &client.stderr));
                client.close().await;
                Err(e)
            }
        }
    }

    /// What the agent said of itself when it was initialized.
    pub fn info(&self) -> &InitializeResponse {
        &self.info
    }

    /// Opens a session in `cwd`, told of XenoNet's own MCP server (`mcp`)
    /// and no other.
    pub async fn open(&self, cwd: &Path, mcp: Option<&Mcp>) -> Result<Session, AcpError> {
        let servers = mcp.map(|m| McpServer::Stdio(McpServerStdio::new(m.name.clone(), m.command.clone()).args(m.args.clone()))).into_iter().collect();
        let req = NewSessionRequest::new(cwd).mcp_servers(servers);
        let id = self.cx.send_request(req).block_task().await.map_err(agent_error)?.session_id;
        self.shared.sessions.locked().insert(id.clone(), watch::channel(0).0);
        Ok(Session { id, cx: self.cx.clone(), shared: self.shared.clone() })
    }

    /// Closes the connection: the agent's input closes, it has a second to
    /// exit, and then its tree is stopped.
    pub async fn close(mut self) {
        self.shared.sessions.locked().clear();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(EXIT_GRACE * 2, task).await;
        }
        if let Some(mut p) = self.process.take() {
            let until = tokio::time::Instant::now() + EXIT_GRACE;
            while !matches!(p.child.try_wait(), Ok(Some(_))) && tokio::time::Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            p.stop();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
        if let Some(mut p) = self.process.take() {
            p.stop();
        }
    }
}

fn with_stderr(why: String, stderr: &Tail) -> String {
    let said = stderr.text();
    if said.is_empty() {
        why
    } else {
        format!("{why}; it said: {said}")
    }
}

/// A session XenoNet opened.
pub struct Session {
    id: SessionId,
    cx: ConnectionTo<Agent>,
    shared: Arc<Shared>,
}

impl Session {
    /// Its id, as the agent named it.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Sends a turn and waits for it to end, while its updates go to the
    /// host and its permission requests are held as prompts. Why it ended
    /// (`Cancelled` after `cancel`).
    pub async fn prompt(&self, turn: Turn) -> Result<StopReason, AcpError> {
        let req = PromptRequest::new(self.id.clone(), vec![ContentBlock::Text(TextContent::new(turn.0))]);
        Ok(self.cx.send_request(req).block_task().await.map_err(agent_error)?.stop_reason)
    }

    /// Cancels its turn: a permission request it's waiting on is answered
    /// cancelled, and the agent is told to stop.
    pub fn cancel(&self) -> Result<(), AcpError> {
        if let Some(tx) = self.shared.sessions.locked().get(&self.id) {
            tx.send_modify(|n| *n += 1);
        }
        self.cx.send_notification(CancelNotification::new(self.id.clone())).map_err(agent_error)
    }
}
