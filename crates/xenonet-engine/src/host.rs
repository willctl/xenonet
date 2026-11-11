//! What the engine asks of its collaborators, one trait each: the clock,
//! the processes, the map's narrow readers, the actors, the repos, the
//! sessions listening after a turn, the waits on mail, the events and the
//! network across machines; what the engine keeps is `store::Store`, and
//! live delivery and the policy are
//! `delivery::router::Delivery` and `delivery::policy::Guard`. The binary
//! implements them over its parts; a test implements them over made-up
//! data. `Collaborators` bundles them for one call.

use crate::delivery::policy::Guard;
use crate::delivery::router::Delivery;
use crate::engine::Result;
use crate::store::Store;
use serde_json::Value;

/// The engine's clock: the time it reasons with, the wall clock a
/// listening hook's deadline is set by, and the time as the locale writes
/// it.
pub trait Clock {
    /// Milliseconds since the epoch.
    fn now(&self) -> f64;
    /// The wall clock, for a deadline.
    fn wall(&self) -> f64 {
        self.now()
    }
    /// A time as the hour and minute read in this locale (9:04 AM).
    fn time_of(&self, ms: f64) -> String;
}

/// The processes on this machine.
pub trait Processes {
    /// Whether a process still runs.
    fn alive(&self, pid: f64) -> bool;
}

/// A node of the map as the engine indexes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    pub kind: Option<String>,
    pub archived: bool,
    /// When it last changed, when the map says.
    pub updated_at: Option<f64>,
    pub name: Option<String>,
    pub path: Option<String>,
}

/// The map's state, read narrowly: it belongs to the state's own layer.
pub trait MapReader {
    /// Each live session's id and when its transcript last moved.
    fn live_ids(&self) -> Result<Vec<(String, f64)>>;
    /// Every node, in the map's order.
    fn index(&self) -> Result<Vec<IndexEntry>>;
    /// A number that changes whenever `index()` would answer differently
    /// (the map's refresh moves it), for the engine's session index to read
    /// the nodes again only then; None when the reader can't say, and the
    /// index reads them on every call.
    fn nodes_version(&self) -> Option<u64> {
        None
    }
    /// A node as the engine reads it: id, kind, title, name, worktree, cwd,
    /// updatedAt, branch, reader, observed, path, archived and its
    /// identities ({ id }), each as the map has it.
    fn node(&self, id: &str) -> Result<Option<Value>>;
    /// A live session (or an observed run): lastEventAt, endedAt, status,
    /// needsYou, observed and its edits ({ path, ts }).
    fn live(&self, id: &str, now: f64) -> Result<Option<Value>>;
    /// Whether the live record is one XenoNet only observes.
    fn observed_live(&self, id: &str) -> Result<bool>;
    /// The project or zone a session counts under: its node id.
    fn home_of(&self, id: &str) -> Result<Option<Value>>;
    /// The identities on the map, as { id, handle }.
    fn identities(&self) -> Result<Vec<Value>>;
}

/// What an actor's status line was set to, or why it wasn't.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusSet {
    Ok(Value),
    Refused(String),
}

/// The actors, as the engine counts them among the agents.
pub trait ActorsView {
    fn has(&self, id: &str) -> Result<bool>;
    fn is_registered(&self, id: &str) -> Result<bool>;
    /// The project or folder holding an absolute path: its node id, or
    /// None.
    fn place_of(&self, dir: &str) -> Result<Option<Value>>;
    /// The actors at work since `since`.
    fn engine_ids(&self, since: f64) -> Result<Vec<String>>;
    /// An actor as the engine needs it: name, presence, status, lastSeen,
    /// cwd, place, edits; None for one that isn't here.
    fn engine_view(&self, id: &str, now: f64) -> Result<Option<Value>>;
    fn set_status(&self, id: &str, text: &str) -> Result<StatusSet>;
    /// What's known of an actor (its product, how it's here), or None.
    fn info(&self, id: &str) -> Result<Option<Value>>;
}

/// The git repository a folder is in.
#[derive(Clone, Debug, PartialEq)]
pub struct Repo {
    /// The working tree's top.
    pub top: String,
    /// The shared git dir, the same for every worktree.
    pub common: String,
    /// The branch, None on a detached HEAD or when git didn't say.
    pub branch: Option<String>,
}

/// Which repository each folder belongs to.
pub trait Repos {
    fn of(&self, dir: &str) -> Result<Option<Repo>>;
    /// Several folders' repos, in order; the binary asks git for them at
    /// once.
    fn of_each(&self, dirs: &[String]) -> Vec<Result<Option<Repo>>> {
        dirs.iter().map(|d| self.of(d)).collect()
    }
}

