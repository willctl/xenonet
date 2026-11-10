//! What the map reads of Claude Code besides its transcripts' lines: the
//! account it's signed in to (~/.claude.json's oauthAccount), and where a
//! session's transcript files are.

use serde_json::{json, Value};
use std::path::Path;
use xenonet_adapter::platform::{SessionFiles, Signin};
use xenonet_adapter::reader::VendorCheck;
use xenonet_os::paths;
use xenonet_sources::lines::{opt_prop, Js};
use xenonet_text::text::{display, trim};

/// `typeof v === "string" && v.trim() ? v.trim() : null`
fn str_of(v: Js<'_>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !trim(s).is_empty() => Some(trim(s).to_string()),
        _ => None,
    }
}

fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

fn tpl(v: Js<'_>) -> String {
    v.map_or_else(|| "undefined".into(), display)
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::String)
}

fn plan_of(t: Js<'_>) -> Value {
    match t.and_then(Value::as_str) {
        Some("claude_max") => "Max".into(),
        Some("claude_pro") => "Pro".into(),
        Some("claude_team") => "Team".into(),
        Some("claude_enterprise") => "Enterprise".into(),
        _ => Value::Null,
    }
}

/// The account Claude Code is signed in to, from ~/.claude.json's
/// oauthAccount. Never the name fields.
pub fn claude_account(data: Js<'_>) -> Value {
    let a = opt_prop(data, "oauthAccount");
    let get = |k: &str| opt_prop(a, k);
    if str_of(get("accountUuid")).is_none() || str_of(get("organizationUuid")).is_none() {
        return Value::Null;
    }
    let ty = nn(get("organizationType")).map_or_else(String::new, display).to_ascii_lowercase();
    let team = ty.contains("team") || ty.contains("enterprise");
    json!({
        "id": format!("claude:{}:{}", tpl(get("accountUuid")), tpl(get("organizationUuid"))),
        "kind": "claude",
        "handle": opt_str(str_of(get("emailAddress"))),
        // A personal organization is named after the email, so only a team's name says anything.
        "detail": { "plan": plan_of(get("organizationType")), "org": if team { opt_str(str_of(get("organizationName"))) } else { Value::Null } },
    })
}

// ~/.claude.json, checked leniently against its schema.
fn read_account(data: &Value, vendor: &dyn VendorCheck) -> Value {
    claude_account(vendor.check("anthropic/claude-code/config", data.clone(), None, None).as_ref())
}

/// Where Claude Code keeps its sign-in: the file the map's
/// claudeConfigFile names. Its transcripts name the account each session
/// ran as.
pub fn signin() -> Signin {
    Signin { setting: "claudeConfigFile", file: None, read: read_account, aliases: &[], provider: None, owners: true }
}

fn dir_names(dir: &str) -> Option<Vec<(String, bool)>> {
    let ents = paths::read_dir_sorted(Path::new(dir)).ok()?;
    Some(ents.into_iter().map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir()))).collect())
}

/// A session's transcript in its project's folder, and its subagents' in
/// `<session>/subagents`.
pub struct Files;

impl SessionFiles for Files {
    fn files(&mut self, dirs: &[String], raw: &str, _now: f64) -> Vec<(String, Option<String>)> {
        let root = dirs.first().map_or("", String::as_str);
        for (name, is_dir) in dir_names(root).unwrap_or_default() {
            if !is_dir {
                continue;
            }
            let main = paths::join_all(&[root, &name, &format!("{raw}.jsonl")]);
            if !Path::new(&main).exists() {
                continue;
            }
            let sub_dir = paths::join_all(&[root, &name, raw, "subagents"]);
            let mut out = vec![(main, None)];
            for (f, _) in dir_names(&sub_dir).unwrap_or_default() {
                if let Some(stem) = f.strip_suffix(".jsonl") {
                    let agent = stem.strip_prefix("agent-").unwrap_or(stem).to_string();
                    out.push((paths::join(&sub_dir, &f), Some(agent)));
                }
            }
            return out;
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_its_sign_in() {
        let a = claude_account(Some(
            &json!({ "oauthAccount": { "accountUuid": "a", "organizationUuid": "o", "emailAddress": " n@x ", "organizationType": "claude_team", "organizationName": "Atlas" } }),
        ));
        assert_eq!(a, json!({ "id": "claude:a:o", "kind": "claude", "handle": "n@x", "detail": { "plan": "Team", "org": "Atlas" } }));
        assert_eq!(claude_account(Some(&json!({}))), Value::Null);
    }
}
