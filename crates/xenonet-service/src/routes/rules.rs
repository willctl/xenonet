//! Who may call each route of the local API, in one table (`RULES`), and
//! what it reads or does. `admit` checks a request's row before its
//! handler runs: a route with no row is refused, so a new route can't ship
//! without one.
//!
//! **The credential.** Every row but an open one takes a credential: the
//! API credential from `<runtime>/api-<port>.cred` in
//! `x-xenonet-credential` (the command, the hooks, the MCP servers, the e2e
//! harness), the window key as the `xenonet-window-<port>` cookie Desktop
//! sets or in `x-xenonet-window` (XenoNet's page), or an actor's own token
//! as `Authorization: Bearer`, which is decided as the actor. Without one,
//! or with one that isn't this service's (whatever else came with it), the
//! answer is 401. The file keeps out other accounts and whatever can't
//! read it; an agent on this account can, so the file alone never reads
//! what matters (`Checked`).
//!
//! **Who** (`Who`), checked here unless it says the handler does:
//!
//! - `Open`: no credential; the route has a check of its own (the row says
//!   which).
//! - `Tray`: the tray app's key, from your own process.
//! - `Local`: any credential. A token alone is authenticated here.
//! - `Program`: the file's or the window's credential; `Authorization` is
//!   the caller's own, for the service it calls.
//! - `Hook`: a credential; the handler checks the process of the session
//!   the hook's body names.
//! - `Checked(scope)`: a read that matters. The window, the service's own
//!   reads, a session in x-xenonet-agent whose process passes the check,
//!   or your own terminal; decided through xenonet-authz with the process
//!   check as the agent's attestation, where an unchecked caller doesn't
//!   read it. Never an actor's token: actors read through their grants.
//! - `You`: the window, or your own terminal (not from under an agent or
//!   an actor product); never an actor.
//! - `Window`: the window's credential from the window's own process;
//!   never an actor.
//! - `Session`: a session named in x-xenonet-agent, its process checked
//!   here; the handler gets the verdict in `req.by.session`.
//! - `Actor`: an actor's token, which the handler decides as the actor
//!   (ActorCaller, through xenonet-authz), with the right the row's act
//!   names.
//!
//! A row lists the kinds it takes. With a token the caller is the actor;
//! with x-xenonet-agent, that session; else the window or the file's
//! holder.
//!
//! A layer that's off (actors, remote actors, A2A) answers 503 to anyone
//! with a credential, before who's asking is looked at: it has no routes
//! to guard.
//!
//! **What** (`Doing`): the authz act a route is, where one fits. Reads,
//! messages, claims and handoffs are decided here for the caller found
//! (an actor's are its handler's); starts and answers by the engine, with
//! the run or the ask they're for. A row marked `elsewhere` is the
//! window's alone when it names another machine (`?host=`, or an id that
//! starts with `@`).

use crate::cores::{ok, reason};
use crate::ctx::Ctx;
use crate::http::{decode_uri_component, By, Reply, Req};
use serde_json::{json, Value};
use xenonet_authz::{
    Act, Action, AgentId, Attestation, Consent, DeviceId, Door, Ground, MachineId, NetworkId, PersonId, Principal, ReadScope, Request, Resource, Role, RootCertificate, SessionId,
    Settings, StandingGrant,
};
use xenonet_engine::engine::is_agent_id;

/// The header the file's credential comes in.
pub const CREDENTIAL: &str = "x-xenonet-credential";
/// The header the service's own in-process reads carry.
pub const OWN: &str = "x-xenonet-self";

/// Who may call a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Open(&'static str),
    Tray,
    Local,
    Program,
    Hook,
    Checked(ReadScope),
    You,
    Window,
    Session,
    Actor,
}

/// What a route reads or does, as an authz act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    /// No act fits: a setting, a theme, the layout, an integration.
    Nothing,
    /// A read, decided here.
    Read(ReadScope),
    /// A message, claim or handoff, decided here.
    Act(Action),
    /// A start or an answer, decided by the engine with the run or ask.
    Engine(Action),
}

/// One route: its method (`*` for any), its path (`*` is one segment,
/// `**` the rest), who may call it, what it does, and whether it's the
/// window's alone when it names another machine.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub method: &'static str,
    pub path: &'static str,
    pub who: &'static [Who],
    pub act: Doing,
    pub elsewhere: bool,
}

use Doing::{Engine, Nothing, Read};
use ReadScope::{Map, Search, State, Transcripts};
use Who::{Checked, Hook, Local, Open, Program, Session, Tray, Window, You};

const fn r(method: &'static str, path: &'static str, who: &'static [Who], act: Doing) -> Rule {
    Rule { method, path, who, act, elsewhere: false }
}

const fn far(method: &'static str, path: &'static str, who: &'static [Who], act: Doing) -> Rule {
    Rule { method, path, who, act, elsewhere: true }
}

