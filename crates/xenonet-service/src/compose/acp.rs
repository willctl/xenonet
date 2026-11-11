//! The apps XenoNet starts over ACP (the registry's `acp`): the runtime's
//! ACP side (`xenonet_runtime::acp::Acp`), on the generic ACP client
//! (xenonet-acp). Each session XenoNet starts is one agent process, one
//! `Client`, held open between turns and closed when it's been idle for
//! `IDLE`, or when the service stops.
//!
//! - A start: the agent's command as its adapter found it, run with the
//!   service's environment as a resumed turn's is (no other app's marks),
//!   which the client strips of XenoNet's credentials and every token; a
//!   session opened in the folder; and the first prompt as a `Turn`, framed
//!   with who asked.
//! - Its id on the map is `<app>:<the agent's session id>`: the agent picks
//!   it, and Grok Build's transcript folder is named by it. XenoNet's own
//!   agent ids stay behind this edge: the router and the asks name the
//!   session as the map does, and this side alone knows the ACP session.
//! - A message the router routes to it, after its consent and wake checks,
//!   is its next turn, framed as hook delivery frames it.
//! - A permission request is an ask held in the runtime's asks, shown in
//!   Needs you, and answered there by the person, an actor's allow scope,
//!   or its hold running out (a Deny). An Allow carries what decide made
//!   for that ask, which the client checks before it picks allow-once.
//! - What the session does goes to the activity feed, for an app whose
//!   transcripts XenoNet doesn't read (Grok Build's it reads).

use futures::future::BoxFuture;
use futures::FutureExt as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use xenonet_acp::schema::{ContentBlock, SessionId, SessionUpdate, StopReason, ToolKind};
use xenonet_acp::{AcpError, Answer, Client, Host, Launch, Prompt, Session, Turn};
use xenonet_api::From;
use xenonet_authz::Information;
use xenonet_engine::delivery::starts::Finish;
use xenonet_os::sync::Locked as _;
use xenonet_runtime::acp::{Acp, Failed, Message, Start, Started};
use xenonet_runtime::runtime::Runtime;
use xenonet_runtime::starts::Answer as Told;

/// How long a session is held open after its last turn, for a reply,
/// before its agent is closed.
const IDLE: Duration = Duration::from_mins(30);
/// How long an agent has to start and open its session.
const OPENS: Duration = Duration::from_secs(90);

/// Puts the ACP side in place on the runtime.
pub fn plug(w: &Arc<Runtime>) {
    let _ = w.acp.set(Arc::new(AcpSide::default()));
}

#[derive(Default)]
pub struct AcpSide {
    /// The sessions held open, by their id on the map.
    open: Arc<Mutex<HashMap<String, Arc<Held>>>>,
}

/// A session XenoNet holds open: its client (taken when it's closed), the
/// session, and its turns.
struct Held {
    app: String,
    agent: String,
    client: tokio::sync::Mutex<Option<Client>>,
    session: Session,
    host: Arc<Hears>,
    /// A turn is under way.
    busy: AtomicBool,
    /// Counts the turns, so an idle close knows no turn came since.
    turns: AtomicU64,
    told: Told,
    w: Weak<Runtime>,
}

/// What the client tells XenoNet of one agent: its permission requests,
/// held as asks, and its sessions' updates.
struct Hears {
    w: Weak<Runtime>,
    app: String,
    /// The session's id on the map, once it's open.
    agent: OnceLock<String>,
    /// Whether the activity feed hears of it here (the map doesn't read
    /// its transcripts).
    feed: bool,
    /// The turn's words so far, and the tools denied in it.
    said: Mutex<String>,
    denied: Arc<Mutex<Vec<String>>>,
}

impl Hears {
    fn agent(&self, session: &SessionId) -> String {
        self.agent.get().cloned().unwrap_or_else(|| format!("{}:{}", self.app, session.0))
    }

    fn turn_begins(&self) {
        self.said.locked().clear();
        self.denied.locked().clear();
    }
}

impl Host for Hears {
    fn ask(&self, prompt: Prompt) -> BoxFuture<'static, Answer> {
        let Some(w) = self.w.upgrade() else { return async { Answer::Deny }.boxed() };
        let me = self.agent(&prompt.session);
        let tool = Some(prompt.title.read().trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| "a tool".into());
        let denied = self.denied.clone();
        // Held on a blocking thread, as a run's ask is: the hold can last
        // ten minutes.
        let asked = tokio::task::spawn_blocking(move || {
            let ok = xenonet_runtime::asks::hold_acp(&w, &me, None, &prompt.ask, &tool, prompt.kind, prompt.input.read());
            (ok, tool)
        });
        async move {
            match asked.await {
                Ok((Some(ok), _)) => Answer::Allow(ok),
                Ok((None, tool)) => {
                    denied.locked().push(tool);
                    Answer::Deny
                }
                Err(_) => Answer::Deny,
            }
        }
        .boxed()
    }

    fn update(&self, session: &SessionId, update: SessionUpdate) {
        if let SessionUpdate::AgentMessageChunk(c) = &update {
            if let ContentBlock::Text(t) = &c.content {
                self.said.locked().push_str(&t.text);
            }
        }
        if !self.feed {
            return;
        }
        let Some(w) = self.w.upgrade() else { return };
        if let Some(ev) = activity(&self.app, &self.agent(session), &update, xenonet_text::time::now_ms()) {
            w.emit(From::State, "activity", ev);
        }
    }

    fn refused(&self, method: &'static str) {
        if let Some(w) = self.w.upgrade() {
            w.log("warn", "acp.refused", &format!("{} asked for {method}, which XenoNet never offers", self.app));
        }
    }
}

