//! An actor product's Mac app, as its observer reads it (Grok Bot's,
//! Muse's; core/actors/app-bundle.js): whether it's installed where apps
//! go, and from its Info.plist the keys asked for and nothing else of the
//! app's: its version (CFBundleShortVersionString), and for an app whose
//! name isn't its maker's alone, its bundle id (CFBundleIdentifier), which
//! says whose it is. Never its data, logs, preferences or tokens. The
//! Info.plist is read again only when it changes.
//!
//! Its collaborators are the app's own fields, as the JS read them: "apps",
//! "id" (the ids that are the product's, a pattern to test), "files" and
//! "binary" (a key read from a binary Info.plist).

use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_base::nodepath::style_of;
use xenonet_base::paths::tilde;
use xenonet_base::sync::Locked;
use xenonet_engine::jsre::js;
use xenonet_engine::peer::{list, Calls, Result, Thrown};
use xenonet_engine::sql::Arg;

/// A key's string from an Info.plist written as XML, that key alone, or
/// None.
pub fn plist_string(text: &str, key: &str) -> Option<String> {
    let r = js(&format!(r"<key>\s*{key}\s*</key>\s*<string>\s*([^<]*?)\s*</string>"), false);
    r.captures(text).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

// A version as an app gives it: short, with no spaces.
fn version_ok(v: &str) -> bool {
    let b = v.as_bytes();
    !b.is_empty() && b.len() <= 32 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
}

// A bundle id, reverse DNS, as Apple allows it.
fn bundle_id_ok(v: &str) -> bool {
    let b = v.as_bytes();
    !b.is_empty() && b.len() <= 155 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
}

fn why(code: Option<&str>) -> &'static str {
    match code {
        Some("ENOENT" | "ENOTDIR") => "missing",
        Some("EACCES" | "EPERM") => "denied",
        _ => "unreadable",
    }
}

fn code_of(t: &Thrown) -> Option<String> {
    t.code.as_ref().and_then(|c| c.as_str().map(str::to_string))
}

/// The last read, and the Info.plist it came from as it was then.
#[derive(Clone, Debug, PartialEq)]
pub struct Plist {
    pub file: Option<String>,
    pub stamp: Value,
    pub version: Option<String>,
    pub id: Option<String>,
    pub state: String,
}

impl Plist {
    pub fn json(&self) -> Value {
        json!({ "file": self.file, "stamp": self.stamp, "version": self.version, "id": self.id, "state": self.state })
    }
}

#[derive(Default)]
pub struct AppBundle {
    plist: Mutex<Option<Plist>>,
}

impl AppBundle {
    pub fn plist(&self) -> Option<Plist> {
        self.plist.locked().clone()
    }

    /// Where the app is, or None.
    pub fn where_(&self, c: &mut dyn Calls) -> Result<Option<String>> {
        for p in list(c.get("apps")?) {
            if c.call("files", "existsSync", vec![p.clone()]).is_ok_and(|r| r.truthy()) {
                return Ok(Some(xenonet_engine::store::js_text(&p)));
            }
        }
        Ok(None)
    }

    /// Whether the last read found the product's bundle id. Before any
    /// read, an app of its name counts.
    pub fn ours(&self, c: &mut dyn Calls) -> Result<bool> {
        let plist = self.plist();
        if !c.get("id")?.truthy() || plist.is_none() {
            return Ok(true);
        }
        match plist.and_then(|p| p.id) {
            None => Ok(false),
            Some(id) => Ok(c.call("id", "test", vec![Arg::text(id)])?.truthy()),
        }
    }

    /// Whether the app is installed: there, and the product's by its id as
    /// last read.
    pub fn installed(&self, c: &mut dyn Calls) -> Result<bool> {
        Ok(self.where_(c)?.is_some() && self.ours(c)?)
    }