const LOCAL: &[Who] = &[Local];
const YOU: &[Who] = &[You];
const WINDOW: &[Who] = &[Window];
const ACTOR: &[Who] = &[Who::Actor];
const AGENTS: &[Who] = &[Session, Who::Actor];
const MESSAGE: Doing = Doing::Act(Action::Message);

/// Every route of the local API. The first row that matches is the
/// route's, so a path's own rows come before a pattern's.
#[rustfmt::skip]
pub const RULES: &[Rule] = &[
    // ---- open: each has a check of its own ----
    r("GET",    "/api/health", &[Open("what this XenoNet is and that it runs, for anything finding it")], Nothing),
    r("GET",    "/api/spotify/callback", &[Open("Spotify sends your browser back here; the sign-in's state, made here, is its check")], Nothing),
    r("POST",   "/api/hook/claude", &[Open("Claude Code's HTTP hooks can't carry a credential; a hook is taken only from the process of the Claude Code session its body names")], Nothing),
    r("POST",   "/api/hook/claude/listen", &[Open("Claude Code's Stop hook, over HTTP; taken only from the process of the session its body names")], Nothing),

    // ---- the tray app, with its own key ----
    r("GET",    "/api/tray/v1/status", &[Tray], Read(Map)),
    r("GET",    "/api/tray/v1/events", &[Tray], Read(Map)),
    r("POST",   "/api/tray/v1/open-console", &[Tray], Nothing),
    // Any other tray path takes the key too, so none can be probed without it.
    r("*",      "/api/tray/**", &[Tray], Nothing),

    // ---- hooks through the xenonet command ----
    r("POST",   "/api/hook/codex", &[Hook], Nothing),
    r("POST",   "/api/hook/grok", &[Hook], Nothing),

    // ---- the map, for any local surface ----
    r("GET",    "/api/graph", LOCAL, Read(Map)),
    r("GET",    "/api/live", LOCAL, Read(Map)),
    r("GET",    "/api/events", LOCAL, Read(Map)),
    r("GET",    "/api/summary", LOCAL, Read(Map)),
    r("GET",    "/api/history", LOCAL, Read(Map)),
    r("GET",    "/api/digest", LOCAL, Read(Map)),
    r("GET",    "/api/daylog", LOCAL, Read(Map)),
    r("GET",    "/api/constellation", LOCAL, Read(Map)),
    r("GET",    "/api/constellation/labels", LOCAL, Read(Map)),
    r("GET",    "/api/sessions", LOCAL, Read(Map)),
    far("GET",  "/api/session/*/stats", LOCAL, Read(Map)),
    far("GET",  "/api/collision", LOCAL, Read(Map)),
    far("GET",  "/api/mcp", LOCAL, Read(Map)),
    r("GET",    "/api/system", LOCAL, Read(Map)),
    r("GET",    "/api/spend", LOCAL, Read(Map)),
    r("GET",    "/api/spend/actors", LOCAL, Read(Map)),
    r("POST",   "/api/narrate", LOCAL, Read(Map)),
    r("GET",    "/api/runs/forks", LOCAL, Read(Map)),
    r("GET",    "/api/engine/agents", LOCAL, Read(Map)),
    r("GET",    "/api/engine/who", LOCAL, Read(Map)),

    // ---- reads that matter ----
    far("GET",  "/api/session/*/transcript", &[Checked(Transcripts)], Read(Transcripts)),
    r("GET",    "/api/search", &[Checked(Search)], Read(Search)),
    r("GET",    "/api/handoff", &[Checked(Transcripts)], Read(Transcripts)),
    r("GET",    "/api/engine/state", &[Checked(State)], Read(State)),
    r("GET",    "/api/delivery", &[Checked(State)], Read(State)),

    // ---- the window's console and its settings ----
    r("POST",   "/api/ack", LOCAL, Nothing),
    r("POST",   "/api/open", LOCAL, Nothing),
    r("POST",   "/api/handoff", LOCAL, Doing::Act(Action::Handoff)),
    r("GET",    "/api/layout", LOCAL, Nothing),
    r("POST",   "/api/layout", LOCAL, Nothing),
    r("DELETE", "/api/layout", LOCAL, Nothing),
    r("POST",   "/api/logs/page", LOCAL, Nothing),
    r("GET",    "/api/logs", LOCAL, Nothing),
    r("GET",    "/api/themes", LOCAL, Nothing),
    r("POST",   "/api/themes/copy", LOCAL, Nothing),
    r("POST",   "/api/themes/reveal", LOCAL, Nothing),
    r("GET",    "/api/themes/*", LOCAL, Nothing),
    r("GET",    "/api/theme", LOCAL, Nothing),
    r("POST",   "/api/theme", YOU, Nothing),
    r("GET",    "/api/environment", LOCAL, Nothing),
    r("POST",   "/api/environment", YOU, Nothing),
    r("GET",    "/api/command", LOCAL, Nothing),
    r("POST",   "/api/command", LOCAL, Nothing),
    r("GET",    "/api/drift", LOCAL, Nothing),
    r("POST",   "/api/drift/seen", LOCAL, Nothing),
    r("GET",    "/api/doctor", LOCAL, Nothing),
    r("POST",   "/api/doctor", LOCAL, Nothing),
    r("GET",    "/api/diagnose", LOCAL, Nothing),
    r("POST",   "/api/diagnose/ask", YOU, Nothing),
    r("POST",   "/api/session/rename", YOU, Nothing),
    r("GET",    "/api/workspace/root", LOCAL, Nothing),
    r("POST",   "/api/workspace/root", YOU, Nothing),
    r("DELETE", "/api/workspace/root", YOU, Nothing),
    r("POST",   "/api/workspace/check", LOCAL, Nothing),
    r("GET",    "/api/projects", LOCAL, Nothing),
    r("POST",   "/api/projects/rule", YOU, Nothing),
    r("DELETE", "/api/projects/rules", YOU, Nothing),
    r("POST",   "/api/projects/rescan", LOCAL, Nothing),
    r("GET",    "/api/projects/codex", LOCAL, Nothing),
    r("POST",   "/api/projects/codex", YOU, Nothing),
    r("DELETE", "/api/projects/codex", YOU, Nothing),
    r("GET",    "/api/identities", LOCAL, Nothing),
    r("POST",   "/api/identities/name", YOU, Nothing),
    r("POST",   "/api/identities/hidden", YOU, Nothing),
    r("POST",   "/api/identities/pin", YOU, Nothing),
    r("POST",   "/api/identities/rescan", LOCAL, Nothing),
    r("GET",    "/api/spotify", LOCAL, Nothing),
    r("POST",   "/api/spotify/disconnect", YOU, Nothing),
    r("POST",   "/api/spotify/app", WINDOW, Nothing),
    r("POST",   "/api/spotify/connect", WINDOW, Nothing),
    r("POST",   "/api/spotify/settings", WINDOW, Nothing),
    r("POST",   "/api/spotify/player", WINDOW, Nothing),
    r("GET",    "/api/decide", LOCAL, Nothing),
    r("POST",   "/api/decide/key", WINDOW, Nothing),
    r("DELETE", "/api/decide/key", YOU, Nothing),
    r("GET",    "/api/refusals", YOU, Nothing),
    r("POST",   "/api/refusals/context", YOU, Nothing),
    r("GET",    "/api/refusals/*/request", YOU, Nothing),
    r("GET",    "/api/integrations", YOU, Nothing),
    r("GET",    "/api/integrations/history", YOU, Nothing),
    r("GET",    "/api/integrations/pi", YOU, Nothing),
    r("GET",    "/api/integrations/sandbox/review", YOU, Nothing),
    r("POST",   "/api/integrations/route/preview", WINDOW, Nothing),
    r("POST",   "/api/integrations/route/send", WINDOW, Nothing),
    r("POST",   "/api/integrations/route/outcome", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/check", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/preview", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/launch", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/inspect", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/retrieve", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/export", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/forget-review", WINDOW, Nothing),
    r("POST",   "/api/integrations/sandbox/delete", WINDOW, Nothing),
    r("POST",   "/api/integrations/pi/enable", WINDOW, Nothing),
    r("POST",   "/api/integrations/pi/import", WINDOW, Nothing),

    // ---- the engine: sessions through the MCP server, and actors ----
    r("GET",    "/api/engine/connect", LOCAL, Nothing),
    r("POST",   "/api/engine/connect", YOU, Nothing),
    r("DELETE", "/api/engine/connect", YOU, Nothing),
    r("POST",   "/api/engine/whoami", AGENTS, Read(Map)),
    r("POST",   "/api/engine/peers", AGENTS, Read(Map)),
    r("POST",   "/api/engine/who-has", AGENTS, Read(Map)),
    r("POST",   "/api/engine/send", AGENTS, MESSAGE),
    r("POST",   "/api/engine/inbox", AGENTS, Nothing),
    r("POST",   "/api/engine/claim", AGENTS, Doing::Act(Action::Claim)),
    r("POST",   "/api/engine/release", AGENTS, Doing::Act(Action::Claim)),
    r("POST",   "/api/engine/focus", AGENTS, Nothing),
    r("POST",   "/api/engine/channel", AGENTS, Nothing),
    r("POST",   "/api/engine/handoff", AGENTS, Doing::Act(Action::Handoff)),
    r("POST",   "/api/engine/opencode-event", AGENTS, Nothing),

    // ---- starts and the runs XenoNet starts ----
    r("POST",   "/api/start", &[Who::Actor, Session, You], Engine(Action::Start)),
    r("POST",   "/api/runs/permission", AGENTS, Nothing),
    r("GET",    "/api/runs/asks", &[Who::Actor, Window], Read(State)),
    r("POST",   "/api/runs/answer", &[Who::Actor, Window], Engine(Action::Answer)),
    r("GET",    "/api/runs/sign-in", YOU, Nothing),
    r("POST",   "/api/runs/sign-in", YOU, Nothing),
    r("DELETE", "/api/runs/sign-in", YOU, Nothing),

    // ---- delivery's controls ----
    r("POST",   "/api/delivery/pause", YOU, Nothing),
    r("POST",   "/api/delivery/grant", YOU, Nothing),
    r("POST",   "/api/delivery/revoke", YOU, Nothing),
    r("POST",   "/api/delivery/allow", YOU, Nothing),
    r("POST",   "/api/delivery/disallow", YOU, Nothing),

    // ---- actors ----
    r("POST",   "/api/actors/you/message", WINDOW, MESSAGE),
    r("GET",    "/api/actors/you", LOCAL, Read(Map)),
    r("POST",   "/api/actors/you", WINDOW, Nothing),
    r("DELETE", "/api/actors/you", WINDOW, Nothing),
    r("GET",    "/api/actors", LOCAL, Read(Map)),
    r("POST",   "/api/actors", WINDOW, Doing::Act(Action::RegisterActor)),
    r("GET",    "/api/actors/events", LOCAL, Read(Map)),
    r("POST",   "/api/actors/events", ACTOR, Nothing),
    r("POST",   "/api/actors/register", LOCAL, Nothing),
    r("GET",    "/api/actors/me", ACTOR, Nothing),
    r("DELETE", "/api/actors/me", ACTOR, Nothing),
    r("GET",    "/api/actors/me/stream", ACTOR, Nothing),
    r("POST",   "/api/actors/heartbeat", ACTOR, Nothing),
    r("POST",   "/api/actors/targets", ACTOR, Nothing),
    r("GET",    "/api/actors/wakes", YOU, Nothing),
    r("GET",    "/api/actors/requests", WINDOW, Nothing),
    r("POST",   "/api/actors/requests/*", WINDOW, Doing::Act(Action::RegisterActor)),
    r("GET",    "/api/actors/tokens", WINDOW, Nothing),
    r("GET",    "/api/actors/observers", LOCAL, Nothing),
    r("POST",   "/api/actors/observers", YOU, Nothing),
    r("GET",    "/api/actors/gateway", YOU, Nothing),
    r("POST",   "/api/actors/gateway", YOU, Nothing),
    r("POST",   "/api/actors/*/wake/test", YOU, Nothing),
    r("PUT",    "/api/actors/*/wake", WINDOW, Nothing),
    r("DELETE", "/api/actors/*/wake", YOU, Nothing),
    r("POST",   "/api/actors/*/gateway", WINDOW, Doing::Act(Action::RegisterActor)),
    r("DELETE", "/api/actors/*/gateway", YOU, Nothing),
    r("POST",   "/api/actors/*/gateway/ask", LOCAL, Nothing),
    r("PUT",    "/api/actors/*/allows", WINDOW, Doing::Act(Action::RegisterActor)),
    r("DELETE", "/api/actors/*/allows", YOU, Nothing),
    r("POST",   "/api/actors/*/token", WINDOW, Doing::Act(Action::RegisterActor)),
    r("GET",    "/api/actors/*", LOCAL, Read(Map)),
    r("DELETE", "/api/actors/*", YOU, Nothing),
    r("GET",    "/api/a2a", YOU, Nothing),
    r("POST",   "/api/a2a", YOU, Nothing),

    // ---- remote actors' connections ----
    r("GET",    "/api/assistant", YOU, Nothing),
    r("GET",    "/api/assistant/places", YOU, Nothing),
    r("GET",    "/api/assistant/audit", YOU, Nothing),
    r("GET",    "/api/assistant/tunnels", YOU, Nothing),
    r("POST",   "/api/assistant/disable", YOU, Nothing),
    r("POST",   "/api/assistant/enable", WINDOW, Nothing),
    r("POST",   "/api/assistant/preview", WINDOW, Nothing),
    r("POST",   "/api/assistant/pair", WINDOW, Doing::Act(Action::RegisterActor)),
    r("POST",   "/api/assistant/*/tunnel", WINDOW, Nothing),
    r("POST",   "/api/assistant/*/tunnel/stop", YOU, Nothing),
    r("POST",   "/api/assistant/*/tunnel/forget", YOU, Nothing),
    r("POST",   "/api/assistant/*/tunnel/*", WINDOW, Nothing),
    r("POST",   "/api/assistant/*/revoke", YOU, Nothing),
    r("POST",   "/api/assistant/*/rotate", WINDOW, Nothing),
    r("POST",   "/api/assistant/*", WINDOW, Doing::Act(Action::RegisterActor)),

    // ---- other machines ----
    r("GET",    "/api/net/state", YOU, Read(State)),
    r("GET",    "/api/net/svc", YOU, Read(State)),
    r("POST",   "/api/net/create", YOU, Nothing),
    r("POST",   "/api/net/join", YOU, Nothing),
    r("POST",   "/api/net/leave", YOU, Nothing),
    r("POST",   "/api/net/ticket", WINDOW, Nothing),
    r("POST",   "/api/net/remove", YOU, Nothing),
    r("POST",   "/api/net/replace", YOU, Nothing),
    r("POST",   "/api/net/approve", YOU, Nothing),
    r("POST",   "/api/net/gateway/take", YOU, Nothing),
    r("POST",   "/api/net/gateway/set", YOU, Nothing),
    r("POST",   "/api/net/svc/publish", YOU, Nothing),
    r("POST",   "/api/net/svc/unpublish", YOU, Nothing),
    r("PUT",    "/api/net/settings", YOU, Nothing),
    r("*",      "/api/svc/**", &[Program], Nothing),
    r("GET",    "/api/hosts", LOCAL, Read(Map)),
    r("POST",   "/api/hosts", YOU, Nothing),
    r("GET",    "/api/hosts/self", LOCAL, Read(Map)),
    r("POST",   "/api/hosts/self", YOU, Nothing),
    r("POST",   "/api/hosts/*/reconnect", LOCAL, Nothing),
    r("POST",   "/api/hosts/*/test", LOCAL, Nothing),
    r("GET",    "/api/hosts/*", LOCAL, Read(Map)),
    r("DELETE", "/api/hosts/*", YOU, Nothing),

    // ---- XenoNet Dev (each answers only in a development instance) ----
    r("POST",   "/api/dev/simulate", LOCAL, Nothing),
    r("POST",   "/api/dev/control", LOCAL, Nothing),
    r("POST",   "/api/dev/capture", LOCAL, Nothing),
];

/// The row of `method` `path`, or None.
pub fn rule(method: &str, path: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| (r.method == "*" || r.method == method) && matches(r.path, path))
}

