//! core/command.js for the command: the xenonet command on your PATH, a
//! small script XenoNet writes, told apart from another program's by its
//! marker line (xenonet-app's command). The launcher's script runs its node
//! and cli/xenonet.js, as the JS writes it; this binary's runs the binary
//! itself, which has no entry script to name. Both are XenoNet's, so
//! either is replaced, and taken out, by the other.

use super::fs_fail;
use crate::out::Fail;
use crate::sys;
use serde_json::{json, Value};
use xenonet_app::command::{self, State, MARKER};
use xenonet_app::fsx;

fn path_var() -> Option<String> {
    std::env::var("PATH").ok()
}

fn state_json(st: &State) -> Value {
    json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
}

/// What a script for the binary alone runs, when it's one: the binary.
fn binary_of(text: &str) -> Option<String> {
    if !text.contains(MARKER) {
        return None;
    }
    let line = text.lines().find(|l| l.starts_with("exec ") || l.starts_with('"'))?;
    if let Some(rest) = line.strip_prefix("exec '") {
        // exec '<binary>' "$@", the quote in it written '\''.
        let quoted = rest.strip_suffix("' \"$@\"")?;
        return (!quoted.contains("' '")).then(|| quoted.replace("'\\''", "'"));
    }
    // "<binary>" %*
    let quoted = line.strip_prefix('"')?.strip_suffix("\" %*")?;
    (!quoted.contains('"')).then(|| quoted.replace("%%", "%"))
}

/// commandState({ dir }), with what this binary's own script runs.
pub fn state(dir: &str) -> Value {
    let mut st = command::command_state(Some(dir), path_var().as_deref(), sys::platform());
    if st.runs.is_none() && st.ours {
        if let Some(bin) = st.file.as_deref().and_then(|f| std::fs::read(f).ok()).and_then(|b| binary_of(&String::from_utf8_lossy(&b))) {
            st.broken = !fsx::exists(&bin);
            st.runs = Some(vec![bin]);
        }
    }
    state_json(&st)
}

/// The script that runs this binary.
fn binary_script(binary: &str) -> Result<String, String> {
    if sys::platform() == "win32" {
        if binary.contains('"') {
            return Err(format!("{binary} has a quote in it, which the command's script can't hold"));
        }
        return Ok(format!(
            "@echo off\n:: {MARKER}: the xenonet command. xenonet uninstall takes it out.\nsetlocal DisableDelayedExpansion\n\"{}\" %*\n",
            binary.replace('%', "%%")
        ));
    }
    Ok(format!("#!/bin/sh\n# {MARKER}: the xenonet command. xenonet uninstall takes it out.\nexec {} \"$@\"\n", command::quote(binary)))
}

/// installCommand(options): { dir, runtime, entry, runAsNode, force } for
/// a script that runs a node and its entry, or { dir, entry, force } for
/// one that runs `entry`, this binary.
pub fn install(opts: &Value) -> Result<Value, Fail> {
    let t = |k: &str| opts.get(k).filter(|v| !v.is_null()).map(xenonet_base::text::js_string);
    let on = |k: &str| xenonet_base::text::truthy(opts.get(k));
    let dir = match t("dir") {
        Some(d) => d,
        None => super::command_dir()?,
    };
    let entry = t("entry").unwrap_or_default();
    let platform = sys::platform();
    if let Some(runtime) = t("runtime") {
        let st = command::install_command(&dir, &runtime, &entry, on("runAsNode"), on("force"), path_var().as_deref(), platform).map_err(fs_fail)?;
        return Ok(state_json(&st));
    }
    let before = command::command_state(Some(&dir), path_var().as_deref(), platform);
    let file = before.file.clone().unwrap_or_default();
    if before.installed && !before.ours && !on("force") {
        return Err(Fail::error(format!("{file} is already there, and it isn't XenoNet's")));
    }
    fsx::mkdir_p(&dir, None).map_err(fs_fail)?;
    let tmp = format!("{file}.tmp-xenonet");
    let script = binary_script(&entry).map_err(Fail::error)?;
    fsx::write_file(&tmp, script.as_bytes(), Some(0o755)).map_err(fs_fail)?;
    fsx::rename(&tmp, &file).map_err(fs_fail)?;
    Ok(state(&dir))
}

/// uninstallCommand({ dir })
pub fn uninstall(dir: &str) -> Result<Value, Fail> {
    let st = command::uninstall_command(dir, path_var().as_deref(), sys::platform()).map_err(fs_fail)?;
    Ok(state_json(&st))
}

#[cfg(test)]
mod tests {
    use super::*;

    // test/cli.test.js's install case, for the binary's own script: it's
    // XenoNet's, runs the binary, replaces only XenoNet's and goes again.
    #[test]
    fn writes_a_script_that_runs_the_binary_and_takes_it_out() {
        if cfg!(windows) {
            return;
        }
        let root = std::env::temp_dir().join(format!("xenonet-cli-command-{}", std::process::id()));
        let dir = root.join("bin").to_string_lossy().into_owned();
        let st = install(&json!({ "dir": dir, "entry": "/opt/xenonet's/bin/xenonet" })).expect("installed");
        assert_eq!(
            (st["installed"].clone(), st["ours"].clone(), st["runs"].clone(), st["broken"].clone()),
            (json!(true), json!(true), json!(["/opt/xenonet's/bin/xenonet"]), json!(true))
        );
        let text = std::fs::read_to_string(format!("{dir}/xenonet")).expect("the script");
        assert!(text.starts_with("#!/bin/sh\n# Added by XenoNet") && text.ends_with("exec '/opt/xenonet'\\''s/bin/xenonet' \"$@\"\n"), "{text}");
        assert_eq!(uninstall(&dir).expect("taken out")["installed"], json!(false));
        std::fs::write(format!("{dir}/xenonet"), "#!/bin/sh\necho someone else's\n").expect("another's");
        assert!(install(&json!({ "dir": dir, "entry": "/e" })).is_err_and(|e| e.message.contains("isn't XenoNet's")));
        assert_eq!(install(&json!({ "dir": dir, "entry": "/e", "force": true })).expect("replaced")["ours"], json!(true));
        let _ = std::fs::remove_dir_all(&root);
    }
}
