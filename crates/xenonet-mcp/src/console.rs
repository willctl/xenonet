//! The console operator (mcp/console.js), `xenonet mcp console`: an MCP
//! server through which Claude Code, Codex and any other MCP client launch,
//! drive and see XenoNet. Installed Tools operate the installed app; a
//! checkout also offers its development instances (core/instances.js).
//! Every tool goes through core/control.js's client, which whoever serves
//! this gives as a `Control`: the command's driver of xenonet-app's
//! control, which xenonet console, script and film use too.
//!
//! Tools act on one instance: demo (made-up data, the default in a
//! checkout), dev (your real sessions), test, film, showcase or installed.
//! A tool's `instance` picks one, and it stays picked for later calls.

use crate::tools::{self, Def};
use crate::zod::{array, boolean, enumeration, int, number, object, string, union, Z};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, DiscoverResult, Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_app::instances::INSTANCES;
use xenonet_base::text::truthy;

const NAME: &str = "xenonet-console";

const INSTRUCTIONS_TOOLS: &str = "XenoNet maps the Claude Code, Codex and Grok Build sessions on this machine, through a background service, a CLI, MCP tools and a desktop console. These tools operate installed Desktop. They launch it, find sessions, inspect panels, control terminals and take screenshots. Each action waits for the view to settle and returns the new state. Use logs to read page errors. The service, terminals and coordination keep running when Desktop closes or restarts, and window actions report that Desktop is unavailable until it returns. Allowing actors, connecting agents, and trusting hosts are the person's to do. This console has no simulation and no arbitrary page execution.";

const INSTRUCTIONS: &str = "XenoNet maps the Claude Code, Codex and Grok Build sessions on this machine, through a background service, a CLI, MCP tools and a desktop console. These tools operate installed XenoNet and development instances. Use installed for the person's app, or demo for made-up data. The service and the coordination between sessions keep running when Desktop closes or restarts. Simulation and arbitrary page execution work only in development instances.

The loop: `launch` (the demo instance, with made-up data, unless you need the user's real sessions: instance \"dev\"), then act with `find`, `select`, `navigate`, `command` and `input`, then `screenshot` to look. Every action waits until the view has settled (the layout cooled, the camera and any gathering at rest) and returns the new state, so don't sleep between calls. `simulate` puts sessions into rare states (a question waiting, an edit collision, a stuck loop) and has actors act. `actors` lists the actors beside the map, and `feed` reads the activity feed. After a change, check `logs` for page errors.

XenoNet Dev hot-reloads: saving under src/ reloads the page, under server/ restarts the server, under electron/ relaunches the app; the tools wait for it to come back. The window shows without taking focus, and the user may be watching it. Screenshots of the dev instance show the user's real session titles.";

/// core/control.js's client for an instance: one of its methods, `kind`,
/// with its arguments, as the command's driver takes them: ping, launch
/// (`{ show, restart }`), quit, driver (`{ method, args }`), simulate
/// (`{ kind, a, b }`), screenshot, input, window and logs (`{ opts }`),
/// and eval (`{ expression }`). Err is the message of what the JS threw.
/// It blocks, so the server calls it on a thread of its own.
pub trait Control: Send + Sync + 'static {
    fn method(&self, instance: &str, kind: &str, args: Value) -> Result<Value, String>;
}

const KINDS: [&str; 10] = ["project", "session", "native", "zone", "resource", "wiki", "page", "core", "actor", "host"];
const SIMULATIONS: [&str; 29] = [
    "needs-you",
    "collision",
    "stuck",
    "wiki-write",
    "turn-end",
    "doctor",
    "clear",
    "actor-spawn",
    "actor-edit",
    "actor-message",
    "actor-online",
    "actor-offline",
    "actor-request",
    "actor-needs-you",
    "actor-tunnel",
    "actor-tunnel-down",
    "actor-read",
    "host-add",
    "host-offline",
    "host-online",
    "host-skew",
    "net-none",
    "net-founder",
    "net-member",
    "net-gateway-down",
    "net-ticket",
    "net-mismatch",
    "net-remote-waiting",
    "net-approval",
];

const LOOK: &str = "Read or change how the console draws its panels. plainPanels true draws them solid, without the glass blur behind them, a large share of a visible frame's GPU time; false brings the glass back. It's kept for this window, as the palette's \"Turn plain panels on or off\" keeps it. Returns the state once the view has settled: state.look says how the panels are drawn.";

fn instance_arg(installed: bool) -> Z {
    let ids: Vec<&'static str> = if installed { vec!["installed"] } else { INSTANCES.iter().map(|i| i.id).collect() };
    enumeration(&ids)
        .optional()
        .describe("Which XenoNet: installed (the default for installed Tools), demo (made-up data; the default in this checkout), dev, test or film. Stays picked for later calls.")
}

fn settle_arg() -> Z {
    boolean().optional().describe("Wait for the view to settle before returning (default true).")
}

fn point_arg() -> Z {
    union(vec![object(vec![("x", number()), ("y", number())]), object(vec![("node", string())])])
        .describe("A point on the page: { x, y } in CSS pixels from the top left, or { node: id } for a node's centre on screen.")
}

