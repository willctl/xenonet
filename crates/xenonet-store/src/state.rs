//! What XenoNet saves for you on this machine, in the store: the projects
//! folder you chose, your theme and environment, where you dragged things
//! on the map, the alerts you dismissed, markers, your rules for the
//! projects folder, each app's settings (the sorter's picks for each
//! folder among them), and what you said of identities (their names,
//! which you hid, which you pinned to a session) with what XenoNet has
//! seen of them (what it knows of each, the sign-ins).
//!
//! Each part is answered as the JSON object it has always been, its keys
//! in the order they were first saved; a value that is a document is kept
//! as its JSON text, written and read as XenoNet's files were.

use crate::{Db, StoreError, R};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use xenonet_text::json::{number, parse, stringify};

pub(crate) const TABLES: &[&str] = &[
    // Your settings: the projects folder (`workspace.root`), the theme and
    // the environment.
    "CREATE TABLE settings (k TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT",
    // Each app's settings, by name and the scope they're for: the sorter's
    // picks for a projects folder are `picks` scoped by the folder's key.
    "CREATE TABLE app_settings (
        app TEXT NOT NULL,
        name TEXT NOT NULL,
        scope TEXT NOT NULL,
        v TEXT NOT NULL,
        PRIMARY KEY (app, name, scope)
    ) STRICT",
    // Where you dragged each thing on the map.
    "CREATE TABLE layout (id TEXT PRIMARY KEY, x REAL NOT NULL, y REAL NOT NULL, z REAL NOT NULL) STRICT",
    // The alerts you dismissed, by session, and when.
    "CREATE TABLE acks (session TEXT PRIMARY KEY, at REAL NOT NULL) STRICT",
    "CREATE TABLE marks (id TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT",
    // Your rules for the projects folder, by the folder's path key.
    "CREATE TABLE project_rules (key TEXT PRIMARY KEY, rule TEXT NOT NULL) STRICT",
    // What you said of identities, and what XenoNet has seen of them.
    "CREATE TABLE identity_names (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT",
    "CREATE TABLE identity_hidden (id TEXT PRIMARY KEY) STRICT",
    "CREATE TABLE identity_pins (node TEXT PRIMARY KEY, identity TEXT NOT NULL, at REAL NOT NULL) STRICT",
    "CREATE TABLE identity_known (id TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT",
    "CREATE TABLE signins (app TEXT PRIMARY KEY, spans TEXT NOT NULL) STRICT",
];

fn text_of(v: &Value) -> String {
    stringify(v, 0)
}

fn value_of(text: &str) -> R<Value> {
    parse(text).map_err(|_| StoreError::new("bad-entry", format!("a saved value isn't JSON: {text}")))
}

// A table's rows as an object, each row's key to its value, in the order
// they were first saved.
fn object(c: &Connection, sql: &str, row: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<(String, Value)>) -> R<Value> {
    let mut q = c.prepare_cached(sql)?;
    let rows = q.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Object(rows.into_iter().collect::<Map<String, Value>>()))
}

