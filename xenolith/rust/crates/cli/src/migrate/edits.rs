//! agent-wiki's entries in the apps' files, found and taken out: its managed instruction block,
//! its Codex approval table, its Claude Code allow rules, its personal marketplace entry and its
//! Claude desktop server. The same edits agent-wiki's own uninstaller made (scripts/uninstall.mjs,
//! install/mod.rs), with its names. Each edit keeps the file's line endings and byte-order mark,
//! and refuses a file that is not valid JSON or TOML rather than guessing.

use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const PLUGIN: &str = "agent-wiki";
pub const SERVER: &str = "agent-wiki";
pub const CLAUDE_MARKET: &str = "agent-wiki-local";
pub const CLAUDE_PLUGIN_ID: &str = "agent-wiki@agent-wiki-local";
pub const DEFAULT_CODEX_ID: &str = "agent-wiki@personal";
pub const CLAUDE_ALLOW: [&str; 2] = ["mcp__plugin_agent-wiki_agent-wiki", "mcp__agent-wiki"];
pub const TOML_COMMENT: &str = "# agent-wiki: run the wiki tools without confirmation (managed by the agent-wiki installer)";
static BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n*<!-- AGENT-WIKI:START[^\n]*-->(?s:.*?)<!-- AGENT-WIKI:END -->\n*").unwrap());

const BOM: char = '\u{feff}';

/// A text file as an edit sees it: LF text without a byte-order mark, and how to write it back.
pub struct Text {
    pub lf: String,
    crlf: bool,
    bom: bool,
}

impl Text {
    pub fn parse(raw: &str) -> Text {
        let bom = raw.starts_with(BOM);
        let body = raw.trim_start_matches(BOM);
        Text { lf: body.replace("\r\n", "\n"), crlf: body.contains("\r\n"), bom }
    }
    pub fn render(&self, next_lf: &str) -> String {
        let t = next_lf.replace("\r\n", "\n");
        let t = if self.crlf { t.replace('\n', "\r\n") } else { t };
        if self.bom { format!("{BOM}{t}") } else { t }
    }
}

pub fn read(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read(file) {
        Ok(b) => String::from_utf8(b).map(Some).map_err(|_| format!("{} is not UTF-8 text", fwd(file))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", fwd(file))),
    }
}

/// Writes through a temporary file and a rename, keeping the old file's permissions.
pub fn write_atomic(file: &Path, content: &[u8]) -> Result<(), String> {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", fwd(dir)))?;
    let tmp = PathBuf::from(format!("{}.xenolith-migrate-{}.tmp", file.display(), std::process::id()));
    std::fs::write(&tmp, content).map_err(|e| format!("cannot write {}: {e}", fwd(&tmp)))?;
    if let Ok(m) = std::fs::metadata(file) {
        let _ = std::fs::set_permissions(&tmp, m.permissions());
    }
    let mut i = 0u64;
    loop {
        match std::fs::rename(&tmp, file) {
            Ok(()) => return Ok(()),
            Err(e) if i < 20 && (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5) | Some(32))) => {
                std::thread::sleep(std::time::Duration::from_millis(50 + i * 50));
                i += 1;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("cannot write {}: {e}", fwd(file)));
            }
        }
    }
}

/// What an edit would write: `f` gets the LF text and returns the new LF text, or None for no
/// change. None when nothing would change (or the file is missing, which stays missing).
pub fn plan(file: &Path, f: impl FnOnce(&str) -> Result<Option<String>, String>) -> Result<Option<String>, String> {
    let Some(raw) = read(file)? else { return Ok(None) };
    let t = Text::parse(&raw);
    let Some(next) = f(&t.lf)? else { return Ok(None) };
    let out = t.render(&next);
    Ok((out != raw).then_some(out))
}

/// Read-modify-write (see `plan`). Returns whether the file changed.
pub fn edit(file: &Path, f: impl FnOnce(&str) -> Result<Option<String>, String>) -> Result<bool, String> {
    let Some(out) = plan(file, f)? else { return Ok(false) };
    write_atomic(file, out.as_bytes())?;
    Ok(true)
}

pub fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn parse_json(text: &str, file: &Path) -> Result<Value, String> {
    match serde_json::from_str::<Value>(text.trim_start_matches(BOM)) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err(format!("{} is not a JSON object", fwd(file))),
        Err(e) => Err(format!("{} is not valid JSON ({e})", fwd(file))),
    }
}

pub fn parse_toml(text: &str, file: &Path) -> Result<toml::Table, String> {
    text.trim_start_matches(BOM).parse::<toml::Table>().map_err(|e| format!("{} is not valid TOML ({})", fwd(file), e.to_string().lines().next().unwrap_or("")))
}

