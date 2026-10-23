// xenonet service status, install and uninstall, against the binary, with
// a made-up launchctl or systemctl and a temporary folder for the launch
// agent or unit (XENONET_SUPERVISOR_BIN, XENONET_SUPERVISOR_DIR), and no
// service running: nothing reaches the machine's own launchd or systemd.
// Registering is refused from under an agent, so where these tests run
// under one, install's refusal is what's checked.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const LAUNCHCTL: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo "$*" >> "$dir/calls.log"
case "$1" in
  bootstrap) touch "$dir/loaded" ;;
  bootout) if [ -f "$dir/loaded" ]; then rm -f "$dir/loaded"; else exit 3; fi ;;
  print) if [ -f "$dir/loaded" ]; then printf 'x = {\n\tstate = not running\n}\n'; else echo "Could not find service" >&2; exit 113; fi ;;
esac
exit 0
"#;

const SYSTEMCTL: &str = r#"#!/bin/sh
dir=$(dirname "$0")
echo "$*" >> "$dir/calls.log"
case "$*" in
  "--user show"*) if [ -f "$dir/loaded" ]; then printf 'LoadState=loaded\nActiveState=inactive\nMainPID=0\nUnitFileState=enabled\n'; else printf 'LoadState=not-found\nActiveState=inactive\nMainPID=0\nUnitFileState=\n'; fi ;;
  "--user enable --now xenonet.service") touch "$dir/loaded" ;;
  "--user disable --now xenonet.service") rm -f "$dir/loaded" ;;
esac
exit 0
"#;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("xenonet-service-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("supervisor")).unwrap();
        let mac = cfg!(target_os = "macos");
        let bin = self_bin(&dir);
        std::fs::write(&bin, if mac { LAUNCHCTL } else { SYSTEMCTL }).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Fixture { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("agents").join(if cfg!(target_os = "macos") { "dev.xenonet.service.plist" } else { "xenonet.service" })
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("supervisor").join("calls.log")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_xenonet"))
            .args(args)
            .env("XENONET_SUPERVISOR_BIN", self_bin(&self.dir))
            .env("XENONET_SUPERVISOR_DIR", self.dir.join("agents"))
            .env("XENONET_RUNTIME_DIR", self.dir.join("runtime"))
            // The tray's app, on a Mac, goes here, never in ~/Applications.
            .env("XENONET_APPLICATIONS_DIR", self.dir.join("Applications"))
            // A port nothing listens on, so no service answers.
            .env("XENONET_PORT", "1")
            .env_remove("XENONET_LAUNCHER_BIN")
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn self_bin(dir: &Path) -> PathBuf {
    dir.join("supervisor").join(if cfg!(target_os = "macos") { "launchctl" } else { "systemctl" })
}

#[test]
fn status_says_how_it_starts_with_no_service_running() {
    let f = Fixture::new("status");
    let (code, out, _) = f.run(&["service", "status"]);
    assert_eq!(code, 1, "no service: {out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["running"], Value::Bool(false));
    let login = &v["login"];
    assert_eq!(login["supported"], Value::Bool(true));
    assert_eq!(login["supervisor"], if cfg!(target_os = "macos") { "launchd" } else { "systemd" });
    assert_eq!(login["file"], f.file().to_string_lossy().as_ref());
    assert_eq!((login["registered"].clone(), login["loaded"].clone()), (Value::Bool(false), Value::Bool(false)));
    assert!(f.calls().iter().all(|c| c.starts_with("print") || c.starts_with("--user show")), "only asked: {:?}", f.calls());
}

#[test]
fn install_and_uninstall_register_with_the_supervisor_or_are_refused_from_under_an_agent() {
    let f = Fixture::new("install");
    let (code, out, err) = f.run(&["service", "install", "--json"]);
    if code != 0 {
        assert!(err.contains("this is yours to do"), "{err}");
        assert!(!f.file().exists());
        assert!(f.calls().iter().all(|c| !c.contains("bootstrap") && !c.contains("enable")), "{:?}", f.calls());
        return;
    }
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "installed");
    let text = std::fs::read_to_string(f.file()).unwrap();
    assert!(text.contains("service") && text.contains("run"), "{text}");
    // Its port, not 47840, goes with it.
    assert!(text.contains("XENONET_PORT"), "{text}");
    let (_, out, _) = f.run(&["service", "status"]);
    let s: Value = serde_json::from_str(&out).unwrap();
    assert_eq!((s["login"]["registered"].clone(), s["login"]["loaded"].clone(), s["login"]["port"].clone()), (Value::Bool(true), Value::Bool(true), Value::from("1")));
    let (code, out, err) = f.run(&["service", "uninstall", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "removed");
    assert!(!f.file().exists());
}

// On a Mac, install puts the tray app the components have in Applications,
// with its own launch agent, and uninstall takes both away. With no tray
// installed (a binary run from target/, as here before the component is
// laid down) it's skipped.
#[cfg(target_os = "macos")]
#[test]
fn on_a_mac_the_tray_starts_at_login_beside_the_service_when_theres_one_installed() {
    let f = Fixture::new("tray");
    let tray_plist = f.dir.join("agents").join("dev.xenonet.tray.plist");
    let app = f.dir.join("Applications").join("XenoNet Tray.app");
    let (code, out, err) = f.run(&["service", "install", "--json"]);
    if code != 0 {
        assert!(err.contains("this is yours to do"), "{err}");
        assert!(!tray_plist.exists() && !app.exists());
        return;
    }
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["tray"]["status"], "absent", "no tray component: {out}");
    assert!(!tray_plist.exists());
    // The tray component, as a bundle installs it.
    let sha = "0123456789abcdef".repeat(4);
    let tray = f.dir.join("runtime").join("components").join("tray");
    let macos = tray.join(format!("0.4.1-{}", &sha[..16])).join("XenoNet Tray.app").join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::write(macos.join("XenoNet Tray"), "the tray").unwrap();
    std::fs::write(macos.parent().unwrap().join("Info.plist"), "<plist><dict><key>CFBundleIdentifier</key><string>dev.xenonet.tray</string></dict></plist>").unwrap();
    std::fs::write(tray.join("current.json"), format!(r#"{{"version":"0.4.1","sha256":"{sha}"}}"#)).unwrap();
    let (code, out, err) = f.run(&["service", "install", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["tray"]["status"], "installed", "{out}");
    assert_eq!(std::fs::read_to_string(app.join("Contents").join("MacOS").join("XenoNet Tray")).unwrap(), "the tray");
    let text = std::fs::read_to_string(&tray_plist).unwrap();
    assert!(
        text.contains("<string>dev.xenonet.tray</string>") && text.contains("XenoNet Tray.app/Contents/MacOS/XenoNet Tray") && text.contains("<key>XENONET_PORT</key>"),
        "{text}"
    );
    assert!(f.calls().iter().any(|c| c.starts_with("bootstrap") && c.ends_with("dev.xenonet.tray.plist")), "{:?}", f.calls());
    let (code, out, err) = f.run(&["service", "uninstall", "--json"]);
    assert_eq!(code, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["tray"]["status"], "removed", "{out}");
    assert!(!tray_plist.exists() && !app.exists());
}