/// Whether `path` fits `pattern`: segment by segment, `*` any one that
/// isn't empty, `**` one or more to the end.
fn matches(pattern: &str, path: &str) -> bool {
    let mut want = pattern.split('/');
    let mut got = path.split('/');
    loop {
        match (want.next(), got.next()) {
            (None, None) => return true,
            (Some("**"), Some(s)) => return !s.is_empty() || got.next().is_some(),
            (Some("*"), Some(s)) if !s.is_empty() => {}
            (Some(w), Some(s)) if w == s => {}
            _ => return false,
        }
    }
}

/// What a request with no row gets: the not-found of the routes it's
/// among, as their handlers word it.
fn not_found(path: &str) -> Reply {
    let under = |p: &str| path == p || path.starts_with(&format!("{p}/"));
    if under("/api/actors") {
        return Reply::coded(404, "not-found", "XenoNet has no such route for actors");
    }
    if under("/api/assistant") {
        return Reply::coded(404, "not-found", "XenoNet has no such route for remote actors");
    }
    if under("/api/integrations") {
        return Reply::error(404, "No such integration route.");
    }
    Reply::error(404, "not found")
}

/// A layer that's off has no routes to guard: its routes answer so, to
/// anyone with a credential, before who's asking is looked at.
fn off(ctx: &Ctx, path: &str) -> Option<Reply> {
    let under = |p: &str| path == p || path.starts_with(&format!("{p}/"));
    let no = |error: &str| crate::ctx::fail(503, "off", error, &[], &json!({}));
    // The window's message goes to sessions too, actors or none.
    if under("/api/actors") && path != "/api/actors/you/message" && ctx.cores.actors.is_none() {
        return Some(no("Actors are off here (XENONET_ACTORS=off)"));
    }
    if under("/api/assistant") && ctx.cores.connections.is_none() {
        return Some(no("Actors are off here (XENONET_ACTORS=off), and remote actors with them"));
    }
    if path == "/api/a2a" && ctx.a2a.get().is_none() {
        return Some(no("A2A isn't here: actors are off"));
    }
    None
}

