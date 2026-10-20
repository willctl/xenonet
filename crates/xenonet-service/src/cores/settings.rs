//! Your standing settings and what reads them: state.json's (core/store.js),
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

pub trait Store: Send + Sync {
    fn ack(&self, _session: &str) -> R<()> {
        pending(STORE)
    }
    fn layout(&self) -> R<Value> {
        pending(STORE)
    }
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
    /// `store.loadReport()`, for the doctor.
    fn load_report(&self) -> R<Value> {
        pending(STORE)
    }
    /// `store.getIdentities()`: the names you gave identities and the ones
    /// you hid (`{ names, hidden, … }`), which a hub's merged map applies to
    /// other hosts' identities too.
    fn identities(&self) -> R<Value> {
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
