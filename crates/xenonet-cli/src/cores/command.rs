//! core/command.js for the command: the xenonet command on your PATH, a
//! small script XenoNet writes that runs the xenonet binary, told apart
//! from another program's by its marker line (xenonet-app's command). One
//! an earlier XenoNet wrote ran a node and cli/xenonet.js; it's XenoNet's
//! too, so installing replaces it and uninstalling takes it out.

use super::fs_fail;
use crate::out::Fail;
use crate::sys;
use serde_json::{json, Value};
use xenonet_app::command::{self, State};

fn path_var() -> Option<String> {
    std::env::var("PATH").ok()
}

fn state_json(st: &State) -> Value {
    json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
}

/// commandState({ dir })
pub fn state(dir: &str) -> Value {
    state_json(&command::command_state(Some(dir), path_var().as_deref(), sys::platform()))
}

/// installCommand({ dir, bin, force }): a script that runs `bin`.
pub fn install(opts: &Value) -> Result<Value, Fail> {
    let t = |k: &str| opts.get(k).filter(|v| !v.is_null()).map(xenonet_base::text::js_string);
    let dir = match t("dir") {
        Some(d) => d,
        None => super::command_dir()?,
    };
    let bin = t("bin").unwrap_or_default();
    let st = command::install_command(&dir, &bin, xenonet_base::text::truthy(opts.get("force")), path_var().as_deref(), sys::platform()).map_err(fs_fail)?;
    Ok(state_json(&st))
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
        let st = install(&json!({ "dir": dir, "bin": "/opt/xenonet's/bin/xenonet" })).expect("installed");
        assert_eq!(
            (st["installed"].clone(), st["ours"].clone(), st["runs"].clone(), st["broken"].clone()),
            (json!(true), json!(true), json!(["/opt/xenonet's/bin/xenonet"]), json!(true))
        );
        let text = std::fs::read_to_string(format!("{dir}/xenonet")).expect("the script");
        assert!(text.starts_with("#!/bin/sh\n# Added by XenoNet") && text.ends_with("exec '/opt/xenonet'\\''s/bin/xenonet' \"$@\"\n"), "{text}");
        assert_eq!(uninstall(&dir).expect("taken out")["installed"], json!(false));
        std::fs::write(format!("{dir}/xenonet"), "#!/bin/sh\necho someone else's\n").expect("another's");
        assert!(install(&json!({ "dir": dir, "bin": "/e" })).is_err_and(|e| e.message.contains("isn't XenoNet's")));
        assert_eq!(install(&json!({ "dir": dir, "bin": "/e", "force": true })).expect("replaced")["ours"], json!(true));
        let _ = std::fs::remove_dir_all(&root);
    }
}