const NO_CREDENTIAL: &str =
    "XenoNet's API takes a credential: XenoNet's own programs read it from api-<port>.cred in its runtime folder (x-xenonet-credential), its window has its own, and an actor brings its token";
const ACTOR_WINDOW: &str = "this is for XenoNet's own window, and an actor can't do it";
const ACTOR_YOURS: &str = "this is yours to do, and an actor can't do it";
const NO_SESSION: &str = "say which session is calling, in x-xenonet-agent (claude:<session id> or codex:<thread id>)";
const NOT_WINDOW: &str = "this is for XenoNet's own window, and the request didn't come with its key";

fn refused(status: u16, code: &str, error: impl Into<String>) -> Reply {
    Reply::coded(status, code, error)
}

/// The credentials a request came with, as they check out, and whether
/// one it came with isn't this service's (a wrong one is refused, whatever
/// else came with it).
struct Creds {
    window: bool,
    file: bool,
    own: bool,
    token: bool,
    wrong: bool,
}

/// The window key in the page's cookie, `xenonet-window-<port>`.
fn cookie_key(req: &Req, port: u16) -> Option<&str> {
    let name = format!("xenonet-window-{port}");
    req.header("cookie")?.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v)
}

impl Creds {
    fn of(ctx: &Ctx, req: &Req) -> Creds {
        let key = &ctx.window_key;
        let (header, cookie) = (req.header("x-xenonet-window"), cookie_key(req, ctx.config.port));
        let (file, own) = (req.header(CREDENTIAL), req.header(OWN));
        let wrong = header.is_some_and(|h| !key.matches(Some(h)))
            || cookie.is_some_and(|c| !key.matches(Some(c)))
            || file.is_some_and(|f| !ctx.api_key.matches(Some(f)))
            || own.is_some_and(|o| !ctx.self_key.matches(Some(o)));
        Creds {
            window: key.matches(header) || key.matches(cookie),
            file: ctx.api_key.matches(file),
            own: ctx.self_key.matches(own),
            token: req.header("authorization").is_some(),
            wrong,
        }
    }
}