/// The sessions listening after a turn (their Stop hooks), until when.
pub trait Listening {
    fn set(&self, id: &str, until: f64);
    fn delete(&self, id: &str);
    fn has(&self, id: &str) -> bool;
}

/// The waits on mail: an inbox's, and a listening hook's.
pub trait Waits {
    /// A message arrived for `id`: every wait on it ends.
    fn wake_waiters(&self, id: &str);
    /// Until a message comes for `me` or `ms` pass.
    fn wait(&self, me: &str, ms: f64);
    /// A listening hook's wait: until a message comes, the main Stop hook
    /// takes one, the hook's request goes, or `ms` pass.
    fn listen_wait(&self, me: &str, ms: f64);
    /// Whether the listening hook's request went away.
    fn aborted(&self) -> bool;
}

/// What the engine tells: message, seen and contact events.
pub trait Events {
    fn emit(&self, name: &str, payload: Value);
}

/// The network across machines, as the engine reaches other hosts.
pub trait Net {
    /// Other hosts' agents as their presence lists them.
    fn agents(&self) -> Result<Vec<Value>>;
    /// The same with each agent's detail, for the console.
    fn console(&self) -> Result<Vec<Value>>;
    /// An agent on another host by its qualified id: as presence shows
    /// it, or a stand-in.
    fn view(&self, id: &str) -> Result<Value>;
    /// The project a folder is a confirmed checkout of (`prj_…`), what
    /// agents are matched by across machines, or None.
    fn project_key(&self, cwd: &str) -> Result<Option<String>>;
    /// A message stored here goes to its recipients on other hosts.
    fn sent(&self, message: &Value) -> Result<()>;
    /// Agents on other hosts with a hand in `files`, in their checkouts of
    /// the same project.
    fn claim_hits(&self, me: &str, files: &[String]) -> Result<Vec<Value>>;
    /// The handoffs open to a session with this view.
    fn handoffs(&self, view: &Value) -> Result<Vec<Value>>;
    /// What another host last said of a delivery: { host, receipt }, or
    /// None.
    fn receipt(&self, message_id: f64, agent: &str) -> Result<Option<Value>>;
}

/// Where a path is in its project: the checkout with the longest path at
/// or above it, the project that checkout is in (or, for a candidate, the
/// one it would join), and the path inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub checkout: String,
    /// The checkout's folder.
    pub top: String,
    pub project: String,
    /// Inside the checkout, with / between parts, a folder's trailing /
    /// kept; empty for its top.
    pub rel: String,
    /// In its project: the founding checkout, or one the person confirmed.
    pub confirmed: bool,
}

/// This machine's projects, as claims need them (xenonet-store's
/// projects): where a path is, who may claim in a project, and the
/// network's copy of each claim.
pub trait Projects {
    /// A session's folder is a checkout: its own, made when the map hasn't
    /// made it yet.
    fn ensure(&self, folder: &str) -> Result<()>;
    /// Where an absolute path (or a glob's folder) is, or None outside
    /// every checkout.
    fn place(&self, path: &str) -> Result<Option<Place>>;
    /// Whether `me` may claim in `project`: an actor only in the projects
    /// its grant names, decided with its token. Why not, when it may not.
    fn may_claim(&self, me: &str, project: &str) -> Result<()>;
    /// Whom this machine's claims are authored by: its signing key's id.
    fn author(&self) -> String;
    /// A claim or a release (a `network/claim/v1` body) goes to the
    /// network's other members, when this machine is in one.
    fn claimed(&self, body: Value);
}

/// The engine's collaborators, for one call. The actors, live delivery,
/// the policy and the network may be absent: without actors there are
/// only sessions; without delivery, messages wait for the hooks; without a
/// policy, everyone sees everyone; off a network, nothing is remote.
pub struct Collaborators<'a> {
    pub store: &'a dyn Store,
    pub clock: &'a dyn Clock,
    pub processes: &'a dyn Processes,
    pub map: &'a dyn MapReader,
    pub actors: Option<&'a dyn ActorsView>,
    pub delivery: Option<&'a dyn Delivery>,
    pub policy: Option<&'a dyn Guard>,
    pub repos: &'a dyn Repos,
    pub listening: &'a dyn Listening,
    pub waits: &'a dyn Waits,
    pub events: &'a dyn Events,
    pub net: Option<&'a dyn Net>,
    /// Without projects, nothing can be claimed.
    pub projects: Option<&'a dyn Projects>,
}
