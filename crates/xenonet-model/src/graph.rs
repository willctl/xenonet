//! The map (`graph/v1`): its nodes, the tree and the ties between them,
//! the identities work ran as, and where it all came from.

use crate::identity::Identity;
use crate::live::NameSource;
use crate::num::{Ms, Uint};
use crate::project::Project;
use crate::schema::{Any, Document, Nullable, Opt, Ref, True};
use crate::session::Session;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

// What a node's action does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ActionId {
    OpenApp,
    Preview,
    Stats,
    Handoff,
    OpenFolder,
    OpenFile,
    OpenVscode,
    NewClaude,
}

// Something the page offers to do with a node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    /// The page handles it itself, rather than asking the server to open something
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub client: Opt<True>,
}

// Where a node sits on the map, and what can be done with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Place {
    pub fx: Nullable<f64>,
    pub fy: Nullable<f64>,
    pub fz: Nullable<f64>,
    pub pinned: bool,
    pub actions: Vec<Action>,
}

// How many sessions a node holds, and when the last was.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub session_count: Uint,
    pub active_count: Uint,
    pub last_activity: Nullable<Ms>,
    pub last_session_at: Nullable<Ms>,
}

// How sure XenoNet is of who ran something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Yours,
    Exact,
    Likely,
    Assumed,
}

// Who a session ran as.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RanAs {
    pub id: String,
    /// yours: you pinned it; exact: the app recorded it; likely: signed in at the time; assumed: the only account XenoNet has seen
    pub confidence: Confidence,
    /// How XenoNet knows, in words
    pub via: String,
}

// How a session is doing, by its last write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionActivity {
    Active,
    Recent,
    Idle,
}

crate::tag!(pub SessionKind = "session");

// What the map adds to a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlace {
    pub kind: SessionKind,
    /// By its last write; an archived session is at most recent
    pub activity: SessionActivity,
    /// Who it ran as; left out when identities are off
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran_as: Option<RanAs>,
    #[serde(flatten)]
    pub place: Place,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

// A document extended with more fields, written as `allOf` of the
// document's `$ref` and the extra object: a node that is a session or a
// project with its place on the map added.
fn extended<B: Document, M: JsonSchema>(g: &mut SchemaGenerator) -> Schema {
    json_schema!({ "allOf": [g.subschema_for::<Ref<B>>(), g.subschema_for::<M>()] })
}

/// A session where it lives on the map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionNode {
    #[serde(flatten)]
    pub session: Ref<Session>,
    #[serde(flatten)]
    pub map: SessionPlace,
}

impl JsonSchema for SessionNode {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SessionNode")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        extended::<Session, SessionPlace>(g)
    }
}

// The state of a project's checkout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub branch: Nullable<String>,
    pub ahead: Uint,
    pub behind: Uint,
    pub staged: Uint,
    pub modified: Uint,
    pub untracked: Uint,
    /// Lines git status reports: files changed, staged or new
    pub dirty: Uint,
    pub checked_at: Ms,
}

// What an identity is to a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityRole {
    Owner,
    PushesAs,
    CommitsAs,
}

// An identity a project's work ran as.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectIdentity {
    pub id: String,
    pub role: IdentityRole,
    pub confidence: Confidence,
    pub via: String,
}

crate::tag!(pub ProjectKind = "project");

// What the map adds to a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPlace {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?project:"))]
    pub id: String,
    pub kind: ProjectKind,
    pub group: bool,
    /// The key of the project it orbits: its repo for a worktree, or the project it's named after
    pub satellite_of: Nullable<String>,
    pub repo: Nullable<Repo>,
    /// Left out when identities are off
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub identities: Opt<Vec<ProjectIdentity>>,
    /// Its remote as its own host resolves it (repoKey in core/schema/ids.js): checkouts of one repo on different machines share it. With hosts on
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub repo_key: Opt<String>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    pub active: bool,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

/// A project or group on the map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectNode {
    #[serde(flatten)]
    pub project: Ref<Project>,
    #[serde(flatten)]
    pub map: ProjectPlace,
}

impl JsonSchema for ProjectNode {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ProjectNode")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        extended::<Project, ProjectPlace>(g)
    }
}

crate::tag!(pub CoreId = "core");
crate::tag!(pub CoreKind = "core");