/// Whether a request names another machine: `?host=`, or an id (in the
/// path, or `?a=` / `?b=`) that starts with `@`.
fn names_elsewhere(req: &Req) -> bool {
    let at = |s: &str| decode_uri_component(s).is_some_and(|d| d.starts_with('@'));
    req.param("host").is_some_and(|h| !h.is_empty()) || ["a", "b"].iter().any(|k| req.param(k).is_some_and(at)) || req.path.split('/').any(at)
}

/// This machine as its own local door sees it. Local acts are decided
/// with no network: nothing here is another machine's, so only who asks
/// and how they were checked matter, and the machine's name doesn't.
struct Here(MachineId);

impl Ground for Here {
    fn own_machine(&self) -> &MachineId {
        &self.0
    }
    fn role(&self, _: &NetworkId, _: &MachineId) -> Option<Role> {
        None
    }
    fn settings(&self, _: Option<&NetworkId>) -> Settings {
        Settings::default()
    }
    fn own_person(&self) -> Option<PersonId> {
        None
    }
    fn device(&self, _: &PersonId, _: &DeviceId) -> Option<xenonet_authz::Device> {
        None
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
    fn consent(&self, _: &SessionId) -> Consent {
        Consent::default()
    }
    fn device_of(&self, _: &MachineId) -> Option<DeviceId> {
        None
    }
    fn delegations(&self) -> Vec<StandingGrant> {
        Vec::new()
    }
    fn allow_scope(&self, _: &[u8]) -> Option<xenonet_authz::AllowScope> {
        None
    }
    fn ask(&self, _: &xenonet_authz::AskId) -> Option<xenonet_authz::Asked> {
        None
    }
    fn governance_synced_at(&self) -> Option<i64> {
        None
    }
}

/// Who the caller is, for authz: this machine (its window, its own reads,
/// you at your terminal), or an agent with how it was checked.
enum Caller {
    Machine,
    Agent(String, Attestation),
}

fn here() -> Option<MachineId> {
    MachineId::new("this-machine").ok()
}

/// Decides `act` for `caller` at this machine's local door: Ok, or why
/// not.
fn decide(caller: &Caller, act: Act, now: f64) -> Result<(), String> {
    let machine = here().ok_or("this machine has no id to decide by")?;
    let principal = match caller {
        Caller::Machine => Principal::Machine { id: machine.clone() },
        Caller::Agent(id, attestation) => {
            let id = AgentId::new(id).or_else(|_| AgentId::new("unchecked")).map_err(|e| e.message)?;
            Principal::Agent { id, machine: machine.clone(), attestation: *attestation }
        }
    };
    let request = Request::new(principal, Resource::Machine(machine.clone()), act, Door::Local, now as i64, None);
    match xenonet_authz::decide(None, &request, &Here(machine), None) {
        xenonet_authz::Decision::Allow(_) => Ok(()),
        d => Err(d.reason().map_or_else(|| "not allowed".to_string(), |r| r.text().to_string())),
    }
}

fn act_of(doing: Doing) -> Option<Act> {
    match doing {
        Read(scope) => Some(Act::Read { scope, filters: None }),
        Doing::Act(Action::Message) => Some(Act::Message),
        Doing::Act(Action::Claim) => Some(Act::Claim),
        Doing::Act(Action::Handoff) => Some(Act::Handoff),
        _ => None,
    }
}

/// How a session's verdict counts: checked, unless the check passed
/// because nothing could be seen.
fn attestation_of(verdict: &Value) -> Attestation {
    if verdict.get("checked") == Some(&Value::Bool(false)) {
        Attestation::Unchecked
    } else {
        Attestation::ProcessChecked
    }
}

fn scope_words(scope: ReadScope) -> &'static str {
    match scope {
        Map => "the map",
        Transcripts => "what sessions said",
        Search => "a search of what sessions said",
        State => "this machine's messages, deliveries and network",
    }
}