/// What a session update shows in the activity feed, as the map's readers
/// write a tool call or a turn's end: None for what it doesn't show.
pub fn activity(app: &str, session: &str, update: &SessionUpdate, now: f64) -> Option<Value> {
    let SessionUpdate::ToolCall(call) = update else { return None };
    let category = match call.kind {
        ToolKind::Read | ToolKind::Search => "read",
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => "edit",
        ToolKind::Execute => "shell",
        ToolKind::Fetch => "web",
        _ => "tool",
    };
    let files: Vec<String> = call.locations.iter().map(|l| l.path.to_string_lossy().into_owned()).collect();
    let mut ev = json!({ "sessionId": session, "app": app, "kind": "tool", "category": category, "detail": clip(&call.title, 200), "ts": now });
    if !files.is_empty() {
        ev["files"] = json!(files);
    }
    Some(ev)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// How a turn ended, as a start's answer reads a run's: its last words,
/// whether it failed, and the tools denied in it.
pub fn finish(ended: &Result<StopReason, AcpError>, said: &str, denied: Vec<String>) -> Finish {
    match ended {
        Ok(StopReason::EndTurn | StopReason::Cancelled) => Finish { text: said.trim().to_string(), error: false, denied },
        Ok(other) => Finish { text: format!("the turn ended: {}", stop_words(*other)), error: true, denied },
        Err(e) => Finish { text: e.to_string(), error: true, denied },
    }
}

fn stop_words(s: StopReason) -> &'static str {
    match s {
        StopReason::MaxTokens => "it reached its most tokens",
        StopReason::MaxTurnRequests => "it reached its most requests in a turn",
        StopReason::Refusal => "the agent refused it",
        _ => "it stopped",
    }
}

/// The variables that mark a process as another app's session, as the
/// registry's adapters give them: the agent leaves them out.
fn marks(app: &str) -> Vec<&'static str> {
    xenonet_engine::delivery::apps::apps().into_iter().filter(|(a, _)| a.id != app).filter_map(|(_, w)| w.marks).collect()
}

/// Waits on async work from whichever thread a start runs on: a blocking
/// thread, or a worker of the service's runtime.
fn wait<T: Send + 'static>(w: &Runtime, f: impl std::future::Future<Output = T> + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    w.rt.spawn(async move {
        let _ = tx.send(f.await);
    });
    if tokio::runtime::Handle::try_current().is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread) {
        tokio::task::block_in_place(|| rx.recv().ok())
    } else {
        rx.recv().ok()
    }
}

impl AcpSide {
    /// A turn of a held session: its updates heard, its end told to
    /// whoever asked for the start, and the session closed once it's been
    /// idle for `IDLE`.
    fn run(&self, held: Arc<Held>, turn: Turn) {
        let Some(w) = held.w.upgrade() else { return };
        let open = self.open.clone();
        held.busy.store(true, Ordering::SeqCst);
        let n = held.turns.fetch_add(1, Ordering::SeqCst) + 1;
        held.host.turn_begins();
        w.rt.clone().spawn(async move {
            let at = Instant::now();
            let ended = held.session.prompt(turn).await;
            let said = held.host.said.locked().clone();
            let denied = std::mem::take(&mut *held.host.denied.locked());
            let f = finish(&ended, &said, denied);
            held.busy.store(false, Ordering::SeqCst);
            let minutes = (at.elapsed().as_secs_f64() / 60.0).round();
            if let Some(w) = held.w.upgrade() {
                if held.host.feed {
                    let interrupted = matches!(ended, Ok(StopReason::Cancelled));
                    w.emit(
                        From::State,
                        "activity",
                        json!({ "sessionId": held.agent, "app": held.app, "kind": "turn-end", "interrupted": interrupted, "ts": xenonet_text::time::now_ms() }),
                    );
                }
                let body = xenonet_runtime::starts::acp_turn_end(&w, &held.app, &held.agent, &f, minutes, true);
                (held.told)(&w, "ended", body);
            }
            // An agent that stopped is closed now; one that answered waits
            // `IDLE` for a reply first.
            if !matches!(ended, Err(AcpError::Start(_))) {
                tokio::time::sleep(IDLE).await;
                if held.turns.load(Ordering::SeqCst) != n || held.busy.load(Ordering::SeqCst) {
                    return;
                }
            }
            open.locked().remove(&held.agent);
            if let Some(c) = held.client.lock().await.take() {
                c.close().await;
            }
            if let Some(w) = held.w.upgrade() {
                w.log("info", "acp", &format!("{}'s agent was closed", held.agent));
                (held.told)(&w, "ended", json!({ "session": held.agent, "minutes": 0, "app": held.app, "live": false, "closed": true }));
            }
        });
    }
}