/// The tools, in the JS server's order; installed Tools have no eval or
/// simulate.
fn defs(installed: bool) -> Vec<Def> {
    let inst = || instance_arg(installed);
    let def = |name, title, description: &str, input, annotations: Value| Def { name, title, description: description.to_string(), input, output: None, annotations };
    let read = || json!({ "readOnlyHint": true, "openWorldHint": false });
    let write = || json!({ "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false });
    let idempotent = || json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false });
    let mut d = vec![
        def(
            "terminal",
            "XenoNet terminals",
            "List, open, read, write to or close a terminal in Desktop. write sends the exact text; include a newline to run a command. Installed terminals survive Desktop restarts. read returns the active tab's output.",
            object(vec![
                ("instance", inst()),
                ("action", enumeration(&["list", "open", "read", "write", "close"]).optional()),
                ("id", int().optional()),
                ("text", string().optional()),
                ("cwd", string().optional()),
                ("profile", string().optional()),
                ("lines", int().min(1.0).max(1000.0).optional()),
            ]),
            json!({ "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true }),
        ),
        def(
            "updates",
            "XenoNet updates",
            "Open Updates and read its state, check for a release, or change automatic checks. Development copies never download or install updates.",
            object(vec![
                ("instance", inst()),
                ("action", enumeration(&["status", "check", "automatic", "components", "component-update"]).optional()),
                ("enabled", boolean().optional()),
                ("component", enumeration(&["service", "tools"]).optional()),
            ]),
            json!({ "readOnlyHint": false, "destructiveHint": false, "openWorldHint": true }),
        ),
        def(
            "background",
            "XenoNet background service",
            "Open Background service and read whether XenoNet's service starts at login, show Desktop's setup offer of it if that's due, or answer the offer Not now. Turning it on or off is yours alone, in XenoNet's window or with xenonet service install.",
            object(vec![("instance", inst()), ("action", enumeration(&["status", "offer", "dismiss"]).optional())]),
            write(),
        ),
        def(
            "status",
            "XenoNet status",
            "Which XenoNet instances are running, and for the current one its window and the page's state: the selection, what's gathered, open panels, alerts, counts and whether the view is still moving.",
            object(vec![("instance", inst())]),
            read(),
        ),
        def(
            "launch",
            "Launch XenoNet",
            "Starts installed XenoNet or a development instance unless it's running, waits until its page is ready, and makes it the current one. By default the window shows without taking focus (the test instance stays hidden). A demo or test copy stuck holding the instance's lock without answering is ended first, since its data is made up; `ended` in the result says so.",
            object(vec![
                ("instance", inst()),
                ("show", enumeration(&["inactive", "front", "hidden"]).optional().describe("inactive: shown, not focused (default); front: shown and focused; hidden: never shown, still renders and screenshots.")),
                ("restart", boolean().optional().describe("Quit it first if it's running, for a fresh start. It also ends a copy stuck holding the instance's lock without answering, the dev instance's too.")),
            ]),
            json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }),
        ),
        def(
            "quit",
            "Quit XenoNet",
            "Quits Desktop for the selected instance. Installed XenoNet keeps its service, terminals and coordination running. For development instances it returns once the app has exited, or once it's ended it if it was stuck on its way out (`killed`). The dev instance is the user's own XenoNet Dev, so quit it only when asked.",
            object(vec![("instance", inst())]),
            json!({ "readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": false }),
        ),
        def(
            "find",
            "Find nodes",
            "Finds nodes on the map whose id, title or name contain every word of `query`: projects, sessions, Codex projects (native), folders outside the projects folder (zone), shared systems (resource), the shared context (wiki) and its pages (page), other machines (host: over SSH with XENONET_HOSTS=1, or in a network; their nodes' ids start with @<host>: and say which host), and actors beside the map (kind actor, by id, name and product). Drawn nodes come first, then the most recent, then actors. Each has its id, for select and input, and where it is on screen.",
            object(vec![
                ("query", string().optional().describe("Words to match; empty lists everything.")),
                ("kind", enumeration(&KINDS).optional()),
                ("limit", int().min(1.0).max(200.0).optional().describe("Default 20.")),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "select",
            "Select a node",
            "Selects a node, as clicking it would: a project, Codex project or folder gathers its sessions around it and the camera frames them; an actor (actor:<slug>) is framed with what it acts on; anything else gets the camera's focus and the inspector. Give an id, or a query to select the best match from find. null clears the selection. Returns the state once the view has settled.",
            object(vec![
                ("id", string().nullable().optional().describe("A node id from find or status, or an actor id from actors; null clears the selection.")),
                ("query", string().optional().describe("Instead of an id: select the first node find gives for these words.")),
                ("kind", enumeration(&KINDS).optional().describe("With query: only this kind.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "navigate",
            "Step through the map",
            "back: Esc, from a gathered session to its project, then out (putting the gathering back). next / previous: ↓ / ↑, the next or previous gathered node. reset: R, frame the whole map. Returns the state once the view has settled.",
            object(vec![("action", enumeration(&["back", "next", "previous", "reset"])), ("settle", settle_arg()), ("instance", inst())]),
            write(),
        ),
        def(
            "stats",
            "Open a session's stats",
            "Opens a session's stats panel, as Stats in the inspector or Mod+I in the palette would: what it would cost at API prices, its models over time, where its tokens went, its turns, context and tools. Give a session id, or a query for the best matching session. Returns the state and the numbers in brief (cost, tokens, calls, turns, models).",
            object(vec![
                ("id", string().optional().describe("A session id from find.")),
                ("query", string().optional().describe("Instead of an id: the first session find gives for these words.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "needs_you",
            "Needs you",
            "Reads the Needs you queue: everything waiting on the person, the longest wait first. That is sessions waiting on a question or a permission, actors waiting on them, programs asking to be actors and sessions' asks for an actor's gateway token. Each item has its key, kind (question, permission, actor, request, gateway, or requests: how many programs ask, in a window that can't see who), who it is (title, and the session's app or another machine's host), what it waits for, since when (or until when a request lapses), the actions that answer it and whether its row shows in the open panel. `open` and `close` show or hide the panel. `show` selects an item on the map, `open-app` opens a session in its app (where its question is answered), and `dismiss` puts a session's wait away, as the panel's buttons do; each takes the item's key and returns the state that follows. Allowing or denying a program, and writing to an actor, are the person's, so this tool does neither.",
            object(vec![
                ("action", enumeration(&["read", "open", "close", "show", "open-app", "dismiss"]).optional().describe("read (the default) lists the queue.")),
                ("key", string().optional().describe("For show, open-app and dismiss: an item's key from the queue.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "refusals",
            "Browse provider refusals",
            "Opens the provider refusal browser. Filter the full collection, sort it, or select a record from the current page. Returns result metadata and selected ID. Viewing a saved request is the person's to do, so this tool can't. Browsing does not change recording settings.",
            object(vec![
                ("action", enumeration(&["open", "filter", "select", "page", "refresh"]).optional()),
                (
                    "filters",
                    object(vec![
                        ("q", string().optional()),
                        ("provider", string().optional()),
                        ("kind", string().optional()),
                        ("model", string().optional()),
                        ("category", string().optional()),
                        ("context", enumeration(&["", "with", "without"]).optional()),
                        ("from", string().optional().describe("Local date YYYY-MM-DD, inclusive.")),
                        ("to", string().optional().describe("Local date YYYY-MM-DD, inclusive.")),
                        ("sort", enumeration(&["newest", "oldest", "provider", "model", "category", "kind"]).optional()),
                    ])
                    .optional(),
                ),
                ("id", string().optional()),
                ("page", int().min(1.0).optional()),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "integrations",
            "Review agent workflows",
            "Browse routing decisions, OpenShell jobs and read-only Pi run metadata. Open a tab, filter, page, select a record, open its cached local review or select a reviewed file. Retrieval, export, forgetting, sending decisions, source configuration, launch and deletion stay in the owner's window; this tool never executes them.",
            object(vec![
                ("action", enumeration(&["open", "filter", "select", "page", "refresh", "review", "file"]).optional()),
                ("tab", enumeration(&["routing", "sandbox", "results", "pi"]).optional()),
                ("filters", object(vec![("q", string().optional()), ("status", string().optional()), ("sort", enumeration(&["newest", "oldest"]).optional())]).optional()),
                ("id", string().optional()),
                ("page", int().min(1.0).optional()),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "spend",
            "Read Token use",
            "Reads the Token use panel: about what today's work would cost at API prices, by app (costs) and by actor (byActor, and the rows the panel draws in actors), and the configured OpenRouter key's spend or its visible setup/error state. Each session counts for one actor at most: the one that reported it as a run, or started it, or started the session it came from; the rest is unattributed, including sessions the person starts. A run reported from elsewhere is counted apart, unpriced. Refresh asks for current cached provider usage and the breakdown again; actors opens (more) or folds (fewer) the actors past the first few. It never sends a model request or changes credentials.",
            object(vec![
                ("refresh", boolean().optional()),
                ("actors", enumeration(&["more", "fewer"]).optional().describe("Open or fold the actors past the first few")),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "rename",
            "Rename a session",
            "Renames a session, as typing over its title in the inspector (F2, or the pencil) would. The name is saved in the session's own app, so it shows there too: Claude Code's transcript and, for a session in Claude desktop, the desktop's record (which the desktop reads when it next starts), or Codex's thread name. The demo keeps the names of its made-up sessions in XenoNet. Another machine's sessions can't be renamed. Give a session id, or a query for the best matching session. Returns the state, and `renamed`: the name as saved, where it went and anything to know.",
            object(vec![
                ("id", string().optional().describe("A session id from find.")),
                ("query", string().optional().describe("Instead of an id: the first session find gives for these words.")),
                ("title", string().min(1.0).max(120.0).describe("The new name, on one line.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "command",
            "Run a palette command",
            "Runs a command from the command palette (⌘K / Ctrl+K) by id, such as wallpaper, replay, poster, daylog, away2, reset, scope-7d or focus-lighting, and returns the state once the view has settled. Without an id, lists the commands.",
            object(vec![("id", string().optional()), ("settle", settle_arg()), ("instance", inst())]),
            write(),
        ),
        def(
            "poster",
            "A poster of today",
            "Renders today's constellation as a poster (src/poster.js) at `size`: story (1080×1920), portrait (1080×1350), card (1200×630) or wide (1920×1080). Projects show as star names and no clock appears, unless `names` or `tokens` reveal them for this one. Returns what it drew (`lines`) and the PNG's size; `save` saves it to the pictures folder, as the export panel's Save PNG does. The panel itself, with its preview, opens with command poster.",
            object(vec![
                ("size", enumeration(&["story", "portrait", "card", "wide"]).optional()),
                ("names", boolean().optional()),
                ("tokens", boolean().optional()),
                ("save", boolean().optional()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "labels",
            "The map's labels",
            "Reads every drawn node's label as it reads now (id, kind, text, visible), and whether an export has them redacted: while a film plays, and during a poster render, projects, groups, folders, pages, shared systems and machines show star names and sessions show no label (core/export-labels.js), unless the export reveals the names. For checking what a film or a poster shows of the map.",
            object(vec![("instance", inst())]),
            read(),
        ),
        def(
            "filters",
            "Filters",
            "The toggles under Filters: claude, codex and grok show or hide each app's sessions (Claude Code, Codex, Grok Build), archived shows or hides archived ones, and external the folders outside the projects folder. `set` turns some on or off, as a click on each toggle does ({ grok: false } hides Grok Build's sessions). Returns the toggles and how many sessions of each app the map has and draws, once the view has settled.",
            object(vec![
                (
                    "set",
                    object(vec![
                        ("claude", boolean().optional()),
                        ("codex", boolean().optional()),
                        ("grok", boolean().optional()),
                        ("archived", boolean().optional()),
                        ("external", boolean().optional()),
                    ])
                    .optional()
                    .describe("Toggles by name, on (true) or off (false)."),
                ),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "identities",
            "Identities",
            "The identities sessions ran as and projects belong to (Claude and ChatGPT accounts, GitHub accounts, git identities), as the chips under Filters show them, each with a key. `only` shows just those identities' sessions and projects ([] shows everything again); `masked` hides them as \"GitHub A\", \"Claude B\"… for screenshots, or shows them. Returns the chips, and the state once the view has settled when something changed.",
            object(vec![
                ("only", array(string()).optional().describe("Chip keys from an earlier call; [] clears the filter.")),
                ("masked", boolean().optional()),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "hosts",
            "Other machines",
            "Other machines on the map: other XenoNets over SSH, with XENONET_HOSTS=1 (a prototype: the console reads each with xenonet relay, read-only), and the machines of a network this one is in, drawn from their presence (network: true; their sessions take a message from you). Each host with how it stands (connecting, live, stale, offline or error), its host node (@<host>:core) and where that is on screen, and its constellation beside this machine's map; the bridges between checkouts of one repo; and the machines' chips under Filters. `only` shows just those machines' work: host ids, and \"@here\" for this machine ([] shows every machine). In XenoNet Dev, simulate host-add shows another XenoNet on this machine as a host. Returns the hosts, and the state once the view has settled when something changed.",
            object(vec![
                ("only", array(string()).optional().describe("Host ids, and \"@here\" for this machine; [] clears the filter.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "network",
            "The network",
            "This machine's network across machines, as Network settings in the palette shows it: whether it's in one, its name and this machine; the gateway (address, kind, link, since when, why it's down, clock skew); each machine (online or since when, its agents and what waits on you, founder, gateway, window, and where you're at), with its agents under it; the tickets waiting, what waits for approval (proposals: each with what it does and who signed it), the outbox, a roster mismatch, and the titles-only and share-load settings. `open` shows the panel, `start` and `join` its forms, and `add` opens Add a machine with the new machine's id (host) and whether you'll use a XenoNet window on it (window) filled in. `message` opens the box that messages a session as you (to: its id, or @<host>:<id> for another machine's) with text in it. Making the join line, starting or joining a network, removing or replacing a machine, approving, the setting and Send are the person's: clicks in the window, at the answer's at and message.sendAt, which only the demo and test instances let this console make. The join line shows once, masked: join.masked says whether it is, never the ticket. In XenoNet Dev, simulate net-… makes up a network to look at. Returns the network, and the state once the view has settled when something opened.",
            object(vec![
                ("action", enumeration(&["read", "open", "start", "join", "add", "message", "close"]).optional().describe("read (the default) says how the network stands without opening anything.")),
                ("host", string().optional().describe("For add: the new machine's id.")),
                ("window", boolean().optional().describe("For add: whether you'll use a XenoNet window on it.")),
                ("to", string().optional().describe("For message: a session's id, @<host>:<id> for another machine's, or an actor's.")),
                ("text", string().optional().describe("For message: what goes in the box.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "machines",
            "Machines",
            "The load panel's larger view: each machine's load now (CPU, GPU, memory), what its parts are (CPU model, threads, GHz, memory, each GPU, platform, arch) and its sessions on the map, with the network's totals. On its own that's this computer; in a network, every machine, this one marked here, and other machines' load only on a window host's console. `read` (the default) says how the view stands, and the panel's own reading: the network's sums and each machine's part of each. `open` opens it, at a machine (at: @here or its id). `filter` picks machines (machines: their keys, @here or ids, which are the Filters chips, so the map is filtered too; [] picks all), a platform (darwin, win32, linux), only those with a GPU and only those online, and sorts (name, cpu, gpu, ram, seen) or shows a table (view). `close` closes it. Returns the view's state, and the state once the view has settled when it changed.",
            object(vec![
                ("action", enumeration(&["read", "open", "filter", "close"]).optional().describe("read (the default) changes nothing.")),
                ("at", string().optional().describe("For open: the machine to show first, @here or its id.")),
                ("machines", array(string()).optional().describe("For filter: the machines to show, by key (@here or an id); [] shows all.")),
                ("platform", enumeration(&["darwin", "win32", "linux", ""]).optional().describe("For filter: only this platform; empty for all.")),
                ("gpu", boolean().optional().describe("For filter: only machines with a GPU.")),
                ("online", boolean().optional().describe("For filter: only machines online.")),
                ("sort", enumeration(&["name", "cpu", "gpu", "ram", "seen"]).optional().describe("For filter: the order.")),
                ("view", enumeration(&["cards", "table"]).optional().describe("For filter: cards, or one table row a machine.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "root",
            "The projects folder",
            "The projects folder XenoNet reads (its root) and where that comes from: XENONET_PROJECTS_DIR, a folder chosen in the console, or the default ~/Projects, with its last scan. `check` says what XenoNet would find in another folder (how many projects and groups, or why it can't use it) without switching. `path` switches to a folder, as Change the projects folder… in the palette does: XenoNet checks it, rescans and returns once the map shows it; null goes back to the default. The layout, your rules and Codex's picks stay with each folder, so switching back brings them back. The demo has a second made-up folder, ~/Clients.",
            object(vec![
                ("path", string().nullable().optional().describe("Switch to this folder: a full path, or one starting with ~ (the demo's own home, in the demo). null goes back to the default.")),
                ("check", string().optional().describe("A folder to check without switching to it.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "simulate",
            "Simulate a session state, an actor acting, or another machine",
            "Puts sessions into states that are otherwise rare, so you can check how they look. The kinds are needs-you (a question waiting), collision (two sessions editing the same file, which needs a and b), stuck (a failing loop), wiki-write (a session writing to the shared context), turn-end, doctor (the doctor finding a problem), and clear, which undoes them all. Without session ids it picks the most recently active sessions. The actor-… kinds have an actor act instead, as an event it reports would. They are actor-spawn (it starts a session), actor-edit (it edits files in a project), actor-message (it messages a session), actor-online and actor-offline. The actor reacts, and a beam runs to what it touched. actor-request makes up a program asking to be an actor (named a, Heron by default), which waits for an Allow in the window. Its card shows in the attention strip, and actors lists it under requests. actor-needs-you has an actor (a, the first by default) wait on you with a made-up question. Its card shows in the attention strip, and its glow beats in the attention color until clear. actor-tunnel gives an actor (a) a made-up remote connection through a tunnel with an adapter connected, so the map draws its gate open between it and what it reads. actor-tunnel-down does the same with no adapter connected, so the gate is shut. actor-read sends a read through it, of node b or what it acts on, with light down the conduit and out to what it read. clear takes the tunnel away. Only the demo and test instances, whose data is made up, answer actor-request, actor-needs-you and the tunnel kinds, and actor-request's token stays out of this answer. With XENONET_HOSTS=1, it covers other machines too. host-add shows another XenoNet on this machine as a host (a is the new host's id, b that XenoNet's port). host-offline and host-online take a host's id as a. host-skew sets a host's clock b ms off, three minutes if b is left out. The net-… kinds make up a network across machines, with other machines' agents, for the Network panel (network), the attention strip and the message box: net-none (this machine on its own), net-founder (this machine started it and is its gateway; two more machines, one offline; on laptop a session waits on you with a question and another seems stuck), net-member (this machine joined; the gateway is another), net-gateway-down (the gateway hasn't answered for 25 minutes, and what's for other machines waits in the outbox), net-ticket (a ticket waits), net-mismatch (a machine sees another list of the network's machines), net-remote-waiting (an agent on another machine waits on you) and net-approval (a window host that isn't the founder signed another in as a window host, which waits for an approval here). For net-founder and net-remote-waiting, a is the machine you're at, this one by default: what waits sounds only there, here or on another machine; and b titles-only has the other machines share titles only, so no question's text, last message or stuck reason comes from them. clear ends it. Everything stays in memory, apart from actor-request. Returns what was used and the page's state after the update arrives.",
            object(vec![
                ("kind", enumeration(&SIMULATIONS)),
                ("a", string().optional().describe("A session id (find with kind session); for the actor-… kinds, an actor id (actors lists them, and the first one that isn't you is the default); for actor-request, the name it asks with; for host-…, a host's id; for net-founder and net-remote-waiting, the machine you're at.")),
                ("b", string().optional().describe("The second session, for collision; for the actor-… kinds, the node it acts on (a session for actor-spawn and actor-message, a project for actor-edit; the session busy most lately when left out); for host-add, the port of the XenoNet to show; for host-skew, milliseconds; for net-founder and net-remote-waiting, titles-only.")),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "actors",
            "Actors",
            "The actors beside the map. They are the person at the console (You), always-on agents and actors of your own, drawn on a layer of their own above the map, with beams into the projects and sessions they act on. Each comes with its id, presence (working, idle, away, offline, unknown), status, how XenoNet knows of it (via), its product, its form and color in the theme, the node ids it beams into now and where it is on screen. The answer also has every beam drawn and the layer's time for its last frame. `requests` lists the programs waiting for an Allow in the window, each with what it asks, who asked, and whether its card is up. `gatewayAsks` lists sessions' asks for an actor's gateway token, each with the actor, who asked, whether it replaces a token, and whether its card is up. It never holds a token, since none exists until the person allows the ask, and no tool here allows one. `tokens` says how each registered actor's token stands, as the Actors panel shows it (works, expired or none, when it expires, and when it was last used, null if never), and never holds a token itself. `remote` lists remote actors' connections (a dot reading through a tunnel, say) and whether cloud access is on. For each connection it gives the actor, state, how many projects it reads, whether labels go, its data mode, expiry and whether an adapter is connected, and never a credential. Its `tunnels` says whether tunnel-client is here, and for each tunnel XenoNet can run, its tunnel id, whether a key is kept (never the key), whether it starts with XenoNet, and whether it runs and why not. The `tunnels` beside the beams give each tunnel's gate on the map, how open it is and where. Allowing an actor, pairing, revoking or turning on a remote connection, and keeping a tunnel's key or starting it, are the person's alone, so no tool here does any of it. The command actors opens the Actors panel for them. `select` selects an actor, as clicking it would. The camera frames it with what it acts on and the inspector shows it (null clears the selection), and the answer then includes the state once the view has settled.",
            object(vec![
                ("select", string().nullable().optional().describe("An actor id (actor:<slug>) to select; null clears the selection.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "feed",
            "Activity feed",
            "The activity feed as it shows now, newest first: each line's kind (for a session tool, prompt, message, agent-message, turn-start, turn-end, needs-you, stuck and the like; for an actor what it did: presence, status, spawned, edited, ran, messaged, said, finished, failed, or agent-message), who it's about (session: a session's id, or actor: an actor's), its title (a session's title, or an actor's name) and its words, and when.",
            object(vec![("limit", int().min(1.0).max(60.0).optional().describe("Default 20.")), ("instance", inst())]),
            read(),
        ),
        def(
            "screenshot",
            "Screenshot XenoNet",
            "The window as it looks now, returned as an image, with a full-size PNG saved in the repo's .xenonet folder. It works when the window is covered, unfocused or hidden. By default it waits for the view to settle and holds every ambient animation still (sky, sonar, sweep, sparks, CSS animation), so two shots of the same state match; still: false shows them as they are.",
            object(vec![
                ("name", string().optional().describe("The saved file's name, without .png (default shot).")),
                ("still", boolean().optional().describe("Hold ambient animation still for the shot (default true).")),
                ("settle", settle_arg()),
                ("maxWidth", int().min(200.0).max(4000.0).optional().describe("Width of the returned image in pixels (default 1568); the saved PNG is full size.")),
                ("format", enumeration(&["jpeg", "png"]).optional().describe("Of the returned image (default jpeg).")),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "input",
            "Keyboard and mouse",
            "Real keyboard and mouse input to the page, which needn't have focus. Any of, in this order: keys (combos like \"Mod+K\", \"Escape\", \"ArrowDown\", \"W\"; Mod is ⌘ on a Mac, Ctrl elsewhere), text (typed into whatever has focus), move (hover), click, drag (Alt-drag moves a node), scroll. Points are { x, y } or { node: id }. Prefer select, navigate and command for what they cover; use input to test the real interaction. Returns what it did and the state once the view has settled.",
            object(vec![
                ("keys", union(vec![string(), array(string())]).optional()),
                ("text", string().optional()),
                ("move", point_arg().optional()),
                ("click", point_arg().optional()),
                ("count", int().min(1.0).max(3.0).optional().describe("Clicks: 2 double-clicks (on a session, the transcript preview).")),
                ("button", enumeration(&["left", "right", "middle"]).optional()),
                ("drag", object(vec![("from", point_arg()), ("to", point_arg())]).optional()),
                (
                    "scroll",
                    object(vec![("x", number()), ("y", number()), ("dx", number().optional()), ("dy", number().optional())])
                        .optional()
                        .describe("A wheel turn at x, y; dy > 0 scrolls down (zooms the map out)."),
                ),
                ("modifiers", array(enumeration(&["Alt", "Control", "Meta", "Shift", "Mod"])).optional().describe("Held for move, click, drag and scroll.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "window",
            "XenoNet's window",
            "Shows the window (\"inactive\": without taking focus; \"front\": focused), hides it, minimizes it (show brings it back), or sets the page's size, such as \"1600x900\", or \"1600x900@2\" for a pixel ratio of 2 (\"window\" goes back to the window's own size).",
            object(vec![
                ("show", enumeration(&["inactive", "front"]).optional()),
                ("hide", boolean().optional()),
                ("minimize", boolean().optional()),
                ("size", string().optional()),
                ("instance", inst()),
            ]),
            idempotent(),
        ),
        def(
            "logs",
            "XenoNet's logs",
            "Recent page console messages, uncaught page errors, app events (loads, crashes) and server output, oldest first. Pass the returned cursor as since to get only what's new.",
            object(vec![
                ("level", enumeration(&["error", "warning", "all"]).optional().describe("error, warning (and errors), or all (default).")),
                ("source", enumeration(&["page", "server", "app"]).optional()),
                ("since", int().optional()),
                ("limit", int().min(1.0).max(600.0).optional().describe("Default 200.")),
                ("instance", inst()),
            ]),
            read(),
        ),
        def(
            "eval",
            "Run JavaScript in the page",
            "Evaluates JavaScript in XenoNet Dev's page and returns the result as JSON: an expression, or a function body with return; await works. window.__xenonet holds the scene, panels and the driver (window.__xenonet.driver). For checks the other tools don't cover.",
            object(vec![("expression", string()), ("instance", inst())]),
            write(),
        ),
        def(
            "theme",
            "Themes",
            "The console's themes, switched live without a reload. list: the themes (id, name, built in or the user's) and the one in use. set: use one in every window, as Theme: in the palette does. preview: show one in this window alone, to look at; keep makes it the one in use, revert goes back to it. switcher: open \"Theme…\", the list a person steps through; step: ↓ or ↑ in it (by 1 or -1), showing the theme it lands on in this window alone, then keep (Enter) or revert (Esc). gallery: open the gallery of theme cards (with id, looking at that one); focus: look at a card's theme, as pointing at it does; make: \"Make your own\" from the card looked at, a copy saved as a file of the user's and put in use (state.made has its file); close: close the switcher or the gallery, putting the theme in use back. Except for list, waits until the theme is drawn and the view has settled, and returns the state (state.theme says which is drawn and which is in use, state.themes what the switcher and the gallery show).",
            object(vec![
                ("action", enumeration(&["list", "set", "preview", "keep", "revert", "switcher", "step", "gallery", "focus", "make", "close"]).optional().describe("Default list.")),
                ("id", string().optional().describe("For set, preview and focus: a theme's id from list; for gallery, the card to look at first.")),
                ("by", int().optional().describe("For step: 1 (↓, the default) or -1 (↑), or further.")),
                ("settle", settle_arg()),
                ("instance", inst()),
            ]),
            write(),
        ),
        def(
            "mcp_info",
            "Inspect an MCP shared system",
            "Open an MCP shared system's rich inspector, with declared author, client connection and authentication status, searchable tool descriptions and input schemas, resource and template context, and recent usage. This does not execute a server tool or read resource contents. After open, tab selects overview, tools, context or activity; search filters tools.",
            object(vec![
                ("action", enumeration(&["open", "tab", "search"]).optional()),
                ("id", string().optional().describe("For open: a shared system's node id from find.")),
                ("tab", enumeration(&["overview", "tools", "context", "activity"]).optional()),
                ("query", string().optional()),
                ("instance", inst()),
                ("settle", settle_arg()),
            ]),
            json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false }),
        ),
        def(
            "environment",
            "Choose the environment around the map",
            "List or preview Nebula, Aurora, Trench and Vault independently of the active theme. All inherit the theme's colors. set saves the choice for every window; preview changes only this window; keep saves the preview; revert cancels it; picker opens the person's environment cards. No reload is needed.",
            object(vec![
                ("action", enumeration(&["list", "set", "preview", "keep", "revert", "picker"]).optional()),
                ("id", string().optional()),
                ("instance", inst()),
                ("settle", settle_arg()),
            ]),
            write(),
        ),
        def(
            "look",
            "How the console draws its panels",
            LOOK,
            object(vec![
                ("plainPanels", boolean().optional().describe("true: solid panels, without the glass blur; false: glass panels, the default. Leave it out to only read.")),
                ("instance", inst()),
                ("settle", settle_arg()),
            ]),
            idempotent(),
        ),
    ];
    if installed {
        d.retain(|t| !matches!(t.name, "eval" | "simulate"));
    }
    d
}

/// A tool's result: its JSON as text, which every client reads, and any
/// images after it.
fn reply(value: &Value, extra: Vec<ContentBlock>) -> CallToolResult {
    let mut content = vec![ContentBlock::text(xenonet_base::json::stringify(value, 1))];
    content.extend(extra);
    let mut r = CallToolResult::success(content);
    r.is_error = None;
    r
}

/// `{ settle }`, as the JS passes it: an empty object when it wasn't given.
fn settle(a: &Value) -> Value {
    Value::Object(tools::pick(a, &[("settle", "settle")]))
}

fn arg(a: &Value, k: &str) -> Value {
    a.get(k).cloned().unwrap_or(Value::Null)
}

fn text_of(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

// The spread of an object into another, as `{ ...value, … }` has it.
fn spread(into: &mut Map<String, Value>, value: &Value) {
    if let Some(o) = value.as_object() {
        into.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
}

struct Console {
    defs: Vec<Def>,
    instructions: &'static str,
    control: Arc<dyn Control>,
    current: Mutex<String>,
}

impl Console {
    /// `target(instance)`: the instance a call names, which stays picked,
    /// or the one picked last.
    fn target(&self, a: &Value) -> String {
        let mut current = self.current.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = a.get("instance").and_then(Value::as_str) {
            *current = id.to_string();
        }
        current.clone()
    }

    async fn call(&self, instance: &str, kind: &str, args: Value) -> Result<Value, String> {
        let (control, instance, kind) = (self.control.clone(), instance.to_string(), kind.to_string());
        tokio::task::spawn_blocking(move || control.method(&instance, &kind, args)).await.map_err(|e| e.to_string())?
    }

    /// `s.driver(method, ...args)`: what's undefined in args goes as null,
    /// as JSON.stringify writes an array.
    async fn driver(&self, instance: &str, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.call(instance, "driver", json!({ "method": method, "args": args })).await
    }

    /// `(await s.driver("find", { query, kind, limit: 1 })).nodes[0]?.id`.
    async fn first(&self, s: &str, query: &str, kind: Option<Value>) -> Result<Option<Value>, String> {
        let mut f = Map::new();
        f.insert("query".into(), json!(query));
        if let Some(k) = kind {
            f.insert("kind".into(), k);
        }
        f.insert("limit".into(), json!(1));
        let found = self.driver(s, "find", vec![Value::Object(f)]).await?;
        Ok(found.get("nodes").and_then(Value::as_array).and_then(|n| n.first()).map(|n| arg(n, "id")))
    }

    async fn run(&self, name: &str, a: &Value) -> Result<CallToolResult, String> {
        let s = self.target(a);
        let one = |v: Value| Ok(reply(&v, vec![]));
        match name {
            "terminal" => {
                let options = Value::Object(tools::pick(a, &[("id", "id"), ("text", "text"), ("cwd", "cwd"), ("profile", "profile"), ("lines", "lines")]));
                one(self.driver(&s, "terminal", vec![a.get("action").cloned().unwrap_or_else(|| json!("list")), options]).await?)
            }
            "updates" => {
                let action = a.get("action").cloned().unwrap_or_else(|| json!("status"));
                let second = if action == "component-update" { arg(a, "component") } else { arg(a, "enabled") };
                one(self.driver(&s, "updates", vec![action, second]).await?)
            }
            "background" => one(self.driver(&s, "background", vec![a.get("action").cloned().unwrap_or_else(|| json!("status"))]).await?),
            "status" => {
                let mut instances = Map::new();
                for i in &INSTANCES {
                    let p = self.call(i.id, "ping", json!({})).await?;
                    let entry = if truthy(Some(&p)) {
                        let mut e = Map::new();
                        e.insert("running".into(), json!(true));
                        e.insert("port".into(), arg(&p, "port"));
                        e.insert("window".into(), arg(&p, "window"));
                        e.insert("ready".into(), json!(truthy(p.get("page").and_then(|g| g.get("ready")))));
                        if let Some(err) = p.get("error").filter(|e| truthy(Some(e))) {
                            e.insert("error".into(), err.clone());
                        }
                        Value::Object(e)
                    } else {
                        json!({ "running": false, "port": i.port })
                    };
                    instances.insert(i.id.to_string(), entry);
                }
                let ready = instances.get(&s).and_then(|e| e.get("ready")).is_some_and(|r| r == &json!(true));
                let state = if ready { self.driver(&s, "state", vec![]).await? } else { Value::Null };
                one(json!({ "current": s, "instances": instances, "state": state }))
            }
            "launch" => {
                let ping = self.call(&s, "launch", Value::Object(tools::pick(a, &[("show", "show"), ("restart", "restart")]))).await?;
                let mut out = Map::new();
                spread(&mut out, &ping);
                out.insert("state".into(), self.driver(&s, "state", vec![]).await?);
                one(Value::Object(out))
            }
            "quit" => one(self.call(&s, "quit", json!({})).await?),
            "find" => one(self.driver(&s, "find", vec![Value::Object(tools::pick(a, &[("query", "query"), ("kind", "kind"), ("limit", "limit")]))]).await?),
            "select" => {
                let mut id = a.get("id").cloned();
                if id.is_none() {
                    if let Some(q) = text_of(a, "query") {
                        id = Some(self.first(&s, &q, a.get("kind").cloned()).await?.ok_or_else(|| format!("nothing on the map matches \"{q}\""))?);
                    }
                }
                one(self.driver(&s, "select", vec![id.unwrap_or(Value::Null), settle(a)]).await?)
            }
            "navigate" => {
                let r = match a.get("action").and_then(Value::as_str) {
                    Some("back") => self.driver(&s, "back", vec![settle(a)]).await?,
                    Some("reset") => self.driver(&s, "reset", vec![settle(a)]).await?,
                    action => self.driver(&s, "step", vec![json!(if action == Some("next") { 1 } else { -1 }), settle(a)]).await?,
                };
                one(r)
            }
            "stats" | "rename" => {
                let query = text_of(a, "query");
                let mut id = text_of(a, "id").map(Value::String);
                if id.is_none() {
                    if let Some(q) = &query {
                        id = self.first(&s, q, Some(json!("session"))).await?.filter(|v| truthy(Some(v)));
                    }
                }
                let Some(id) = id else {
                    return Err(query.map_or_else(|| "give a session id or a query".to_string(), |q| format!("no session matches \"{q}\"")));
                };
                let args = if name == "stats" { vec![id, settle(a)] } else { vec![id, arg(a, "title"), settle(a)] };
                one(self.driver(&s, name, args).await?)
            }
            "needs_you" => one(self.driver(&s, "needsYou", vec![a.get("action").cloned().unwrap_or_else(|| json!("read")), arg(a, "key"), settle(a)]).await?),
            "refusals" => {
                let action = a.get("action").and_then(Value::as_str).unwrap_or("open");
                let second = match action {
                    "filter" => arg(a, "filters"),
                    "select" => arg(a, "id"),
                    _ => arg(a, "page"),
                };
                one(self.driver(&s, "refusals", vec![json!(action), second, settle(a)]).await?)
            }
            "integrations" => {
                let action = a.get("action").and_then(Value::as_str).unwrap_or("open");
                let second = match action {
                    "open" => arg(a, "tab"),
                    "filter" => arg(a, "filters"),
                    "select" | "file" => arg(a, "id"),
                    _ => arg(a, "page"),
                };
                one(self.driver(&s, "integrations", vec![json!(action), second, settle(a)]).await?)
            }
            "spend" => {
                let actors = a.get("actors").cloned();
                if a.get("refresh") == Some(&Value::Bool(true)) {
                    let read = self.driver(&s, "spend", vec![json!("refresh")]).await?;
                    if actors.is_none() {
                        return one(read);
                    }
                }
                one(self.driver(&s, "spend", vec![actors.unwrap_or_else(|| json!("read"))]).await?)
            }
            "command" => match text_of(a, "id") {
                Some(id) => one(self.driver(&s, "command", vec![json!(id), settle(a)]).await?),
                None => one(json!({ "commands": self.driver(&s, "commands", vec![]).await? })),
            },
            "poster" => {
                let mut o = tools::pick(a, &[("size", "size")]);
                o.insert("reveal".into(), Value::Object(tools::pick(a, &[("names", "names"), ("tokens", "tokens")])));
                o.extend(tools::pick(a, &[("save", "save")]));
                one(self.driver(&s, "poster", vec![Value::Object(o)]).await?)
            }
            "labels" => one(self.driver(&s, "labels", vec![]).await?),
            "filters" => {
                let set = a.get("set").filter(|v| truthy(Some(v)));
                let state = match set {
                    Some(set) => self.driver(&s, "filters", vec![set.clone(), settle(a)]).await?,
                    None => self.driver(&s, "state", vec![]).await?,
                };
                let mut out = Map::new();
                out.insert("filters".into(), state.get("filters").cloned().unwrap_or(Value::Null));
                if let Some(apps) = state.get("counts").and_then(|c| c.get("apps")) {
                    out.insert("apps".into(), apps.clone());
                }
                if set.is_some() {
                    out.insert("state".into(), state);
                }
                one(Value::Object(out))
            }
            "identities" => {
                let mut state = Value::Null;
                if let Some(masked) = a.get("masked") {
                    state = self.driver(&s, "maskIdentities", vec![masked.clone(), settle(a)]).await?;
                }
                if let Some(only) = a.get("only") {
                    state = self.driver(&s, "filterIdentities", vec![only.clone(), settle(a)]).await?;
                }
                let mut out = Map::new();
                out.insert("identities".into(), self.driver(&s, "identities", vec![]).await?);
                if truthy(Some(&state)) {
                    out.insert("state".into(), state);
                }
                one(Value::Object(out))
            }
            "hosts" => {
                let state = match a.get("only") {
                    Some(only) => self.driver(&s, "filterHosts", vec![only.clone(), settle(a)]).await?,
                    None => Value::Null,
                };
                let mut out = Map::new();
                out.insert("hosts".into(), self.driver(&s, "hosts", vec![]).await?);
                if truthy(Some(&state)) {
                    out.insert("state".into(), state);
                }
                one(Value::Object(out))
            }
            "network" => match a.get("action").and_then(Value::as_str).unwrap_or("read") {
                "message" => one(self.driver(&s, "messageAs", vec![arg(a, "to"), arg(a, "text"), settle(a)]).await?),
                "add" => one(self.driver(&s, "network", vec![json!("add"), Value::Object(tools::pick(a, &[("host", "host"), ("window", "window")])), settle(a)]).await?),
                action => one(self.driver(&s, "network", vec![json!(action), Value::Null, settle(a)]).await?),
            },
            "machines" => match a.get("action").and_then(Value::as_str).unwrap_or("read") {
                "read" => {
                    let mut out = Map::new();
                    spread(&mut out, &self.driver(&s, "machines", vec![json!({})]).await?);
                    out.insert("system".into(), self.driver(&s, "system", vec![]).await?);
                    one(Value::Object(out))
                }
                "open" => one(self.driver(&s, "machines", vec![json!({ "open": true, "at": arg(a, "at") }), settle(a)]).await?),
                "close" => one(self.driver(&s, "machines", vec![json!({ "close": true }), settle(a)]).await?),
                _ => {
                    let filter = tools::pick(a, &[("machines", "machines"), ("platform", "platform"), ("gpu", "gpu"), ("online", "online")]);
                    let mut o = Map::new();
                    if !filter.is_empty() {
                        o.insert("filter".into(), Value::Object(filter));
                    }
                    for k in ["sort", "view"] {
                        if let Some(v) = a.get(k).filter(|v| truthy(Some(v))) {
                            o.insert(k.into(), v.clone());
                        }
                    }
                    one(self.driver(&s, "machines", vec![Value::Object(o), settle(a)]).await?)
                }
            },
            "root" => {
                if let Some(check) = a.get("check") {
                    return one(json!({ "check": self.driver(&s, "checkRoot", vec![check.clone()]).await? }));
                }
                if let Some(folder) = a.get("path") {
                    return one(self.driver(&s, "setRoot", vec![folder.clone(), settle(a)]).await?);
                }
                one(self.driver(&s, "root", vec![]).await?)
            }
            "simulate" => {
                let mut result = self.call(&s, "simulate", Value::Object(tools::pick(a, &[("kind", "kind"), ("a", "a"), ("b", "b")]))).await?;
                // A made-up registration's token is still a token: it stays here.
                if let Some(o) = result.as_object_mut() {
                    o.remove("token");
                }
                // The page hears about it over its event stream a moment later.
                tokio::time::sleep(Duration::from_millis(400)).await;
                let mut out = Map::new();
                spread(&mut out, &result);
                out.insert("state".into(), self.driver(&s, "state", vec![]).await?);
                one(Value::Object(out))
            }
            "actors" => {
                let state = match a.get("select") {
                    Some(select) => self.driver(&s, "select", vec![select.clone(), settle(a)]).await?,
                    None => Value::Null,
                };
                let mut out = Map::new();
                spread(&mut out, &self.driver(&s, "actors", vec![]).await?);
                out.insert("requests".into(), self.driver(&s, "actorRequests", vec![]).await?);
                out.insert("tokens".into(), self.driver(&s, "actorTokens", vec![]).await?);
                out.insert("remote".into(), self.driver(&s, "remoteConnections", vec![]).await.unwrap_or(Value::Null));
                if truthy(Some(&state)) {
                    out.insert("state".into(), state);
                }
                one(Value::Object(out))
            }
            "feed" => one(self.driver(&s, "feed", vec![Value::Object(tools::pick(a, &[("limit", "limit")]))]).await?),
            "screenshot" => {
                let opts = tools::pick(a, &[("name", "name"), ("still", "still"), ("settle", "settle"), ("maxWidth", "maxWidth"), ("format", "format")]);
                let mut shot = self.call(&s, "screenshot", json!({ "opts": opts })).await?;
                let image = shot.as_object_mut().and_then(|o| o.remove("image"));
                let mut out = Map::new();
                out.insert("instance".into(), json!(s));
                spread(&mut out, &shot);
                let extra = match image.filter(|i| truthy(Some(i))) {
                    Some(i) => vec![ContentBlock::image(xenonet_base::text::js_string(&arg(&i, "data")), xenonet_base::text::js_string(&arg(&i, "mimeType")))],
                    None => vec![],
                };
                Ok(reply(&Value::Object(out), extra))
            }
            "input" => {
                let mut opts = a.as_object().cloned().unwrap_or_default();
                opts.remove("instance");
                one(self.call(&s, "input", json!({ "opts": opts })).await?)
            }
            "window" => {
                let mut opts = a.as_object().cloned().unwrap_or_default();
                opts.remove("instance");
                one(self.call(&s, "window", json!({ "opts": opts })).await?)
            }
            "logs" => {
                let mut opts = tools::pick(a, &[("source", "source"), ("since", "since"), ("limit", "limit")]);
                if let Some(level) = a.get("level").filter(|l| l.as_str() != Some("all")) {
                    opts.insert("level".into(), level.clone());
                }
                one(self.call(&s, "logs", json!({ "opts": opts })).await?)
            }
            "eval" => one(json!({ "value": self.call(&s, "eval", json!({ "expression": arg(a, "expression") })).await? })),
            "theme" => {
                let action = a.get("action").and_then(Value::as_str).unwrap_or("list");
                let id = text_of(a, "id");
                let opts = settle(a);
                if action == "list" {
                    return one(self.driver(&s, "themes", vec![]).await?);
                }
                if id.is_none() && matches!(action, "set" | "preview" | "focus") {
                    return Err(format!("say which theme to {action}: an id from list"));
                }
                let r = match action {
                    "switcher" => self.driver(&s, "themeSwitcher", vec![json!(true), opts]).await?,
                    "step" => self.driver(&s, "stepTheme", vec![a.get("by").cloned().unwrap_or_else(|| json!(1)), opts]).await?,
                    "gallery" => self.driver(&s, "themeGallery", vec![a.get("id").cloned().unwrap_or(json!(true)), opts]).await?,
                    "focus" => self.driver(&s, "focusTheme", vec![arg(a, "id"), opts]).await?,
                    "make" => self.driver(&s, "makeTheme", vec![opts]).await?,
                    "close" => {
                        let state = self.driver(&s, "state", vec![]).await?;
                        let open = |k: &str| truthy(state.get("overlays").and_then(|o| o.get(k)));
                        if open("switcher") {
                            self.driver(&s, "themeSwitcher", vec![json!(false), opts]).await?
                        } else if open("gallery") {
                            self.driver(&s, "themeGallery", vec![json!(false), opts]).await?
                        } else {
                            self.driver(&s, "state", vec![]).await?
                        }
                    }
                    _ => {
                        let method = match action {
                            "set" => "theme",
                            "preview" => "previewTheme",
                            "keep" => "keepTheme",
                            _ => "revertTheme",
                        };
                        let mut args = vec![];
                        if let (Some(id), "set" | "preview") = (&id, action) {
                            args.push(json!(id));
                        }
                        args.push(opts);
                        self.driver(&s, method, args).await?
                    }
                };
                one(r)
            }
            "mcp_info" => {
                let action = a.get("action").and_then(Value::as_str).unwrap_or("open");
                if action == "open" {
                    let Some(id) = text_of(a, "id") else { return Err("choose a shared system to inspect".into()) };
                    return one(self.driver(&s, "mcp", vec![json!(id), settle(a)]).await?);
                }
                let r = if action == "tab" {
                    self.driver(&s, "mcpTab", vec![a.get("tab").cloned().unwrap_or_else(|| json!("overview")), settle(a)]).await?
                } else {
                    self.driver(&s, "mcpSearch", vec![a.get("query").cloned().unwrap_or_else(|| json!("")), settle(a)]).await?
                };
                one(r)
            }
            "environment" => {
                let action = a.get("action").and_then(Value::as_str).unwrap_or("list");
                if action == "list" {
                    return one(self.driver(&s, "environments", vec![]).await?);
                }
                let id = text_of(a, "id");
                if id.is_none() && matches!(action, "set" | "preview") {
                    return Err("choose an environment from list".into());
                }
                let method = match action {
                    "set" => "environment",
                    "preview" => "previewEnvironment",
                    "keep" => "keepEnvironment",
                    "revert" => "revertEnvironment",
                    _ => "environmentPicker",
                };
                let mut args = vec![];
                if let (Some(id), "set" | "preview") = (&id, action) {
                    args.push(json!(id));
                }
                args.push(settle(a));
                one(self.driver(&s, method, args).await?)
            }
            "look" => match a.get("plainPanels").and_then(Value::as_bool) {
                Some(on) => one(self.driver(&s, "plainPanels", vec![json!(on), settle(a)]).await?),
                None => one(self.driver(&s, "state", vec![]).await?),
            },
            _ => Err(format!("Tool {name} not found")),
        }
    }
}

impl ServerHandler for Console {
    fn get_info(&self) -> ServerConfig {
        let caps = ServerCapabilities::builder().enable_tools().enable_tool_list_changed().build();
        ServerConfig::new(caps).with_server_info(Implementation::new(NAME, tools::VERSION)).with_instructions(self.instructions.to_string())
    }

    // Every version rmcp knows, so the 2026-07-28 probe reaches discover().
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS)
    }

    fn discover(&self, _context: RequestContext<RoleServer>) -> impl Future<Output = Result<DiscoverResult, McpError>> + Send + '_ {
        std::future::ready(Ok(DiscoverResult::from_server_info(vec![ProtocolVersion::V_2026_07_28], self.get_info())))
    }

    fn list_tools(&self, _request: Option<PaginatedRequestParams>, context: RequestContext<RoleServer>) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(tools::list(&self.defs, &context, NAME)))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let (def, a) = match tools::check(&self.defs, &request.name, request.arguments) {
            Ok(found) => found,
            Err(answer) => return answer.map(|r| tools::answer(r, &context, NAME)),
        };
        let r = match self.run(def.name, &a).await {
            Ok(r) => r,
            Err(message) => tools::failed(message),
        };
        Ok(tools::answer(r, &context, NAME))
    }
}

/// Serves the console operator on stdio until the client goes, its tools
/// answered through `control`; `installed` for installed Tools'
/// (package.json's xenonetComponent), which operate installed XenoNet only.
pub async fn serve(control: Arc<dyn Control>, installed: bool) -> Result<(), String> {
    let picked = std::env::var("XENONET_INSTANCE").ok().filter(|id| INSTANCES.iter().any(|i| i.id == id));
    let current = picked.unwrap_or_else(|| (if installed { "installed" } else { "demo" }).to_string());
    let server = Console { defs: defs(installed), instructions: if installed { INSTRUCTIONS_TOOLS } else { INSTRUCTIONS }, control, current: Mutex::new(current) };
    let running = match server.serve(rmcp::transport::stdio()).await {
        Ok(r) => r,
        // A client that left before the handshake: nothing to say.
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let _ = running.waiting().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_js_servers_tools_in_order() {
        let names: Vec<&str> = defs(false).iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            [
                "terminal",
                "updates",
                "background",
                "status",
                "launch",
                "quit",
                "find",
                "select",
                "navigate",
                "stats",
                "needs_you",
                "refusals",
                "integrations",
                "spend",
                "rename",
                "command",
                "poster",
                "labels",
                "filters",
                "identities",
                "hosts",
                "network",
                "machines",
                "root",
                "simulate",
                "actors",
                "feed",
                "screenshot",
                "input",
                "window",
                "logs",
                "eval",
                "theme",
                "mcp_info",
                "environment",
                "look"
            ]
        );
        let installed: Vec<&str> = defs(true).iter().map(|d| d.name).collect();
        assert!(!installed.contains(&"eval") && !installed.contains(&"simulate") && installed.len() == 34);
    }
}