/// Checks a request against its row before its handler runs, and fills
/// in `req.by`: Ok to go on, or the refusal to answer.
pub async fn admit(ctx: &Ctx, req: &mut Req) -> Result<(), Reply> {
    let Some(row) = rule(&req.method, &req.path) else { return Err(not_found(&req.path)) };
    let has = |w: Who| row.who.contains(&w);
    if row.who.iter().any(|w| matches!(w, Open(_))) {
        return Ok(());
    }
    if has(Tray) {
        return tray(ctx, req).await;
    }
    let creds = Creds::of(ctx, req);
    let program_only = row.who.iter().all(|w| matches!(w, Program | Hook));
    if creds.wrong || !(creds.window || creds.file || creds.own || (creds.token && !program_only)) {
        return Err(refused(401, "credential", NO_CREDENTIAL));
    }
    if let Some(r) = off(ctx, &req.path) {
        return Err(r);
    }
    let mut by = By { not_window: NOT_WINDOW.into(), ..By::default() };
    if creds.window {
        if creds.token {
            by.not_window = ACTOR_WINDOW.into();
        } else {
            let seen = ctx.cores.verify.window(req.peer).await;
            by.window = ok(&seen);
            if !by.window {
                by.not_window = reason(&seen);
            }
        }
    }
    if row.elsewhere && !by.window && names_elsewhere(req) {
        return Err(crate::routes::hosts::window_only_reply());
    }
    let caller = who(ctx, req, row, &creds, &mut by).await?;
    // The act, for whoever isn't an actor: an actor's is its handler's.
    if let (Some(caller), Some(act)) = (caller, act_of(row.act)) {
        if let Err(why) = decide(&caller, act, ctx.now()) {
            return Err(refused(403, "not-allowed", why));
        }
    }
    req.by = by;
    Ok(())
}

