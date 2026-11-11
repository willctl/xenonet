//! Your standing settings and what reads them: what XenoNet saves for you
//! (core/store.js, now the store's),
//! themes (core/themefiles.js), the xenonet command on your PATH
//! (core/command.js), opening things (core/open.js), the schema checks
//! (core/schema/index.js) and the demo's made-up world (core/demo.js).

use super::{pending, Pending, R};
use serde_json::Value;

const STORE: &str = "core/store.js";
const THEMES: &str = "core/themefiles.js";
const COMMAND: &str = "core/command.js";
const OPEN: &str = "core/open.js";
const SCHEMAS: &str = "core/schema/index.js";
const DEMO: &str = "core/demo.js";

/// What XenoNet saves for you on this machine (the store's state domain):
/// the map's layout, dismissed alerts, markers, the projects folder you
/// chose and your rules for it, each app's settings, your theme and
/// environment, and what you said of identities with what XenoNet has
/// seen of them. Each part answers as its JSON object, in the order its
/// keys were saved.
pub trait Store: Send + Sync {
    fn ack(&self, _session: &str) -> R<()> {
        pending(STORE)
    }
    /// The alerts dismissed: `{ session: at }`.
    fn acks(&self) -> R<Value> {
        pending(STORE)
    }
    fn marks(&self) -> R<Value> {
        pending(STORE)
    }
    fn layout(&self) -> R<Value> {
        pending(STORE)
    }
    /// Each id's new place (`{ x, y, z }`, kept to two decimals), or null
    /// to forget it; a place that isn't three numbers is left alone.
    fn set_layout(&self, _positions: &Value) -> R<()> {
        pending(STORE)
    }
    fn clear_layout(&self) -> R<()> {
        pending(STORE)
    }
    /// `store.getTheme()`, null when you never picked one.
    fn theme(&self) -> R<Value> {
        pending(STORE)
    }
    fn set_theme(&self, _id: &str) -> R<()> {
        pending(STORE)
    }
    fn environment(&self) -> R<Value> {
        pending(STORE)
    }
    fn set_environment(&self, _id: &str) -> R<()> {
        pending(STORE)
    }
    /// The projects folder you chose, or null.
    fn workspace_root(&self) -> R<Value> {
        pending(STORE)
    }
    fn set_workspace_root(&self, _dir: Option<&str>) -> R<()> {
        pending(STORE)
    }
    /// Your rules for the projects folder: `{ key: rule }`.
    fn project_rules(&self) -> R<Value> {
        pending(STORE)
    }
    /// A rule, or (None) its removal.
    fn set_project_rule(&self, _key: &str, _rule: Option<&Value>) -> R<()> {
        pending(STORE)
    }
    /// Every rule, or with a projects folder's path key only those for the
    /// folders inside it.
    fn clear_project_rules(&self, _under_key: Option<&str>) -> R<()> {
        pending(STORE)
    }
    /// An app's setting for a scope (`picks` for a projects folder's key),
    /// if it was saved.
    fn app_setting(&self, _app: &str, _name: &str, _scope: &str) -> R<Option<Value>> {
        pending(STORE)
    }
    fn set_app_setting(&self, _app: &str, _name: &str, _scope: &str, _value: Option<&Value>) -> R<()> {
        pending(STORE)
    }
    /// `store.getIdentities()`: the names you gave identities, the ones
    /// you hid, the ones you pinned to a node, what XenoNet knows of each
    /// and the sign-ins it has seen (`{ names, hidden, pins, known,
    /// signins }`), which a hub's merged map applies to other hosts'
    /// identities too.
    fn identities(&self) -> R<Value> {
        pending(STORE)
    }
    fn set_identity_name(&self, _id: &str, _name: Option<&str>) -> R<()> {
        pending(STORE)
    }
    fn set_identity_hidden(&self, _id: &str, _hidden: bool) -> R<()> {
        pending(STORE)
    }
    /// The identity pinned to a node, now, or (None) no pin.
    fn set_identity_pin(&self, _node: &str, _identity: Option<&str>) -> R<()> {
        pending(STORE)
    }
    /// What XenoNet knows of an identity, in place of what it knew.
    fn set_identity_known(&self, _id: &str, _value: &Value) -> R<()> {
        pending(STORE)
    }
    /// An app's sign-ins as XenoNet has seen them, in place of what was.
    fn set_signins(&self, _app: &str, _spans: &Value) -> R<()> {
        pending(STORE)
    }
}

pub trait Themes: Send + Sync {
    /// `readThemes(dir)`: `{ themes: { id: theme }, problems }`.
    fn read(&self, _dir: &str) -> R<Value> {
        pending(THEMES)
    }
    /// `copyTheme(dir, from, id, name)`: the new file.
    fn copy(&self, _dir: &str, _from: &str, _id: &str, _name: &Value) -> R<Value> {
        pending(THEMES)
    }
    /// The theme in use: the one `store` names, else XenoNet's own, from
    /// the themes in `dir`. The routes answer with it, and a theme's edit
    /// sends it to the pages.
    fn current(&self, dir: &str, store: &dyn Store) -> R<Value> {
        let read = self.read(dir)?;
        let id = store.theme()?;
        let themes = &read["themes"];
        Ok(id.as_str().and_then(|i| themes.get(i)).or_else(|| themes.get("xenonet")).cloned().unwrap_or(Value::Null))
    }
}

pub trait Command: Send + Sync {
    /// `commandState()`.
    fn state(&self) -> R<Value> {
        pending(COMMAND)
    }
    /// `installCommand({ entry, runtime })`.
    fn install(&self, _entry: &str, _runtime: Option<&str>) -> R<Value> {
        pending(COMMAND)
    }
}

pub trait Opener: Send + Sync {
    /// `urlFor(node, action)`: what an action opens, or None.
    fn url_for(&self, _node: &Value, _action: &str) -> R<Option<String>> {
        pending(OPEN)
    }
    /// `handoffUrl(target, folder, brief)`.
    fn handoff_url(&self, _target: &str, _folder: &Value, _brief: &str) -> R<String> {
        pending(OPEN)
    }
    /// `launch(target)`: the service opens it itself.
    fn launch(&self, _target: &str) -> R<()> {
        pending(OPEN)
    }
    /// `reveal(file)`: shows it in the file manager.
    fn reveal(&self, _file: &str) -> R<()> {
        pending(OPEN)
    }
}

/// The open.js constants the routes check against.
pub const MAX_BRIEF: usize = 6000;
pub const NEEDS_INPUT_URL: &str = "claude://code/needs-input";

pub trait Schemas: Send + Sync {
    /// `check(schema, value)`: Ok, or the first issue as `<path> <message>`.
    /// One that can't be checked yet is refused.
    fn check(&self, _schema: &str, _value: &Value) -> Result<(), String> {
        Err(format!("the body can't be checked yet: {SCHEMAS} isn't in the Rust service"))
    }
}

pub trait Demo: Send + Sync {
    /// `createWorld()`: the made-up workspace's folders, written.
    fn create_world(&self) -> R<Value> {
        pending(DEMO)
    }
}

impl Store for Pending {}
impl Themes for Pending {}
impl Command for Pending {}
impl Opener for Pending {}
impl Schemas for Pending {}
impl Demo for Pending {}
