//! XenoNet's plugin for Hermes Agent (core/actors/hermes-plugin.js,
//! plugins/hermes/xenonet): putting it in $HERMES_HOME/plugins/xenonet,
//! taking it out, and saying whether it's installed, turned on and
//! reporting. `xenonet connect hermes` and the doctor use this.
//!
//! XenoNet only copies the plugin's two files. It never edits Hermes's
//! config.yaml (plugins are off until their name is in plugins.enabled,
//! which is the owner's step) and never makes a token: the owner's token
//! goes in the plugin's `token` file.

use std::path::{Path, PathBuf};

/// The plugin's files, as this XenoNet ships them.
pub const FILES: [(&str, &str); 2] =
    [("plugin.yaml", include_str!("../../../plugins/hermes/xenonet/plugin.yaml")), ("__init__.py", include_str!("../../../plugins/hermes/xenonet/__init__.py"))];

/// A report older than this means the plugin isn't running (it beats every 15 s).
pub const STALE_MS: f64 = 120_000.0;

/// Hermes's root folder: HERMES_HOME when given, else ~/.hermes.
pub fn root(home: &str, hermes_home: Option<&str>) -> PathBuf {
    match hermes_home.filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => Path::new(home).join(".hermes"),
    }
}

pub fn dir(home: &str, hermes_home: Option<&str>) -> PathBuf {
    root(home, hermes_home).join("plugins").join("xenonet")
}

fn bare(s: &str) -> &str {
    s.trim().trim_matches(['"', '\''])
}

/// Whether config.yaml's plugins.enabled lists xenonet, in either YAML form:
/// `enabled: [a, xenonet]` or a block of `- xenonet` lines.
pub fn enabled_in(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines.iter().position(|l| l.trim_end() == "plugins:" || (l.starts_with("plugins:") && l["plugins:".len()..].trim_start().starts_with('#'))) else {
        return false;
    };
    for (i, l) in lines.iter().enumerate().skip(at + 1) {
        // A comment at column 0 doesn't end the block: Hermes's own config
        // has them inside `plugins:`.
        if l.starts_with('#') {
            continue;
        }
        if l.chars().next().is_some_and(|c| !c.is_whitespace()) {
            return false;
        }
        let indent = l.len() - l.trim_start().len();
        let Some(rest) = l.trim_start().strip_prefix("enabled:") else { continue };
        let rest = rest.split(" #").next().unwrap_or("").trim();
        if let Some(inner) = rest.strip_prefix('[') {
            return inner.trim_end_matches(']').split(',').any(|n| bare(n) == "xenonet");
        }
        for item in &lines[i + 1..] {
            let t = item.trim_start();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let Some(v) = t.strip_prefix('-') else { return false };
            if item.len() - t.len() < indent {
                return false;
            }
            if bare(v.split(" #").next().unwrap_or("")) == "xenonet" {
                return true;
            }
        }
        return false;
    }
    false
}

/// What's there.
#[derive(Debug, PartialEq)]
pub struct State {
    pub dir: PathBuf,
    pub installed: bool,
    /// The files are the ones this XenoNet ships.
    pub current: bool,
    pub enabled: bool,
    /// "none", "loose" (others can read it: the plugin refuses it) or "ok".
    pub token: &'static str,
    /// The plugin's last heartbeat: whether it was answered, and how long ago.
    pub reported: Option<(bool, f64)>,
}

#[cfg(unix)]
fn others_read(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o077 != 0
}
#[cfg(not(unix))]
fn others_read(_: &std::fs::Metadata) -> bool {
    false
}

pub fn inspect(home: &str, hermes_home: Option<&str>, now_ms: f64) -> State {
    let d = dir(home, hermes_home);
    let installed = FILES.iter().all(|(n, _)| d.join(n).exists());
    let current = installed && FILES.iter().all(|(n, src)| std::fs::read_to_string(d.join(n)).is_ok_and(|t| t == *src));
    let token = match std::fs::metadata(d.join("token")) {
        Ok(m) if m.is_file() => {
            if others_read(&m) {
                "loose"
            } else {
                "ok"
            }
        }
        _ => "none",
    };
    let reported = std::fs::read_to_string(d.join("state.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| {
        let at = v.get("at")?.as_f64()?;
        Some((v.get("ok") == Some(&serde_json::Value::Bool(true)), (now_ms - at).max(0.0)))
    });
    let enabled = std::fs::read_to_string(root(home, hermes_home).join("config.yaml")).is_ok_and(|t| enabled_in(&t));
    State { dir: d, installed, current, enabled, token, reported }
}

