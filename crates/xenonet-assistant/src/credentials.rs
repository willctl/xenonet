//! core/assistant/credentials.js: grant credentials and tombstones on
//! disk, in the instance's state folder under assistant/:
//!
//!   `credentials/<grant>.json`  a grant's credential, written once at your
//!                             approval, readable only by your account, and
//!                             deleted when the grant is revoked; XenoNet
//!                             keeps only its SHA-256
//!   `revoked/<grant>`           a grant revoked, whatever engine.sqlite says
//!   `revisions/<grant>`         a grant's revision now
//!   disabled                  every cloud connection off
//!
//! A credential never goes in a tool's arguments, an environment variable
//! or a log: the adapter reads it here and sends it only over XenoNet's
//! private local connection.

use crate::crypto::{hex, random_bytes, sha256_hex};
use serde_json::{json, Map, Value};
use xenonet_app::fsx::{self, FsError};
use xenonet_app::nodepath::NATIVE;
use xenonet_base::text::js_string;

pub const TOKEN_PREFIX: &str = "xenonet_assistant_";

/// /^xenonet_assistant_[0-9a-f]{64}$/
pub fn is_token(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX).is_some_and(|h| h.len() == 64 && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
}

/// GRANT_ID in core/schema/assistant.js: /^gr_[0-9a-f]{24}$/.
pub fn is_grant(s: &str) -> bool {
    s.strip_prefix("gr_").is_some_and(|h| h.len() == 24 && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
}

pub fn hash_token(token: &str) -> String {
    sha256_hex(token)
}

/// A new credential and the hash XenoNet keeps of it: 256 random bits.
pub fn new_token() -> (String, String) {
    let token = format!("{TOKEN_PREFIX}{}", hex(&random_bytes(32)));
    let hash = hash_token(&token);
    (token, hash)
}

pub fn assistant_dir(state_dir: &str) -> String {
    NATIVE.join(&[state_dir, "assistant"])
}

/// The grant as the file names take it, or "not a grant id".
pub fn check_grant(grant: &Value) -> Result<String, FsError> {
    let g = js_string(grant);
    if is_grant(&g) {
        Ok(g)
    } else {
        Err(fsx::said("not a grant id"))
    }
}

fn credential_file(dir: &str, grant: &str) -> String {
    NATIVE.join(&[dir, "credentials", &format!("{grant}.json")])
}

// fs.writeFileSync(file, data, { flag: "wx", mode }): only a file that
// isn't there yet.
fn write_new(p: &str, data: &[u8], mode: u32) -> Result<(), FsError> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = o.open(p).map_err(|e| FsError::of(e, "open", p))?;
    f.write_all(data).map_err(|e| FsError::of(e, "write", p))
}

/// What keeps a file private, and who else can read it (private-file.js).
pub trait Private {
    fn protect(&mut self, file: &str) -> Result<(), String>;
    fn readers(&mut self, file: &str) -> Result<Vec<String>, String>;
}