    /// The installed app's version, with its bundle id when that's asked
    /// for, read again only when the file changes.
    pub fn read(&self, c: &mut dyn Calls) -> Result<Plist> {
        let app = self.where_(c)?;
        let file = app.as_ref().map(|a| style_of(a).join(&[a, "Contents", "Info.plist"]));
        let mut stamp = Value::Null;
        let mut failed: Option<Thrown> = None;
        if let Some(f) = &file {
            match c.call("files", "statSync", vec![Arg::text(f)]) {
                Ok(st) => {
                    let n = |k: &str| xenonet_engine::store::js_text(&st.field(k));
                    stamp = Value::String(format!("{}:{}", n("mtimeMs"), n("size")));
                }
                Err(e) => {
                    stamp = Value::String(code_of(&e).unwrap_or_else(|| "?".into()));
                    failed = Some(e);
                }
            }
        }
        if let Some(p) = self.plist() {
            if p.file == file && p.stamp == stamp {
                return Ok(p);
            }
        }
        let mut read = Plist { file: file.clone(), stamp, version: None, id: None, state: failed.as_ref().map_or("missing", |e| why(code_of(e).as_deref())).into() };
        *self.plist.locked() = Some(read.clone());
        let Some(file) = file.filter(|_| failed.is_none()) else { return Ok(read) };
        let done = (|| -> Result<()> {
            let bytes = match c.call("files", "readFileSync", vec![Arg::text(&file)])? {
                Arg::Bytes(b) => b,
                other => xenonet_engine::store::js_text(&other).into_bytes(),
            };
            let binary = bytes.len() >= 6 && &bytes[..6] == b"bplist";
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let key = |c: &mut dyn Calls, k: &str| -> Result<Option<String>> {
                if binary {
                    let v = c.call("", "binary", vec![Arg::text(&file), Arg::text(k)])?;
                    Ok(if v.is_nullish() { None } else { Some(xenonet_engine::store::js_text(&v)) })
                } else {
                    Ok(plist_string(&text, k))
                }
            };
            let version = key(c, "CFBundleShortVersionString")?;
            read.version = version.filter(|v| version_ok(v));
            *self.plist.locked() = Some(read.clone());
            if c.get("id")?.truthy() {
                let id = key(c, "CFBundleIdentifier")?;
                read.id = id.filter(|i| bundle_id_ok(i));
            }
            read.state = if read.version.is_some() { "ok" } else { "unreadable" }.into();
            Ok(())
        })();
        if let Err(e) = done {
            read.state = why(code_of(&e).as_deref()).into();
        }
        *self.plist.locked() = Some(read.clone());
        Ok(read)
    }

    /// What the doctor shows of the app: whether it's there (and, when its
    /// id is asked, whether it's the product's), and whether its version
    /// could be read.
    pub fn sources(&self, c: &mut dyn Calls, home: &str) -> Result<Value> {
        let app = self.where_(c)?;
        let at = match &app {
            Some(a) => a.clone(),
            None => list(c.get("apps")?).first().map_or_else(|| "undefined".into(), xenonet_engine::store::js_text),
        };
        let plist = self.plist();
        let other = plist.as_ref().is_some_and(|p| p.id.is_some()) && !self.ours(c)?;
        let state = if app.is_none() {
            "missing"
        } else if other {
            "other"
        } else {
            "ok"
        };
        let version_file = plist.as_ref().and_then(|p| p.file.clone()).unwrap_or_else(|| style_of(&at).join(&[&at, "Contents", "Info.plist"]));
        Ok(json!([
            { "name": "app", "file": tilde(&at, home), "state": state },
            { "name": "app's version", "file": tilde(&version_file, home), "state": plist.map_or_else(|| "missing".to_string(), |p| p.state) },
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_keys() {
        let xml = "<dict>\n<key>CFBundleShortVersionString</key>\n\t<string> 1.2.3 </string><key>CFBundleIdentifier</key><string>com.meta.endo</string></dict>";
        assert_eq!(plist_string(xml, "CFBundleShortVersionString").as_deref(), Some("1.2.3"));
        assert_eq!(plist_string(xml, "CFBundleIdentifier").as_deref(), Some("com.meta.endo"));
        assert!(version_ok("1.2.3-beta+4") && !version_ok("1 2") && !version_ok(""));
    }
}