/// The projects folder, at the center.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CoreNode {
    pub id: CoreId,
    pub kind: CoreKind,
    /// ~/ and the projects folder's name
    pub name: String,
    pub path: String,
    pub home: Nullable<String>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
}

crate::tag!(pub NativeKind = "native");
crate::tag!(pub CodexApp = "codex");

/// A Codex desktop project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NativeNode {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?native:codex:"))]
    pub id: String,
    pub kind: NativeKind,
    pub app: CodexApp,
    pub name: String,
    pub roots: Vec<String>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    pub active: bool,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

// The app whose sessions a zone holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ZoneApp {
    Claude,
    Codex,
}

crate::tag!(pub ZoneKind = "zone");

/// A folder outside the projects folder where sessions ran.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZoneNode {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?zone:"))]
    pub id: String,
    pub kind: ZoneKind,
    pub name: String,
    pub path: Nullable<String>,
    pub app: Nullable<ZoneApp>,
    pub external: True,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    pub active: bool,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

// A note in the shared context's inbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InboxNote {
    pub id: String,
    pub title: String,
    pub app: Nullable<String>,
}

crate::tag!(pub WikiKind = "wiki");

/// The shared context: Xenolith, or Agent Wiki before it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WikiNode {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?resource:[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))]
    pub id: String,
    pub kind: WikiKind,
    pub name: String,
    /// The MCP server that writes to it: xenolith or agent-wiki, or the name the folder's marker (xenolith.json) gives
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"))]
    pub server: Opt<String>,
    pub path: String,
    pub page_count: Uint,
    pub inbox: Vec<InboxNote>,
    /// Why XenoNet shows none of its pages, log or inbox: the folder follows a later version of Xenolith's format than XenoNet reads
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub problem: Opt<String>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

/// A front matter value: text, or a JSON list when it starts with [
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FrontMatter {
    Text(String),
    List(Vec<Any>),
}

// A write to a context page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageWrite {
    pub ts: Ms,
    pub app: String,
    pub title: String,
    pub pages: Vec<String>,
}

crate::tag!(pub PageKind = "page");

/// A context page: a page of the shared context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageNode {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?wiki:"))]
    pub id: String,
    pub kind: PageKind,
    pub name: FrontMatter,
    pub slug: String,
    #[serde(rename = "type")]
    pub page_type: FrontMatter,
    pub summary: FrontMatter,
    pub tags: Vec<Any>,
    pub updated: Nullable<Ms>,
    pub updated_by: Nullable<FrontMatter>,
    pub path: String,
    #[schemars(length(max = 6))]
    pub writes: Vec<PageWrite>,
    /// Other names it goes by, from its aliases, in lowercase; left out when it has none
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub aliases: Opt<Vec<String>>,
    /// The Xenolith space it's in, such as project:atlas, when that isn't this machine's own; its id is then wiki:<space>/<slug>
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub space: Opt<String>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

crate::tag!(pub ResourceKind = "resource");

/// A shared system sessions reach through MCP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceNode {
    #[schemars(regex(pattern = r"^(?:@[a-z][a-z0-9-]{1,31}:)?resource:"))]
    pub id: String,
    pub kind: ResourceKind,
    /// What the shared system is called: the MCP server, or for a server whose id is a uuid the name it says it is, or "Connector" and the start of the id
    pub name: String,
    /// The MCP server's id, when name is something else
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub server: Opt<String>,
    /// Where a name that isn't the server's id came from
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub name_source: Opt<NameSource>,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
    /// The host it's from, for a node from another machine; its id is then @<host>:…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live, so it shows what was true when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

// How another machine's XenoNet is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HostState {
    Connecting,
    Live,
    Stale,
    Offline,
    Error,
}

crate::tag!(pub HostKind = "host");

