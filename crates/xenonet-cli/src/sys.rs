//! What the command asks of this machine as Node's `os`, `process` and
//! core/dirs.js tell it: the home folder, the platform, XenoNet's own
//! folders and where the command runs.

use xenonet_base::paths;

/// An environment variable, unless it's empty (`process.env.X || …`).
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// os.homedir(): HOME (USERPROFILE on Windows).
pub fn home() -> String {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(name).unwrap_or_default()
}

/// process.platform.
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// path.sep.
pub const SEP: &str = if cfg!(windows) { "\\" } else { "/" };

/// appDir(kind, { name }): XenoNet's (or another instance's) own folder of
/// a kind, "state" or "cache".
pub fn app_dir(kind: &str, name: &str) -> String {
    let xdg = std::env::var(if kind == "state" { "XDG_STATE_HOME" } else { "XDG_CACHE_HOME" }).ok();
    let local = std::env::var("LOCALAPPDATA").ok();
    xenonet_base::dirs::app_dir(kind, xdg.as_deref(), local.as_deref(), platform(), &home(), name)
}

/// XENONET_STATE_DIR, or the state folder of XenoNet (or XenoNet Dev, with
/// `dev`), as the commands that read it from disk find it.
pub fn state_dir(dev: bool) -> String {
    var("XENONET_STATE_DIR").unwrap_or_else(|| app_dir("state", if dev { "XenoNet Dev" } else { "XenoNet" }))
}

/// process.cwd().
pub fn cwd() -> String {
    std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

/// path.resolve(p).
pub fn resolve(p: &str) -> String {
    paths::resolve(p)
}

/// path.basename(p).
pub fn base_name(p: &str) -> String {
    let trimmed = p.trim_end_matches(|c| c == '/' || (cfg!(windows) && c == '\\'));
    let cut = trimmed.rfind(|c| c == '/' || (cfg!(windows) && c == '\\')).map_or(trimmed, |i| &trimmed[i + 1..]);
    cut.to_string()
}

/// path.join(a, b).
pub fn join(a: &str, b: &str) -> String {
    paths::join(a, b)
}

/// The tilde form of a path under the home folder.
pub fn tilde(p: &str) -> String {
    paths::tilde(p, &home())
}

/// XenoNet's own folder, beside the command: the first of the folders the
/// binary is in, and those above it, with XenoNet's package.json (a
/// checkout's, for target/debug/xenonet), else the binary's own folder. It
/// stands for the JS's ROOT, the folder its code is in.
pub fn xenonet_root() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors()
        .skip(1)
        .take(5)
        .find(|d| d.join("package.json").is_file() && d.join("cli").is_dir())
        .or_else(|| exe.parent())
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether this is installed Tools' command, whose console operator drives
/// installed XenoNet only: as it was built (package.json's
/// xenonetComponent), the Tools package's binary (the package.json beside
/// it says so), or the launcher's copy of that binary, which hooks and MCP
/// entries run and which has a `.from` file beside it naming the Tools it
/// was copied from (docs/design/contracts.md, section 12). One binary goes
/// in a checkout, the Service and the Tools.
pub fn installed_tools() -> bool {
    if crate::commands::console::INSTALLED {
        return true;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if exe.file_name().is_some_and(|n| exe.with_file_name(format!("{}.from", n.to_string_lossy())).is_file()) {
        return true;
    }
    std::fs::read_to_string(std::path::Path::new(&xenonet_root()).join("package.json"))
        .ok()
        .and_then(|t| xenonet_base::json::parse(&t).ok())
        .is_some_and(|p| p.get("xenonetComponent").and_then(serde_json::Value::as_str) == Some("tools"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_as_path_has_them() {
        assert_eq!(base_name("/w/atlas/src/refunds.ts"), "refunds.ts");
        assert_eq!(base_name("/w/atlas/"), "atlas");
        assert_eq!(base_name("refunds.ts"), "refunds.ts");
        assert_eq!(base_name(""), "");
    }
}