// A table's rows of (key, JSON text) as an object.
fn documents(c: &Connection, sql: &str) -> R<Value> {
    let mut q = c.prepare_cached(sql)?;
    let rows: Vec<(String, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out = Map::new();
    for (k, t) in rows {
        out.insert(k, value_of(&t)?);
    }
    Ok(Value::Object(out))
}

impl Db {
    /// One of your settings, or null when it was never saved.
    pub fn setting(&self, k: &str) -> R<Value> {
        let t: Option<String> = self.read(|c| Ok(c.query_row("SELECT v FROM settings WHERE k = ?1", params![k], |r| r.get(0)).optional()?))?;
        t.map_or(Ok(Value::Null), |t| value_of(&t))
    }

    /// A setting saved, or (None) forgotten.
    pub fn set_setting(&self, k: &str, v: Option<&Value>) -> R<()> {
        self.write(|c| {
            match v {
                Some(v) => c.execute("INSERT INTO settings (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v", params![k, text_of(v)])?,
                None => c.execute("DELETE FROM settings WHERE k = ?1", params![k])?,
            };
            Ok(())
        })
    }

    /// An app's setting for a scope, if it was saved.
    pub fn app_setting(&self, app: &str, name: &str, scope: &str) -> R<Option<Value>> {
        let t: Option<String> =
            self.read(|c| Ok(c.query_row("SELECT v FROM app_settings WHERE app = ?1 AND name = ?2 AND scope = ?3", params![app, name, scope], |r| r.get(0)).optional()?))?;
        t.map(|t| value_of(&t)).transpose()
    }

    /// An app's setting for a scope saved, or (None) forgotten.
    pub fn set_app_setting(&self, app: &str, name: &str, scope: &str, v: Option<&Value>) -> R<()> {
        self.write(|c| {
            match v {
                Some(v) => c.execute(
                    "INSERT INTO app_settings (app, name, scope, v) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (app, name, scope) DO UPDATE SET v = excluded.v",
                    params![app, name, scope, text_of(v)],
                )?,
                None => c.execute("DELETE FROM app_settings WHERE app = ?1 AND name = ?2 AND scope = ?3", params![app, name, scope])?,
            };
            Ok(())
        })
    }

    /// Where you dragged each thing: `{ id: { x, y, z } }`.
    pub fn layout(&self) -> R<Value> {
        self.read(|c| {
            object(c, "SELECT id, x, y, z FROM layout ORDER BY rowid", |r| Ok((r.get(0)?, json!({ "x": number(r.get(1)?), "y": number(r.get(2)?), "z": number(r.get(3)?) }))))
        })
    }

    /// Places changed at once: each id's new place, or None to forget it.
    pub fn set_layout(&self, changes: &[(String, Option<[f64; 3]>)]) -> R<()> {
        self.write(|c| {
            for (id, place) in changes {
                match place {
                    Some([x, y, z]) => c.execute(
                        "INSERT INTO layout (id, x, y, z) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (id) DO UPDATE SET x = excluded.x, y = excluded.y, z = excluded.z",
                        params![id, x, y, z],
                    )?,
                    None => c.execute("DELETE FROM layout WHERE id = ?1", params![id])?,
                };
            }
            Ok(())
        })
    }

    /// Every place forgotten.
    pub fn clear_layout(&self) -> R<()> {
        self.write(|c| Ok(c.execute_batch("DELETE FROM layout")?))
    }

    /// The alerts you dismissed: `{ session: at }`.
    pub fn acks(&self) -> R<Value> {
        self.read(|c| object(c, "SELECT session, at FROM acks ORDER BY rowid", |r| Ok((r.get(0)?, number(r.get(1)?)))))
    }

    pub fn ack(&self, session: &str, at: f64) -> R<()> {
        self.write(|c| {
            c.execute("INSERT INTO acks (session, at) VALUES (?1, ?2) ON CONFLICT (session) DO UPDATE SET at = excluded.at", params![session, at])?;
            Ok(())
        })
    }

    pub fn marks(&self) -> R<Value> {
        self.read(|c| documents(c, "SELECT id, v FROM marks ORDER BY rowid"))
    }

    /// Your rules for the projects folder: `{ key: rule }`.
    pub fn project_rules(&self) -> R<Value> {
        self.read(|c| documents(c, "SELECT key, rule FROM project_rules ORDER BY rowid"))
    }

    /// A rule saved, or (None) taken out.
    pub fn set_project_rule(&self, key: &str, rule: Option<&Value>) -> R<()> {
        self.write(|c| {
            match rule {
                Some(r) => c.execute("INSERT INTO project_rules (key, rule) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET rule = excluded.rule", params![key, text_of(r)])?,
                None => c.execute("DELETE FROM project_rules WHERE key = ?1", params![key])?,
            };
            Ok(())
        })
    }

    /// Rules taken out at once.
    pub fn drop_project_rules(&self, keys: &[String]) -> R<()> {
        self.write(|c| {
            for k in keys {
                c.execute("DELETE FROM project_rules WHERE key = ?1", params![k])?;
            }
            Ok(())
        })
    }

    /// What you said of identities and what XenoNet has seen of them:
    /// `{ names, hidden, pins, known, signins }`.
    pub fn identities(&self) -> R<Value> {
        self.read(|c| {
            let names = object(c, "SELECT id, name FROM identity_names ORDER BY rowid", |r| Ok((r.get(0)?, Value::String(r.get(1)?))))?;
            let hidden = object(c, "SELECT id FROM identity_hidden ORDER BY rowid", |r| Ok((r.get(0)?, Value::Bool(true))))?;
            let pins = object(c, "SELECT node, identity, at FROM identity_pins ORDER BY rowid", |r| {
                Ok((r.get(0)?, json!({ "identity": r.get::<_, String>(1)?, "at": number(r.get(2)?) })))
            })?;
            let known = documents(c, "SELECT id, v FROM identity_known ORDER BY rowid")?;
            let signins = documents(c, "SELECT app, spans FROM signins ORDER BY rowid")?;
            Ok(json!({ "names": names, "hidden": hidden, "pins": pins, "known": known, "signins": signins }))
        })
    }

    /// The name you gave an identity, or (None) none.
    pub fn set_identity_name(&self, id: &str, name: Option<&str>) -> R<()> {
        self.write(|c| {
            match name {
                Some(n) => c.execute("INSERT INTO identity_names (id, name) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET name = excluded.name", params![id, n])?,
                None => c.execute("DELETE FROM identity_names WHERE id = ?1", params![id])?,
            };
            Ok(())
        })
    }

    pub fn set_identity_hidden(&self, id: &str, hidden: bool) -> R<()> {
        self.write(|c| {
            if hidden {
                c.execute("INSERT OR IGNORE INTO identity_hidden (id) VALUES (?1)", params![id])?;
            } else {
                c.execute("DELETE FROM identity_hidden WHERE id = ?1", params![id])?;
            }
            Ok(())
        })
    }

    /// The identity you pinned to a node, and when, or (None) no pin.
    pub fn set_identity_pin(&self, node: &str, pin: Option<(&str, f64)>) -> R<()> {
        self.write(|c| {
            match pin {
                Some((identity, at)) => c.execute(
                    "INSERT INTO identity_pins (node, identity, at) VALUES (?1, ?2, ?3) ON CONFLICT (node) DO UPDATE SET identity = excluded.identity, at = excluded.at",
                    params![node, identity, at],
                )?,
                None => c.execute("DELETE FROM identity_pins WHERE node = ?1", params![node])?,
            };
            Ok(())
        })
    }

    /// What XenoNet knows of an identity, in place of what it knew.
    pub fn set_identity_known(&self, id: &str, v: &Value) -> R<()> {
        self.write(|c| {
            c.execute("INSERT INTO identity_known (id, v) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET v = excluded.v", params![id, text_of(v)])?;
            Ok(())
        })
    }

    /// An app's sign-ins as XenoNet has seen them, in place of what was.
    pub fn set_signins(&self, app: &str, spans: &Value) -> R<()> {
        self.write(|c| {
            c.execute("INSERT INTO signins (app, spans) VALUES (?1, ?2) ON CONFLICT (app) DO UPDATE SET spans = excluded.spans", params![app, text_of(spans)])?;
            Ok(())
        })
    }
}