pub fn to_json(v: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default())
}

// ---------------------------------------------------------------- the instruction block

pub fn has_block(text: &str) -> bool {
    BLOCK_RE.is_match(text)
}

/// Removes agent-wiki's managed block: None when there is none, "" when nothing else remains.
pub fn remove_block(text: &str) -> Option<String> {
    let m = BLOCK_RE.find(text)?;
    let (before, after) = (text[..m.start()].trim_end(), text[m.end()..].trim());
    let rest: Vec<&str> = [before, after].into_iter().filter(|s| !s.trim().is_empty()).collect();
    Some(if rest.is_empty() { String::new() } else { format!("{}\n", rest.join("\n\n")) })
}

// ---------------------------------------------------------------- Codex config.toml

fn header_re(plugin_id: &str) -> Regex {
    Regex::new(&format!(r#"^\[\s*plugins\s*\.\s*"{}"\s*\.\s*mcp_servers\s*\.\s*"?{}"?\s*\]\s*(#.*)?$"#, regex::escape(plugin_id), regex::escape(SERVER))).unwrap()
}

/// agent-wiki's approval table: plugins."<id>".mcp_servers.agent-wiki.
pub fn toml_has_approval(t: &toml::Table, plugin_id: &str) -> bool {
    t.get("plugins").and_then(|p| p.get(plugin_id)).and_then(|p| p.get("mcp_servers")).and_then(|m| m.get(SERVER)).and_then(|s| s.get("default_tools_approval_mode")).is_some()
}

/// The plugin's own table, which `codex plugin add` writes (`enabled = true`), not just the parent
/// of agent-wiki's approval table.
pub fn toml_has_plugin(t: &toml::Table, plugin_id: &str) -> bool {
    t.get("plugins").and_then(|p| p.get(plugin_id)).and_then(|p| p.as_table()).is_some_and(|p| p.keys().any(|k| k != "mcp_servers"))
}

/// The plugin ids under [plugins] that are agent-wiki's ("agent-wiki@<marketplace>").
pub fn toml_plugin_ids(t: &toml::Table) -> Vec<String> {
    t.get("plugins").and_then(|p| p.as_table()).map(|p| p.keys().filter(|k| k.starts_with("agent-wiki@")).cloned().collect()).unwrap_or_default()
}

/// Removes the approval table (and agent-wiki's comment above it), as agent-wiki's uninstaller did.
pub fn toml_remove_approval(text: &str, plugin_id: &str, file: &Path) -> Result<Option<String>, String> {
    parse_toml(text, file)?;
    let mut lines: Vec<&str> = text.split('\n').collect();
    let re = header_re(plugin_id);
    let Some(h) = lines.iter().position(|l| re.is_match(l.trim())) else { return Ok(None) };
    let start = if h > 0 && lines[h - 1].trim() == TOML_COMMENT { h - 1 } else { h };
    let mut end = h + 1;
    while end < lines.len() && !lines[end].trim_start().starts_with('[') {
        end += 1;
    }
    while end - 1 > h && (lines[end - 1].trim().is_empty() || lines[end - 1].trim_start().starts_with('#')) {
        end -= 1;
    }
    lines.drain(start..end);
    static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
    let out = format!("{}\n", GAPS.replace_all(&lines.join("\n"), "\n\n").trim_end());
    parse_toml(&out, file).map_err(|e| format!("refusing to write: the result would not be valid TOML ({e})"))?;
    Ok(Some(out))
}

// ---------------------------------------------------------------- Claude Code settings.json

pub fn allow_entries(v: &Value) -> Vec<String> {
    v["permissions"]["allow"].as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|s| CLAUDE_ALLOW.contains(s)).map(String::from).collect()).unwrap_or_default()
}

pub fn allow_remove(text: &str, file: &Path) -> Result<Option<String>, String> {
    let mut s = parse_json(text, file)?;
    let Some(perms) = s.get_mut("permissions").and_then(Value::as_object_mut) else { return Ok(None) };
    let Some(allow) = perms.get("allow").and_then(Value::as_array).cloned() else { return Ok(None) };
    if !allow.iter().any(|a| CLAUDE_ALLOW.iter().any(|r| a == r)) {
        return Ok(None);
    }
    let kept: Vec<Value> = allow.into_iter().filter(|a| !CLAUDE_ALLOW.iter().any(|r| a == r)).collect();
    if kept.is_empty() {
        perms.remove("allow");
    } else {
        perms.insert("allow".into(), Value::Array(kept));
    }
    if perms.is_empty() {
        s.as_object_mut().unwrap().remove("permissions");
    }
    Ok(Some(to_json(&s)))
}