/// Who the caller is by the row's kinds: the caller authz decides the act
/// for (None for an actor, whose handler decides), or the refusal.
async fn who(ctx: &Ctx, req: &Req, row: &Rule, creds: &Creds, by: &mut By) -> Result<Option<Caller>, Reply> {
    let has = |w: Who| row.who.contains(&w);
    let checked = row.who.iter().find_map(|w| if let Checked(s) = w { Some(*s) } else { None });
    let surface = creds.window || creds.file || creds.own;
    // With a token, the caller is the actor.
    if creds.token && !row.who.iter().all(|w| matches!(w, Program | Hook)) {
        if has(Who::Actor) {
            return Ok(None);
        }
        if has(Local) {
            if surface {
                return Ok(Some(Caller::Agent(String::new(), Attestation::Unchecked)));
            }
            let Some(caller) = ctx.caller.as_ref() else { return Err(refused(401, "credential", NO_CREDENTIAL)) };
            return match caller.authenticate(req, false).await {
                Ok(_) => Ok(None),
                Err(r) => Err(crate::ctx::fail(r.status, &r.code, r.error, &r.headers, &r.extra)),
            };
        }
        return Err(refused(403, "yours", if has(Window) && !has(You) { ACTOR_WINDOW } else { ACTOR_YOURS }));
    }
    // A session that says who it is, checked against its process.
    if (has(Session) || checked.is_some()) && !creds.token {
        if let Some(me) = req.header("x-xenonet-agent").filter(|m| is_agent_id(m)).map(str::to_string) {
            let verdict = ctx.cores.verify.check(req.peer, &me, false).await;
            if !ok(&verdict) {
                return Err(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
            }
            let attestation = attestation_of(&verdict);
            by.session = Some((me.clone(), verdict));
            if has(Session) || checked.is_some() {
                return Ok(Some(Caller::Agent(me, attestation)));
            }
        }
    }
    if has(Local) || has(Program) || has(Hook) {
        return Ok(Some(if by.window || creds.own { Caller::Machine } else { Caller::Agent(String::new(), Attestation::Unchecked) }));
    }
    if by.window && (has(Window) || has(You) || checked.is_some()) {
        return Ok(Some(Caller::Machine));
    }
    if let Some(scope) = checked {
        if creds.own {
            return Ok(Some(Caller::Machine));
        }
        let you = ctx.cores.verify.person(req.peer).await;
        if ok(&you) {
            return Ok(Some(Caller::Machine));
        }
        let caller = Caller::Agent(String::new(), Attestation::Unchecked);
        let why = decide(&caller, Act::Read { scope, filters: None }, ctx.now()).err().unwrap_or_default();
        return Err(refused(
            403,
            "checked",
            format!("{why}: {} reads only from XenoNet's window, a session XenoNet can check (x-xenonet-agent), or your own terminal, and {}", scope_words(scope), reason(&you)),
        ));
    }
    if has(You) {
        let you = ctx.cores.verify.person(req.peer).await;
        return if ok(&you) { Ok(Some(Caller::Machine)) } else { Err(refused(403, "yours", reason(&you))) };
    }
    if has(Window) {
        return Err(refused(403, "yours", by.not_window.clone()));
    }
    // An actor's row without its token: the handler says it takes one,
    // and a session's row without a session says which it needs.
    if has(Who::Actor) {
        return Ok(None);
    }
    Err(Reply::error(400, NO_SESSION))
}

/// The tray's routes: its key, never an actor's token, from your own
/// process.
async fn tray(ctx: &Ctx, req: &Req) -> Result<(), Reply> {
    let no = |error: &str| Reply::send(403, &json!({ "error": error, "code": "tray" }), &[]);
    if req.header("authorization").is_some() {
        return Err(no("this is for XenoNet's tray app, and an actor can't use it"));
    }
    if !ctx.tray_key.matches(req.header("x-xenonet-tray")) {
        return Err(no("this is for XenoNet's tray app, and the request didn't come with its key"));
    }
    let you = ctx.cores.verify.person(req.peer).await;
    if !ok(&you) {
        return Err(refused(403, "yours", reason(&you)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_fit_segment_by_segment() {
        assert!(matches("/api/actors/*/wake", "/api/actors/wren/wake"));
        assert!(!matches("/api/actors/*/wake", "/api/actors//wake"), "a segment isn't empty");
        assert!(!matches("/api/actors/*", "/api/actors/wren/wake"));
        assert!(matches("/api/svc/**", "/api/svc/desk/notes/a/b"));
        assert!(matches("/api/svc/**", "/api/svc/desk"));
        assert!(!matches("/api/svc/**", "/api/svc/"));
        assert!(!matches("/api/graph", "/api/graph/"));
    }

    #[test]
    fn every_route_has_one_first_row_and_a_path_without_one_is_refused() {
        assert_eq!(rule("GET", "/api/actors/you").unwrap().who, &[Local]);
        assert_eq!(rule("POST", "/api/actors/you/message").unwrap().who, &[Window]);
        assert_eq!(rule("GET", "/api/actors/wren").unwrap().act, Read(Map));
        assert_eq!(rule("POST", "/api/assistant/gr_0123456789abcdef01234567/tunnel/stop").unwrap().who, &[You]);
        assert_eq!(rule("POST", "/api/assistant/gr_0123456789abcdef01234567/tunnel/start").unwrap().who, &[Window]);
        assert_eq!(rule("PATCH", "/api/svc/desk/notes/x").unwrap().who, &[Program]);
        // No row, no route: a path made up now, or a method a path doesn't take.
        assert!(rule("GET", "/api/brand-new").is_none());
        assert!(rule("PUT", "/api/graph").is_none());
        assert!(rule("GET", "/api/projects/rules").is_none());
        assert_eq!(not_found("/api/actors/x/y/z").body_json()["code"], "not-found");
        assert_eq!(not_found("/api/brand-new").body_json(), json!({ "error": "not found" }));
        // Every row says who; the open ones are these, each with its reason.
        assert!(RULES.iter().all(|r| !r.who.is_empty()));
        let open: Vec<&str> = RULES.iter().filter(|r| r.who.iter().any(|w| matches!(w, Open(why) if !why.is_empty()))).map(|r| r.path).collect();
        assert_eq!(open, ["/api/health", "/api/spotify/callback", "/api/hook/claude", "/api/hook/claude/listen"]);
        // Rows are each other's only once: no row is shadowed by one above it.
        for (i, row) in RULES.iter().enumerate() {
            let probe = row.path.replace("**", "x/y").replace('*', "x");
            let method = if row.method == "*" { "GET" } else { row.method };
            let first = RULES.iter().position(|r| (r.method == "*" || r.method == method) && matches(r.path, &probe)).unwrap();
            assert!(first == i || RULES[first].path != row.path, "{} {} is shadowed by row {first}", row.method, row.path);
        }
    }

    #[test]
    fn reads_that_matter_need_more_than_the_file() {
        for scope in [Transcripts, Search, State] {
            assert!(decide(&Caller::Machine, Act::Read { scope, filters: None }, 0.0).is_ok());
            assert!(decide(&Caller::Agent("claude:s".into(), Attestation::ProcessChecked), Act::Read { scope, filters: None }, 0.0).is_ok());
            let why = decide(&Caller::Agent(String::new(), Attestation::Unchecked), Act::Read { scope, filters: None }, 0.0).unwrap_err();
            assert!(why.contains("unchecked agent"), "{why}");
        }
        // The map, a message, a claim and a handoff are anyone's here.
        let anyone = Caller::Agent(String::new(), Attestation::Unchecked);
        for act in [Act::Read { scope: Map, filters: None }, Act::Message, Act::Claim, Act::Handoff] {
            assert!(decide(&anyone, act, 0.0).is_ok());
        }
    }

    #[test]
    fn another_machine_is_named_by_host_or_an_at_id() {
        let named = |target: &str| names_elsewhere(&Req::fake("GET", target, &[], None));
        assert!(named("/api/mcp?server=x&host=desk"));
        assert!(!named("/api/mcp?server=x&host="));
        assert!(named("/api/session/%40desk%3Aclaude%3Aabc/transcript"));
        assert!(named("/api/collision?a=%40desk:claude:a&b=claude:b"));
        assert!(!named("/api/session/claude%3Aabc/transcript"));
    }

    #[test]
    fn the_cookie_is_found_by_its_port() {
        let req = Req::fake("GET", "/api/graph", &[("cookie", "a=b; xenonet-window-47840=k1;xenonet-window-47842=k2")], None);
        assert_eq!(cookie_key(&req, 47842), Some("k2"));
        assert_eq!(cookie_key(&req, 47840), Some("k1"));
        assert_eq!(cookie_key(&req, 1), None);
    }
}