/// Writes a grant's credential where only your account can read it, and
/// checks that no one else can; a file that others can read is deleted
/// and the write fails.
pub fn write_credential(dir: &str, grant: &Value, token: &Value, device: Option<&Value>, expires_at: &Value, private: &mut dyn Private) -> Result<String, FsError> {
    let g = check_grant(grant)?;
    let file = credential_file(dir, &g);
    fsx::mkdir_p(&NATIVE.dirname(&file), Some(0o700))?;
    fsx::rm_rf(&file)?;
    let wrote = (|| -> Result<(), String> {
        write_new(&file, b"", 0o600).map_err(|e| e.message)?;
        private.protect(&file)?;
        let mut doc = Map::new();
        doc.insert("schema".into(), json!("assistant/credential/v1"));
        doc.insert("grant_id".into(), grant.clone());
        if let Some(d) = device {
            doc.insert("device_id".into(), d.clone());
        }
        doc.insert("expires_at".into(), expires_at.clone());
        doc.insert("token".into(), token.clone());
        fsx::write_file(&file, format!("{}\n", xenonet_base::json::stringify(&Value::Object(doc), 0)).as_bytes(), None).map_err(|e| e.message)?;
        let others = private.readers(&file)?;
        if !others.is_empty() {
            return Err(format!("others could read it ({})", others.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
        }
        Ok(())
    })();
    if let Err(why) = wrote {
        let _ = fsx::rm_rf(&file);
        return Err(fsx::said(format!("XenoNet couldn't keep the grant's credential private: {why}")));
    }
    Ok(file)
}

/// A grant's credential, { token, device, expiresAt }, or None when there's
/// none (revoked, or never written on this computer).
pub fn read_credential(dir: &str, grant: &Value) -> Option<Value> {
    let g = check_grant(grant).ok()?;
    let c = xenonet_base::json::parse(&fsx::read_text(&credential_file(dir, &g)).ok()?).ok()?;
    let token = c.get("token").and_then(Value::as_str).filter(|t| is_token(t))?;
    if !xenonet_app::jsv::strict_eq(c.get("grant_id"), Some(grant)) {
        return None;
    }
    let expires = c.get("expires_at").and_then(Value::as_f64).filter(|n| n.is_finite());
    Some(json!({ "token": token, "device": c.get("device_id").cloned().unwrap_or(Value::Null), "expiresAt": expires.map_or(Value::Null, xenonet_base::json::number) }))
}

pub fn delete_credential(dir: &str, grant: &Value) -> Result<(), FsError> {
    let g = check_grant(grant)?;
    fsx::rm_rf(&credential_file(dir, &g))
}

/// Keeps a grant's revision, as the service moved it on.
pub fn mark_revision(dir: &str, grant: &Value, revision: &Value) -> Result<(), FsError> {
    let g = check_grant(grant)?;
    let file = NATIVE.join(&[dir, "revisions", &g]);
    fsx::mkdir_p(&NATIVE.dirname(&file), Some(0o700))?;
    fsx::write_file(&file, format!("{}\n", js_string(revision)).as_bytes(), Some(0o600))
}

/// A grant's revision, or None when there's no record of it.
pub fn revision_of(dir: &str, grant: &Value) -> Option<f64> {
    let g = check_grant(grant).ok()?;
    let text = fsx::read_text(&NATIVE.join(&[dir, "revisions", &g])).ok()?;
    let n = xenonet_base::text::js_number(Some(&Value::String(xenonet_base::text::js_trim(&text).to_string())));
    (n.fract() == 0.0 && n > 0.0 && n.is_finite()).then_some(n)
}

// ---------- tombstones ----------

pub fn mark_revoked(dir: &str, grant: &Value, at: f64) -> Result<(), FsError> {
    let g = check_grant(grant)?;
    let file = NATIVE.join(&[dir, "revoked", &g]);
    fsx::mkdir_p(&NATIVE.dirname(&file), Some(0o700))?;
    match write_new(&file, format!("{}\n", xenonet_base::json::stringify(&json!({ "at": xenonet_base::json::number(at) }), 0)).as_bytes(), 0o600) {
        Err(e) if !e.is("EEXIST") => Err(e),
        _ => Ok(()),
    }
}

pub fn is_marked_revoked(dir: &str, grant: &Value) -> bool {
    let g = js_string(grant);
    is_grant(&g) && fsx::exists(&NATIVE.join(&[dir, "revoked", &g]))
}

pub fn marked_revoked(dir: &str) -> Vec<String> {
    fsx::read_dir_names(&NATIVE.join(&[dir, "revoked"])).map(|names| names.into_iter().filter(|g| is_grant(g)).collect()).unwrap_or_default()
}

pub fn mark_disabled(dir: &str, at: f64) -> Result<(), FsError> {
    fsx::mkdir_p(dir, Some(0o700))?;
    fsx::write_file(&NATIVE.join(&[dir, "disabled"]), format!("{}\n", xenonet_base::json::stringify(&json!({ "at": xenonet_base::json::number(at) }), 0)).as_bytes(), Some(0o600))
}

pub fn is_disabled(dir: &str) -> bool {
    fsx::exists(&NATIVE.join(&[dir, "disabled"]))
}

pub fn clear_disabled(dir: &str) -> Result<(), FsError> {
    fsx::rm_rf(&NATIVE.join(&[dir, "disabled"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fine(Vec<String>);
    impl Private for Fine {
        fn protect(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn readers(&mut self, _: &str) -> Result<Vec<String>, String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn a_credential_only_its_owner_can_read() {
        let dir = fsx::mkdtemp(&format!("{}/xn-cred-", std::env::temp_dir().display())).unwrap();
        let grant = json!("gr_0123456789abcdef01234567");
        let (token, hash) = new_token();
        assert!(is_token(&token) && hash.len() == 64);
        let file = write_credential(&dir, &grant, &json!(token), Some(&json!("dev_x")), &Value::Null, &mut Fine(vec![])).unwrap();
        assert_eq!(read_credential(&dir, &grant).unwrap()["token"], json!(token));
        let err = write_credential(&dir, &grant, &json!(token), None, &Value::Null, &mut Fine(vec!["Everyone".into()])).unwrap_err();
        assert_eq!(err.message, "XenoNet couldn't keep the grant's credential private: others could read it (Everyone)");
        assert!(!fsx::exists(&file), "the file others could read is gone");
        assert!(check_grant(&json!("gr_../x")).is_err());
        mark_revoked(&dir, &grant, 5.0).unwrap();
        mark_revoked(&dir, &grant, 6.0).unwrap();
        assert_eq!(marked_revoked(&dir), vec!["gr_0123456789abcdef01234567".to_string()]);
        mark_revision(&dir, &grant, &json!(3)).unwrap();
        assert_eq!(revision_of(&dir, &grant), Some(3.0));
        fsx::rm_rf(&dir).unwrap();
    }
}
