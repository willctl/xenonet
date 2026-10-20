//! core/command.js: the xenonet command on your PATH, a small script
//! (~/.local/bin/xenonet, or xenonet.cmd in the app's local folder on
//! Windows) that runs the xenonet binary. One an earlier XenoNet wrote ran
//! a node and cli/xenonet.js; it's read as XenoNet's too. XenoNet's script
//! is told apart from another program's by its marker line, and only
//! XenoNet's is replaced or taken out.

use crate::fsx::{self, FsError};
use crate::nodepath::{Style, NATIVE};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub const MARKER: &str = "Added by XenoNet";

/// A path as sh reads it in single quotes.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A path as cmd.exe reads it in double quotes, with % doubled. A quote or
/// a line break can't be written safely, so it's refused.
pub fn batch_quote(s: &str) -> Result<String, String> {
    if s.contains(['"', '\r', '\n', '\0']) {
        return Err("a command path can't contain a quote or a line break".into());
    }
    Ok(format!("\"{}\"", s.replace('%', "%%")))
}

/// Where the command goes: the app's standard local data folder on
/// Windows, ~/.local/bin elsewhere (joined as this machine joins paths).
pub fn command_dir(home: &str, platform: &str, local_app_data: Option<&str>) -> String {
    if platform == "win32" {
        let local = local_app_data.map_or_else(|| NATIVE.join(&[home, "AppData", "Local"]), String::from);
        NATIVE.join(&[&local, "XenoNet", "bin"])
    } else {
        NATIVE.join(&[home, ".local", "bin"])
    }
}

/// The script itself: it runs `bin`, the xenonet binary.
pub fn command_script(bin: &str, platform: &str) -> Result<String, String> {
    if platform == "win32" {
        return Ok(format!("@echo off\n:: {MARKER}: the xenonet command. xenonet uninstall takes it out.\nsetlocal DisableDelayedExpansion\n{} %*\n", batch_quote(bin)?));
    }
    Ok(format!("#!/bin/sh\n# {MARKER}: the xenonet command. xenonet uninstall takes it out.\nexec {} \"$@\"\n", quote(bin)))
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub file: Option<String>,
    pub installed: bool,
    pub ours: bool,
    pub runs: Option<Vec<String>>,
    pub broken: bool,
    pub on_path: bool,
}

// What a script runs: the binary, or an earlier one's node and its entry.
fn batch_runs() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?mR)^"([^"]+)"(?: "([^"]+)")? %\*$"#).expect("a fixed pattern"))
}

fn sh_runs() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"exec '((?:[^']|'\\'')+)'(?: '((?:[^']|'\\'')+)')? "\$@""#).expect("a fixed pattern"))
}

/// What's there now: whether there's a xenonet, whether it's XenoNet's,
/// what it runs (and whether those files still exist), and whether its
/// folder is on PATH.
pub fn command_state(dir: Option<&str>, path_var: Option<&str>, platform: &str) -> State {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return State { file: None, installed: false, ours: false, runs: None, broken: false, on_path: false };
    };
    let file = NATIVE.join(&[dir, if platform == "win32" { "xenonet.cmd" } else { "xenonet" }]);
    let text = std::fs::read(&file).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    let runs = text.as_deref().and_then(|t| {
        let (re, unquote): (&Regex, fn(&str) -> String) = if platform == "win32" { (batch_runs(), |s| s.replace("%%", "%")) } else { (sh_runs(), |s| s.replace("'\\''", "'")) };
        re.captures(t).map(|c| c.iter().skip(1).flatten().map(|m| unquote(m.as_str())).collect::<Vec<String>>())
    });
    let on_path = path_var.unwrap_or("").split(NATIVE.delimiter()).any(|d| !d.is_empty() && same_place(d, dir, platform));
    let broken = runs.as_ref().is_some_and(|r| r.iter().any(|f| !fsx::exists(f)));
    State { ours: text.as_deref().is_some_and(|t| t.contains(MARKER)), installed: text.is_some(), file: Some(file), runs, broken, on_path }
}

/// Whether two folders are the same place: on Windows, paths differ only
/// in case and a trailing backslash.
pub fn same_place(a: &str, b: &str, platform: &str) -> bool {
    if platform != "win32" {
        return NATIVE.resolve(&[a]) == NATIVE.resolve(&[b]);
    }
    let norm = |p: &str| Style::Win32.resolve(&[p]).trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// The registry keys Path is kept under, the machine's and then the
/// user's, as a terminal opened now gets them.
pub const PATH_KEYS: [&str; 2] = ["HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", "HKCU\\Environment"];

/// reg.exe, under SystemRoot.
pub fn reg_exe(env: &Map<String, Value>) -> String {
    let root = env_value(env, "SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Style::Win32.join(&[&root, "System32", "reg.exe"])
}

/// reg query's arguments for one key's Path.
pub fn reg_query(key: &str) -> Vec<String> {
    vec!["query".into(), key.into(), "/v".into(), "Path".into()]
}

fn reg_line() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?imR)^\s+Path\s+REG_(?:EXPAND_)?SZ\s+([^\r\n\u2028\u2029]*?)\s*$").expect("a fixed pattern"))
}

fn var_ref() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"%([^%]+)%").expect("a fixed pattern"))
}