pub fn install(home: &str, hermes_home: Option<&str>) -> std::io::Result<PathBuf> {
    let d = dir(home, hermes_home);
    std::fs::create_dir_all(&d)?;
    for (n, src) in FILES {
        std::fs::write(d.join(n), src)?;
    }
    Ok(d)
}

/// Takes the plugin's folder out, token and state included; config.yaml
/// stays as it is. Whether there was a folder.
pub fn uninstall(home: &str, hermes_home: Option<&str>) -> std::io::Result<bool> {
    let d = dir(home, hermes_home);
    let was = d.exists();
    if was {
        std::fs::remove_dir_all(&d)?;
    }
    Ok(was)
}

/// What the owner still does, given `inspect`'s answer; `shown` writes a path.
pub fn owner_steps(st: &State, shown: &dyn Fn(&Path) -> String) -> Vec<String> {
    let mut steps = Vec::new();
    if st.token != "ok" {
        let loose = if st.token == "loose" { " The file there is readable by others, so the plugin ignores it: chmod 600 it." } else { "" };
        steps.push(format!(
            "Give the plugin a token: in XenoNet's Actors panel, add an actor for Hermes Agent and save its token to {} (only you can read it).{loose}",
            shown(&st.dir.join("token"))
        ));
    }
    if !st.enabled {
        steps.push("Turn the plugin on, which Hermes leaves to you: hermes plugins enable xenonet, or add xenonet to plugins.enabled in config.yaml. It takes effect with the next Hermes session.".into());
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xn-hp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn enabled_in_reads_both_forms_and_nothing_else() {
        assert!(enabled_in("plugins:\n  enabled:\n    - other\n    - xenonet\n"));
        assert!(enabled_in("plugins:\n  enabled: [other, 'xenonet']\n"));
        assert!(!enabled_in("plugins:\n  enabled: []\n"));
        assert!(!enabled_in("plugins:\n  enabled:\n    - other\nmodel: xenonet\n"));
        assert!(!enabled_in("other:\n  enabled:\n    - xenonet\n"));
        assert!(!enabled_in("plugins:\n  disabled:\n    - xenonet\n"));
        assert!(!enabled_in(""));
        // Hermes's own config puts column-0 comments inside the block.
        assert!(enabled_in("plugins:\n  clone: 1\n\n# ===\n# Model\n# ===\n  enabled:\n    - xenonet\n  disabled: []\nmodel:\n"));
        assert!(enabled_in("plugins:\n  enabled:\n    - other\n# note\n    - xenonet\n"));
    }

    #[test]
    fn install_inspect_uninstall_in_a_temp_hermes_home() {
        let h = temp("a");
        let hh = Some(h.as_str());
        std::fs::write(Path::new(&h).join("config.yaml"), "model: x\n").unwrap();
        let st = inspect("/nowhere", hh, 1000.0);
        assert!(!st.installed);
        let d = install("/nowhere", hh).unwrap();
        assert_eq!(d, Path::new(&h).join("plugins/xenonet"));
        assert_eq!(std::fs::read_to_string(Path::new(&h).join("config.yaml")).unwrap(), "model: x\n");
        let st = inspect("/nowhere", hh, 1000.0);
        assert_eq!((st.installed, st.current, st.enabled, st.token, st.reported), (true, true, false, "none", None));
        assert_eq!(owner_steps(&st, &|p| p.display().to_string()).len(), 2);
        std::fs::write(d.join("state.json"), r#"{"ok":true,"at":400}"#).unwrap();
        assert_eq!(inspect("/nowhere", hh, 1000.0).reported, Some((true, 600.0)));
        std::fs::write(d.join("__init__.py"), "# edited").unwrap();
        assert!(!inspect("/nowhere", hh, 0.0).current);
        assert!(uninstall("/nowhere", hh).unwrap());
        assert!(!uninstall("/nowhere", hh).unwrap());
        let _ = std::fs::remove_dir_all(&h);
    }

    #[cfg(unix)]
    #[test]
    fn a_loose_token_file_is_called_loose() {
        use std::os::unix::fs::PermissionsExt;
        let h = temp("b");
        let hh = Some(h.as_str());
        let d = install("/nowhere", hh).unwrap();
        std::fs::write(d.join("token"), "xenonet_actor_x").unwrap();
        std::fs::set_permissions(d.join("token"), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(inspect("/nowhere", hh, 0.0).token, "loose");
        std::fs::set_permissions(d.join("token"), std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(inspect("/nowhere", hh, 0.0).token, "ok");
        let _ = std::fs::remove_dir_all(&h);
    }
}