/// Another machine's XenoNet, at the center of its constellation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostNode {
    #[schemars(regex(pattern = r"^@[a-z][a-z0-9-]{1,31}:core$"))]
    pub id: String,
    pub kind: HostKind,
    pub host: String,
    /// Its id here
    pub name: String,
    /// Its projects folder, as it names it (~/Projects)
    pub path: Nullable<String>,
    /// What it calls itself, when that isn't its id here
    pub calls: Nullable<String>,
    /// connecting; live; stale (no word for 45 seconds); offline (the connection ended, and XenoNet tries again); error (it needs you)
    pub state: HostState,
    pub since: Ms,
    pub last_heard: Nullable<Ms>,
    /// Its clock minus this one's, once known
    pub offset_ms: Nullable<f64>,
    /// Its XenoNet
    pub version: Nullable<String>,
    pub platform: Nullable<String>,
    /// It shows made-up data
    pub demo: Nullable<bool>,
    /// Why it isn't live, in words
    pub error: Nullable<String>,
    /// What's shown is its last snapshot, kept from before this XenoNet started
    pub cached: bool,
    /// It sent more than the 5,000 nodes a host may have on the map, so its oldest sessions are left out
    pub truncated: bool,
    pub stale: bool,
    #[serde(flatten)]
    pub place: Place,
    #[serde(flatten)]
    pub tally: Tally,
}

// A node on the map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Node {
    /// A session (session/v1) where it lives on the map
    Session(Box<SessionNode>),
    /// A project or group (project/v1) on the map
    Project(Box<ProjectNode>),
    /// The projects folder, at the center
    Core(CoreNode),
    /// A Codex desktop project
    Native(NativeNode),
    /// A folder outside the projects folder where sessions ran: the home folder, scratch folders, temp, or one folder
    Zone(ZoneNode),
    /// The shared context: Xenolith, or Agent Wiki before it
    Wiki(WikiNode),
    /// A context page: a page of the shared context
    Page(PageNode),
    /// A shared system sessions reach through MCP
    Resource(ResourceNode),
    /// Another machine's XenoNet, at the center of its constellation (with hosts on)
    Host(HostNode),
}

// What a link in the tree means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    Contains,
    Nested,
    Satellite,
    Native,
    Zone,
    Session,
    Lineage,
    LineageCross,
    Wiki,
    Page,
    Resource,
    Host,
}

// A link in the tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    pub source: String,
    pub target: String,
    pub kind: LinkKind,
}

// What a tie outside the tree means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RelationKind {
    Wikilink,
    Mentions,
    SameRepo,
}

// A tie outside the tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Relation {
    pub a: String,
    pub b: String,
    pub kind: RelationKind,
}

// How many of each thing the map holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Context pages on the map
    pub wiki_pages: Uint,
    pub dirty_repos: Uint,
    pub projects: Uint,
    pub groups: Uint,
    pub sessions: Uint,
    pub claude: Uint,
    pub codex: Uint,
    pub active: Uint,
    pub archived: Uint,
}

// Another machine on the map, and how it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub state: HostState,
    pub since: Ms,
    pub last_heard: Nullable<Ms>,
    pub offset_ms: Nullable<f64>,
    pub version: Nullable<String>,
    pub platform: Nullable<String>,
    pub demo: Nullable<bool>,
    pub error: Nullable<String>,
    pub cached: bool,
    pub truncated: bool,
    pub nodes: Uint,
    pub sessions: Uint,
}

// How one source of the map went.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// projects, codex, codex-projects, claude, live, wiki (the shared context), search, identities
    pub name: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub error: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub note: Opt<String>,
}

/// The map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub generated_at: Ms,
    pub nodes: Vec<Node>,
    /// The tree, by node ids; lineage-cross ties a session to its parent in another place, host ties another machine's node to this one's core, wiki ties the shared context to the core, and page a context page to the shared context
    pub links: Vec<Link>,
    /// Ties outside the tree: context pages that link to each other (wikilink), pages that mention a project (mentions), and checkouts of one repo on different machines (same-repo)
    pub relations: Vec<Relation>,
    pub stats: Stats,
    pub identities: Vec<Ref<Identity>>,
    /// Start with identities masked (XENONET_HIDE_IDENTITIES=1)
    pub hide_identities: bool,
    /// The demo's made-up map, whose sessions exist only in XenoNet
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub demo: Opt<bool>,
    /// The other machines on the map and how each is, with hosts on
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub hosts: Opt<Vec<Host>>,
    /// This XenoNet's own host id, if it named itself, with hosts on
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_id: Option<String>,
    /// Where the map came from, and how each source went this time
    pub sources: Vec<Source>,
}

impl Document for Graph {
    const ID: &'static str = "graph/v1";
    const TITLE: &'static str = "Graph";
    const DESCRIPTION: &'static str = "The map: the projects folder, its groups and projects, Codex desktop projects, the folders outside it where sessions ran, the sessions, Xenolith and the MCP servers sessions share, with the identities work ran as.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Graph>];