/// reg query's line for Path, split into folders with %VARS% filled in.
pub fn reg_path(stdout: &str, env: &Map<String, Value>) -> Vec<String> {
    let Some(value) = reg_line().captures(stdout).map(|c| c[1].to_string()).filter(|v| !v.is_empty()) else { return Vec::new() };
    value
        .split(';')
        .map(|d| {
            let filled = var_ref().replace_all(d, |c: &regex::Captures<'_>| env_value(env, &c[1]).unwrap_or_else(|| c[0].to_string()));
            xenonet_base::text::js_trim(&filled).to_string()
        })
        .filter(|d| !d.is_empty())
        .collect()
}

/// An environment variable as Windows reads its name, whatever its case:
/// the exact name first, then the first that matches without case.
pub fn env_value(env: &Map<String, Value>, name: &str) -> Option<String> {
    let text = |v: &Value| match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(xenonet_base::text::js_string(other)),
    };
    if let Some(v) = env.get(name).and_then(text) {
        return Some(v);
    }
    let lower = name.to_lowercase();
    env.iter().find(|(k, _)| k.to_lowercase() == lower).and_then(|(_, v)| text(v))
}

/// Writes XenoNet's script, which runs `bin`, through a temporary file
/// renamed into place. Another program's xenonet is left alone unless
/// `force`.
pub fn install_command(dir: &str, bin: &str, force: bool, path_var: Option<&str>, platform: &str) -> Result<State, FsError> {
    let before = command_state(Some(dir), path_var, platform);
    let file = before.file.clone().unwrap_or_default();
    if before.installed && !before.ours && !force {
        return Err(fsx::said(format!("{file} is already there, and it isn't XenoNet's")));
    }
    fsx::mkdir_p(dir, None)?;
    let tmp = format!("{file}.tmp-xenonet");
    let script = command_script(bin, platform).map_err(fsx::said)?;
    fsx::write_file(&tmp, script.as_bytes(), Some(0o755))?;
    fsx::rename(&tmp, &file)?;
    Ok(command_state(Some(dir), path_var, platform))
}

/// Takes XenoNet's script out; another program's is left alone.
pub fn uninstall_command(dir: &str, path_var: Option<&str>, platform: &str) -> Result<State, FsError> {
    let st = command_state(Some(dir), path_var, platform);
    if !st.installed {
        return Ok(st);
    }
    let file = st.file.clone().unwrap_or_default();
    if !st.ours {
        return Err(fsx::said(format!("{file} isn't XenoNet's, so XenoNet leaves it alone")));
    }
    fsx::unlink(&file)?;
    Ok(command_state(Some(dir), path_var, platform))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scripts_quote_what_they_run() {
        let sh = command_script("/opt/xenonet's/bin/xenonet", "linux").unwrap();
        assert_eq!(sh, "#!/bin/sh\n# Added by XenoNet: the xenonet command. xenonet uninstall takes it out.\nexec '/opt/xenonet'\\''s/bin/xenonet' \"$@\"\n");
        assert!(command_script("C:\\a\"b", "win32").is_err());
        assert!(command_script("C:\\100%\\xenonet.exe", "win32").unwrap().ends_with("\n\"C:\\100%%\\xenonet.exe\" %*\n"));
    }

    #[test]
    fn an_earlier_xenonets_script_is_still_its_own() {
        let dir = std::env::temp_dir().join(format!("xenonet-command-earlier-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        std::fs::write(dir.join("xenonet"), "#!/bin/sh\n# Added by XenoNet: the xenonet command.\nELECTRON_RUN_AS_NODE=1 exec '/n' '/e.js' \"$@\"\n").unwrap();
        let st = command_state(Some(&d), None, "linux");
        assert!(st.ours && st.broken);
        assert_eq!(st.runs, Some(vec!["/n".to_string(), "/e.js".to_string()]));
        std::fs::write(dir.join("xenonet.cmd"), "@echo off\r\n:: Added by XenoNet\r\n\"C:\\n.exe\" \"C:\\e.js\" %*\r\n").unwrap();
        assert_eq!(command_state(Some(&d), None, "win32").runs, Some(vec!["C:\\n.exe".to_string(), "C:\\e.js".to_string()]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installs_and_takes_out_only_its_own() {
        let dir = std::env::temp_dir().join(format!("xenonet-command-{}", std::process::id()));
        let d = dir.to_string_lossy().to_string();
        let st = install_command(&d, "/b'x/xenonet", false, Some(&d), "linux").unwrap();
        assert!(st.installed && st.ours && st.on_path && st.broken);
        assert_eq!(st.runs, Some(vec!["/b'x/xenonet".to_string()]));
        assert!(!uninstall_command(&d, None, "linux").unwrap().installed);
        std::fs::write(dir.join("xenonet"), "#!/bin/sh\necho other\n").unwrap();
        assert!(install_command(&d, "/b", false, None, "linux").unwrap_err().message.ends_with("isn't XenoNet's"));
        assert!(uninstall_command(&d, None, "linux").unwrap_err().message.ends_with("leaves it alone"));
        assert!(install_command(&d, "/b", true, None, "linux").unwrap().ours);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn windows_paths_from_the_registry() {
        let env: Map<String, Value> = json!({ "USERPROFILE": "C:\\Users\\ada", "systemroot": "C:\\Windows" }).as_object().unwrap().clone();
        assert_eq!(
            reg_path("\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    %UserProfile%\\.local\\bin;;%NOT_SET%\\x\r\n\r\n", &env),
            ["C:\\Users\\ada\\.local\\bin", "%NOT_SET%\\x"]
        );
        assert_eq!(reg_path("    Path    REG_SZ    C:\\a;;C:\\b\r\n", &Map::new()), ["C:\\a", "C:\\b"]);
        assert_eq!(reg_exe(&env), "C:\\Windows\\System32\\reg.exe");
        assert!(same_place("C:\\Users\\Ada\\XenoNet\\bin\\", "c:\\users\\ada\\xenonet\\bin", "win32"));
        assert!(!same_place("C:\\Users\\Ada\\XenoNet\\bin2", "C:\\Users\\Ada\\XenoNet\\bin", "win32"));
    }
}