impl Acp for AcpSide {
    fn start(&self, w: &Arc<Runtime>, s: Start) -> Result<Started, Failed> {
        let env = xenonet_engine::delivery::resume::resume_env(std::env::vars().collect());
        let launch = Launch::new(&s.program, s.args.clone(), &s.cwd, env, &marks(&s.app));
        let feed = xenonet_adapter::platforms().get(&s.app).is_none_or(|p| !p.app().has("lines"));
        let host =
            Arc::new(Hears { w: Arc::downgrade(w), app: s.app.clone(), agent: OnceLock::new(), feed, said: Mutex::new(String::new()), denied: Arc::new(Mutex::new(Vec::new())) });
        let (h, cwd) = (host.clone(), s.cwd.clone());
        // No session in `OPENS`: the client is dropped, which stops the
        // agent's tree. XenoNet's own MCP server isn't named in the
        // session: the app's own settings name it, as `xenonet connect`
        // wrote them.
        let opened = wait(w, async move {
            tokio::time::timeout(OPENS, async move {
                let client = Client::start(&launch, h).await?;
                match client.open(std::path::Path::new(&cwd), None).await {
                    Ok(session) => Ok((client, session)),
                    Err(e) => {
                        client.close().await;
                        Err(e)
                    }
                }
            })
            .await
            .ok()
        });
        let name = xenonet_adapter::platforms().get(&s.app).map_or_else(|| s.app.clone(), |p| p.app().name.clone());
        let (client, session) = match opened.flatten() {
            Some(Ok(x)) => x,
            Some(Err(AcpError::SignIn(why))) => return Err(Failed::SignIn(format!("{name} isn't signed in here ({why}): sign in to it yourself, then start it again"))),
            Some(Err(e)) => return Err(Failed::Unproven(e.to_string())),
            None => return Err(Failed::Unproven(format!("{name} didn't open a session in {} s", OPENS.as_secs()))),
        };
        let agent = format!("{}:{}", s.app, session.id().0);
        let _ = host.agent.set(agent.clone());
        let pid = client.pid();
        let held = Arc::new(Held {
            app: s.app.clone(),
            agent: agent.clone(),
            client: tokio::sync::Mutex::new(Some(client)),
            session,
            host,
            busy: AtomicBool::new(false),
            turns: AtomicU64::new(0),
            told: s.answer,
            w: Arc::downgrade(w),
        });
        self.open.locked().insert(agent.clone(), held.clone());
        let delegation = s.delegation.as_ref().map(|(by, on)| (by.as_str(), on.as_str()));
        let turn = Turn::started(&s.by, s.from.as_deref(), &s.prompt, delegation);
        self.run(held, turn);
        Ok(Started { session: agent, pid })
    }

    fn holds(&self, agent: &str) -> bool {
        self.open.locked().get(agent).is_some_and(|h| !h.busy.load(Ordering::SeqCst))
    }

    fn deliver(&self, agent: &str, m: Message<'_>) -> Result<String, String> {
        let held = self.open.locked().get(agent).cloned().ok_or("XenoNet holds no such session open")?;
        // One turn at a time: a second message waits for the hooks.
        if held.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("it's in a turn".into());
        }
        let turn = Turn::message(m.id, m.from, m.verified, &Information::new(m.text.to_string()), m.user);
        self.run(held, turn);
        Ok(format!("its next turn, over ACP ({agent})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_acp::schema::{ToolCall, ToolCallLocation};

    #[test]
    fn a_tool_call_shows_in_the_feed_as_the_readers_write_one() {
        let call = ToolCall::new("t1", "Edit notes.md").kind(ToolKind::Edit).locations(vec![ToolCallLocation::new("/w/notes.md")]);
        let ev = activity("hermes-agent", "hermes-agent:s1", &SessionUpdate::ToolCall(call), 5.0).unwrap();
        assert_eq!(
            ev,
            json!({ "sessionId": "hermes-agent:s1", "app": "hermes-agent", "kind": "tool", "category": "edit", "detail": "Edit notes.md", "files": ["/w/notes.md"], "ts": 5.0 })
        );
        let call = ToolCall::new("t2", "Run tests").kind(ToolKind::Execute);
        assert_eq!(activity("x", "x:s", &SessionUpdate::ToolCall(call), 1.0).unwrap()["category"], "shell");
    }

    #[test]
    fn a_turn_ends_as_a_runs_does() {
        let ok = finish(&Ok(StopReason::EndTurn), " done \n", vec![]);
        assert_eq!((ok.text.as_str(), ok.error), ("done", false));
        let refused = finish(&Ok(StopReason::Refusal), "", vec!["rm".into()]);
        assert!(refused.error && refused.denied == ["rm"]);
        let gone = finish(&Err(AcpError::Start("it closed".into())), "", vec![]);
        assert!(gone.error && gone.text.contains("it closed"));
    }
}