/// enabledPlugins["agent-wiki@agent-wiki-local"] in settings.json.
pub fn claude_plugin_enabled(v: &Value) -> bool {
    v["enabledPlugins"].get(CLAUDE_PLUGIN_ID).is_some()
}

// ---------------------------------------------------------------- personal marketplace

pub fn market_has(v: &Value) -> bool {
    v["plugins"].as_array().is_some_and(|l| l.iter().any(|x| x["name"] == PLUGIN))
}

/// Removes the agent-wiki entry: (new text, whether the list is now empty).
pub fn market_remove(text: &str, file: &Path) -> Result<Option<(String, bool)>, String> {
    let mut m = parse_json(text, file)?;
    let Some(list) = m["plugins"].as_array_mut() else { return Ok(None) };
    let before = list.len();
    list.retain(|x| x["name"] != PLUGIN);
    if list.len() == before {
        return Ok(None);
    }
    let emptied = list.is_empty();
    Ok(Some((to_json(&m), emptied)))
}

// ---------------------------------------------------------------- Claude desktop

pub fn desktop_has(v: &Value) -> bool {
    v["mcpServers"].get(SERVER).is_some()
}

pub fn desktop_remove(text: &str, file: &Path) -> Result<Option<String>, String> {
    let mut c = parse_json(text, file)?;
    let Some(servers) = c["mcpServers"].as_object_mut() else { return Ok(None) };
    if servers.remove(SERVER).is_none() {
        return Ok(None);
    }
    if servers.is_empty() {
        c.as_object_mut().unwrap().remove("mcpServers");
    }
    Ok(Some(to_json(&c)))
}

/// A JSON file's own registration of agent-wiki: installed_plugins.json or known_marketplaces.json.
pub fn json_mentions(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(o) => o.contains_key(key) || o.values().any(|x| json_mentions(x, key)),
        Value::Array(a) => a.iter().any(|x| json_mentions(x, key)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_toml_allow_market_desktop() {
        let f = Path::new("x");
        let block = "<!-- AGENT-WIKI:START (managed by the agent-wiki installer; replaced on re-install) -->\nold pointer\n<!-- AGENT-WIKI:END -->";
        let text = format!("# Mine\n\n{block}\n\n<!-- XENOLITH:START (x) -->\nnew\n<!-- XENOLITH:END -->\n");
        assert!(has_block(&text));
        assert_eq!(remove_block(&text).unwrap(), "# Mine\n\n<!-- XENOLITH:START (x) -->\nnew\n<!-- XENOLITH:END -->\n");
        assert_eq!(remove_block(block).unwrap(), "");
        let toml_text = format!("model = \"x\"\n\n{TOML_COMMENT}\n[plugins.\"agent-wiki@personal\".mcp_servers.agent-wiki]\ndefault_tools_approval_mode = \"approve\"\n\n[other]\na = 1\n");
        let t = parse_toml(&toml_text, f).unwrap();
        assert!(toml_has_approval(&t, "agent-wiki@personal"));
        assert_eq!(toml_remove_approval(&toml_text, "agent-wiki@personal", f).unwrap().unwrap(), "model = \"x\"\n\n[other]\na = 1\n");
        assert!(toml_remove_approval("not = = toml", "agent-wiki@personal", f).is_err());
        let s = "{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(ls)\",\n      \"mcp__agent-wiki\"\n    ]\n  }\n}\n";
        assert_eq!(allow_remove(s, f).unwrap().unwrap(), "{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(ls)\"\n    ]\n  }\n}\n");
        let m = r#"{"name":"personal","plugins":[{"name":"agent-wiki"},{"name":"xenolith"}]}"#;
        let (out, emptied) = market_remove(m, f).unwrap().unwrap();
        assert!(!emptied && !out.contains("agent-wiki"));
        let d = r#"{"mcpServers":{"agent-wiki":{"command":"x"}}}"#;
        assert_eq!(desktop_remove(d, f).unwrap().unwrap(), "{}\n");
        assert!(desktop_remove("[", f).is_err());
    }

    #[test]
    fn bom_and_crlf_survive_an_edit() {
        let t = Text::parse("\u{feff}{\r\n  \"a\": 1\r\n}\r\n");
        assert_eq!(t.lf, "{\n  \"a\": 1\n}\n");
        assert_eq!(t.render("{\n  \"b\": 2\n}\n"), "\u{feff}{\r\n  \"b\": 2\r\n}\r\n");
    }
}
