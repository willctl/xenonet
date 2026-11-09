//! An actor product's Mac app, as its observer reads it (Grok Bot's,
//! Muse's): whether it's installed where apps go, and from its Info.plist
//! the keys asked for and nothing else of the app's: its version
//! (CFBundleShortVersionString), and for an app whose name isn't its
//! maker's alone, its bundle id (CFBundleIdentifier), which says whose it
//! is. Never its data, logs, preferences or tokens. The Info.plist is read
//! again only when it changes.

use super::{Fail, ObserverHost, Result};
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_base::json::number_string;
use xenonet_base::nodepath::style_of;
use xenonet_base::paths::tilde;
use xenonet_base::sync::Locked;
use xenonet_base::text::JS_SPACE;

/// A key's string from an Info.plist written as XML, that key alone, or
/// None.
pub fn plist_string(text: &str, key: &str) -> Option<String> {
    let r = regex::Regex::new(&format!(r"<key>[{JS_SPACE}]*{key}[{JS_SPACE}]*</key>[{JS_SPACE}]*<string>[{JS_SPACE}]*([^<]*?)[{JS_SPACE}]*</string>")).ok()?;
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

/// A product's app: its bundle's name (`Muse.app`), the bundle ids that
/// are the product's, and the last read of its Info.plist.
pub struct AppBundle {
    bundle: &'static str,
    id: regex::Regex,
    plist: Mutex<Option<Plist>>,
}

impl AppBundle {
    /// An app by its bundle's name, whose bundle id matches `id` (a
    /// pattern the adapter fixes).
    pub fn new(bundle: &'static str, id: &str) -> Self {
        AppBundle { bundle, id: regex::Regex::new(id).expect("a bundle id pattern that compiles"), plist: Mutex::new(None) }
    }

    pub fn plist(&self) -> Option<Plist> {
        self.plist.locked().clone()
    }

    /// Where the app would be: /Applications, then ~/Applications.
    pub fn apps(&self, h: &dyn ObserverHost) -> Vec<String> {
        ["/Applications".to_string(), xenonet_base::paths::join(h.home(), "Applications")].iter().map(|d| xenonet_base::paths::join(d, self.bundle)).collect()
    }

    /// Where the app is, or None.
    pub fn where_(&self, h: &dyn ObserverHost) -> Option<String> {
        self.apps(h).into_iter().find(|p| h.files().exists(p))
    }

    /// Whether the last read found the product's bundle id. Before any
    /// read, an app of its name counts.
    pub fn ours(&self) -> bool {
        match self.plist() {
            None => true,
            Some(p) => p.id.is_some_and(|id| self.id.is_match(&id)),
        }
    }

    /// Whether the app is installed: there, and the product's by its id as
    /// last read.
    pub fn installed(&self, h: &dyn ObserverHost) -> bool {
        self.where_(h).is_some() && self.ours()
    }

    /// The installed app's version, with its bundle id, read again only
    /// when the file changes.
    pub fn read(&self, h: &dyn ObserverHost) -> Plist {
        let app = self.where_(h);
        let file = app.as_ref().map(|a| style_of(a).join(&[a, "Contents", "Info.plist"]));
        let mut stamp = Value::Null;
        let mut failed: Option<Fail> = None;
        if let Some(f) = &file {
            match h.files().stat(f) {
                Ok(st) => stamp = Value::String(format!("{}:{}", number_string(st.mtime_ms), number_string(st.size))),
                Err(e) => {
                    stamp = Value::String(e.code.clone().unwrap_or_else(|| "?".into()));
                    failed = Some(e);
                }
            }
        }
        if let Some(p) = self.plist() {
            if p.file == file && p.stamp == stamp {
                return p;
            }
        }
        let mut read = Plist { file: file.clone(), stamp, version: None, id: None, state: failed.as_ref().map_or("missing", |e| why(e.code.as_deref())).into() };
        *self.plist.locked() = Some(read.clone());
        let Some(file) = file.filter(|_| failed.is_none()) else { return read };
        let done = (|| -> Result<()> {
            let bytes = h.files().read(&file)?;
            let binary = bytes.len() >= 6 && &bytes[..6] == b"bplist";
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let key = |k: &str| if binary { h.plist_key(&file, k) } else { plist_string(&text, k) };
            let version = key("CFBundleShortVersionString");
            read.version = version.filter(|v| version_ok(v));
            *self.plist.locked() = Some(read.clone());
            let id = key("CFBundleIdentifier");
            read.id = id.filter(|i| bundle_id_ok(i));
            read.state = if read.version.is_some() { "ok" } else { "unreadable" }.into();
            Ok(())
        })();
        if let Err(e) = done {
            read.state = why(e.code.as_deref()).into();
        }
        *self.plist.locked() = Some(read.clone());
        read
    }

    /// What the doctor shows of the app: whether it's there (and whether
    /// it's the product's), and whether its version could be read.
    pub fn sources(&self, h: &dyn ObserverHost) -> Vec<Value> {
        let home = h.home();
        let app = self.where_(h);
        let at = match &app {
            Some(a) => a.clone(),
            None => self.apps(h).first().map_or_else(|| "undefined".into(), String::clone),
        };
        let plist = self.plist();
        let other = plist.as_ref().is_some_and(|p| p.id.is_some()) && !self.ours();
        let state = if app.is_none() {
            "missing"
        } else if other {
            "other"
        } else {
            "ok"
        };
        let version_file = plist.as_ref().and_then(|p| p.file.clone()).unwrap_or_else(|| style_of(&at).join(&[&at, "Contents", "Info.plist"]));
        vec![
            json!({ "name": "app", "file": tilde(&at, home), "state": state }),
            json!({ "name": "app's version", "file": tilde(&version_file, home), "state": plist.map_or_else(|| "missing".to_string(), |p| p.state) }),
        ]
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
