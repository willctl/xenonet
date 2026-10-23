//! macOS and Linux: the server and the curator as per-user services (launchd LaunchAgents, systemd
//! user units). Both run as the user, so the curator needs no tray to host it; both restart on
//! failure, and the server exits when an install replaces it (--exit-on-upgrade) to be restarted on
//! the new build.

use super::InstallError;
use super::places::Places;
use super::sys::{Ran, run};
use std::path::{Path, PathBuf};

pub const SERVER_LABEL: &str = "dev.xenolith.server";
pub const CURATOR_LABEL: &str = "dev.xenolith.curator";
/// The tray's login item on macOS, which the tray writes itself (`--do repair-autostart`). Linux
/// starts the tray from an autostart file instead.
#[cfg(target_os = "macos")]
pub const TRAY_LABEL: &str = "dev.xenolith.tray";
/// The labels Xenolith used before it had its own, tray first: retiring the tray's before anything
/// new is loaded means two trays never start.
pub const LEGACY_LABELS: [&str; 3] = ["dev.xenonet.xenolith.tray", "dev.xenonet.xenolith.server", "dev.xenonet.xenolith.curator"];
pub const SERVER_UNIT: &str = "xenolith.service";
pub const CURATOR_UNIT: &str = "xenolith-curator.service";

/// A path as one shell token: single quotes when needed.
pub fn shell_token(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "_./+:@%,=-".contains(c)) { s.into_owned() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// PATH for the services: the usual places for node (an npm-installed codex is a node script) and codex.
fn service_path(p: &Places) -> String {
    let mut dirs: Vec<String> = vec![];
    if let Some(codex) = super::sys::which("codex").and_then(|c| c.parent().map(Path::to_path_buf)) {
        dirs.push(codex.to_string_lossy().into_owned());
    }
    if let Some(node) = super::sys::which("node").and_then(|c| c.parent().map(Path::to_path_buf)) {
        dirs.push(node.to_string_lossy().into_owned());
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(String::from));
    dirs.push(p.home.join(".local").join("bin").to_string_lossy().into_owned());
    dirs.dedup();
    dirs.join(":")
}

fn launch_agents(p: &Places) -> PathBuf {
    p.home.join("Library").join("LaunchAgents")
}

fn plist(label: &str, args: &[String], log: &Path, path: &str) -> String {
    let a: String = args.iter().fold(String::new(), |mut out, x| {
        out.push_str(&format!("<string>{}</string>", xml(x)));
        out
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key><string>{label}</string>\n  <key>ProgramArguments</key>\n  <array>{a}</array>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n  <key>ThrottleInterval</key><integer>5</integer>\n  <key>ProcessType</key><string>Background</string>\n  <key>EnvironmentVariables</key>\n  <dict><key>PATH</key><string>{}</string></dict>\n  <key>StandardErrorPath</key><string>{}</string>\n  <key>StandardOutPath</key><string>/dev/null</string>\n</dict>\n</plist>\n",
        xml(path),
        xml(&log.to_string_lossy())
    )
}

fn unit(description: &str, args: &[String], path: &str, restart_sec: u32) -> String {
    let exec: Vec<String> = args.iter().map(|a| if a.contains([' ', '"', '\\']) { format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")) } else { a.clone() }).collect();
    format!(
        "# Written by the xenolith installer; removed by xenolith uninstall.\n[Unit]\nDescription={description}\nAfter=default.target\n\n[Service]\nExecStart={}\nEnvironment=PATH={path}\nRestart=always\nRestartSec={restart_sec}\nNoNewPrivileges=yes\n\n[Install]\nWantedBy=default.target\n",
        exec.join(" ")
    )
}

fn uid() -> u32 {
    // SAFETY: getuid has no failure mode.
    unsafe { libc::getuid() }
}

/// Writes and (re)starts the server and curator services. Returns what it did, for the log.
pub fn install_services(p: &Places, port: u16, curator: bool) -> Result<Vec<String>, InstallError> {
    let agent = p.agent.to_string_lossy().into_owned();
    let server_args = vec![agent.clone(), "serve".into(), "--http".into(), "--port".into(), port.to_string(), "--exit-on-upgrade".into()];
    let curator_args = vec![agent, "curator".into()];
    let path = service_path(p);
    let mut done = vec![];
    let err = |e: std::io::Error| InstallError(e.to_string());
    std::fs::create_dir_all(&p.app.log_dir).map_err(err)?;
    if cfg!(target_os = "macos") {
        let dir = launch_agents(p);
        std::fs::create_dir_all(&dir).map_err(err)?;
        let mut jobs = vec![(SERVER_LABEL, server_args, p.app.log_dir.join("service.log"))];
        if curator {
            jobs.push((CURATOR_LABEL, curator_args, p.app.log_dir.join("curator.log")));
        }
        done.extend(load_launch_agents(&dir, &format!("gui/{}", uid()), &jobs, &path, &mut |args| run(Path::new("launchctl"), args))?);
    } else {
        let dir = p.app.config_dir.parent().map(|d| d.join("systemd").join("user")).unwrap_or_else(|| p.home.join(".config").join("systemd").join("user"));
        std::fs::create_dir_all(&dir).map_err(err)?;
        std::fs::write(dir.join(SERVER_UNIT), unit("Xenolith MCP server (127.0.0.1)", &server_args, &path, 2)).map_err(err)?;
        let mut units = vec![SERVER_UNIT];
        if curator {
            std::fs::write(dir.join(CURATOR_UNIT), unit("Xenolith curator", &curator_args, &path, 5)).map_err(err)?;
            units.push(CURATOR_UNIT);
        }
        let sc = Path::new("systemctl");
        let _ = run(sc, &["--user", "daemon-reload"]);
        for u in units {
            let r = run(sc, &["--user", "enable", u]);
            let s = run(sc, &["--user", "restart", u]);
            done.push(format!("{u}: {}", if r.ok && s.ok { "enabled and started".to_string() } else { format!("systemctl: {}", if s.ok { r.last_line() } else { s.last_line() }) }));
        }
    }
    Ok(done)
}

/// What launchctl is to these functions: the real one, or a fake in the tests.
type Launchctl<'a> = &'a mut dyn FnMut(&[&str]) -> Ran;

/// Boots out and removes the LaunchAgents under the old labels, the tray's first. A job is booted
/// out by its label as well, in case its plist was deleted while it stayed loaded. Returns what it
/// found, for the log.
pub fn retire_legacy(dir: &Path, domain: &str, launchctl: Launchctl) -> Vec<String> {
    let mut done = vec![];
    for label in LEGACY_LABELS {
        let file = dir.join(format!("{label}.plist"));
        let loaded = launchctl(&["bootout", &format!("{domain}/{label}")]).ok;
        let had = file.exists();
        if had {
            let _ = launchctl(&["bootout", domain, &file.to_string_lossy()]);
            let _ = std::fs::remove_file(&file);
        }
        if loaded || had {
            done.push(format!("{label}: stopped and removed (now {})", label.replacen("dev.xenonet.xenolith.", "dev.xenolith.", 1)));
        }
    }
    done
}

/// Writes and (re)loads the LaunchAgents in `dir`, after the old labels are gone.
fn load_launch_agents(dir: &Path, domain: &str, jobs: &[(&str, Vec<String>, PathBuf)], path: &str, launchctl: Launchctl) -> Result<Vec<String>, InstallError> {
    let mut done = retire_legacy(dir, domain, launchctl);
    for (label, args, log) in jobs {
        let file = dir.join(format!("{label}.plist"));
        std::fs::write(&file, plist(label, args, log, path)).map_err(|e| InstallError(e.to_string()))?;
        let _ = launchctl(&["bootout", domain, &file.to_string_lossy()]);
        let r = launchctl(&["bootstrap", domain, &file.to_string_lossy()]);
        done.push(format!("{label}: {}", if r.ok { "loaded".to_string() } else { format!("launchctl bootstrap: {}", r.last_line()) }));
    }
    Ok(done)
}

pub fn uninstall_services(p: &Places) -> Vec<String> {
    let mut done = vec![];
    if cfg!(target_os = "macos") {
        let domain = format!("gui/{}", uid());
        done.extend(retire_legacy(&launch_agents(p), &domain, &mut |args| run(Path::new("launchctl"), args)));
        for label in [SERVER_LABEL, CURATOR_LABEL] {
            let file = launch_agents(p).join(format!("{label}.plist"));
            if file.exists() {
                let _ = run(Path::new("launchctl"), &["bootout", &domain, &file.to_string_lossy()]);
                let _ = std::fs::remove_file(&file);
                done.push(format!("removed {label}"));
            }
        }
    } else {
        let dir = p.app.config_dir.parent().map(|d| d.join("systemd").join("user")).unwrap_or_else(|| p.home.join(".config").join("systemd").join("user"));
        for u in [SERVER_UNIT, CURATOR_UNIT] {
            if dir.join(u).exists() {
                let _ = run(Path::new("systemctl"), &["--user", "disable", "--now", u]);
                let _ = std::fs::remove_file(dir.join(u));
                done.push(format!("removed {u}"));
            }
        }
        let _ = run(Path::new("systemctl"), &["--user", "daemon-reload"]);
    }
    done
}

/// Whether a graphical session is there to show a tray icon.
pub fn has_display() -> bool {
    cfg!(target_os = "macos") || std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launchctl that records each call and answers as `loaded` says: a job bootout by label
    /// succeeds only for the labels that are loaded.
    fn fake<'a>(calls: &'a mut Vec<String>, loaded: &'static [&'static str]) -> impl FnMut(&[&str]) -> Ran + 'a {
        move |args: &[&str]| {
            calls.push(args.join(" "));
            let ok = match args {
                ["bootout", target] => loaded.iter().any(|l| target.ends_with(&format!("/{l}"))),
                _ => true,
            };
            Ran { ok, code: Some(if ok { 0 } else { 3 }), stdout: String::new(), stderr: String::new() }
        }
    }

    #[test]
    fn the_old_labels_go_tray_first_before_anything_new_is_loaded() {
        let dir = std::env::temp_dir().join(format!("xenolith-launchagents-{}", xenolith_core::text::random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        for l in LEGACY_LABELS {
            std::fs::write(dir.join(format!("{l}.plist")), "old").unwrap();
        }
        std::fs::write(dir.join("com.example.other.plist"), "someone else's").unwrap();
        let mut calls = vec![];
        let jobs =
            vec![(SERVER_LABEL, vec!["xenolith".to_string(), "serve".into()], dir.join("service.log")), (CURATOR_LABEL, vec!["xenolith".to_string(), "curator".into()], dir.join("curator.log"))];
        let done = load_launch_agents(&dir, "gui/501", &jobs, "/usr/bin", &mut fake(&mut calls, &["dev.xenonet.xenolith.tray", "dev.xenonet.xenolith.server"])).unwrap();

        let first_bootstrap = calls.iter().position(|c| c.starts_with("bootstrap")).expect("the new jobs are loaded");
        let last_old = calls.iter().rposition(|c| c.contains("dev.xenonet.")).unwrap();
        assert!(last_old < first_bootstrap, "every old label is gone before a new one loads: {calls:?}");
        let old: Vec<&String> = calls.iter().filter(|c| c.contains("dev.xenonet.")).collect();
        assert!(old[0].contains("dev.xenonet.xenolith.tray") && old[1].contains("dev.xenonet.xenolith.tray"), "the tray first: {calls:?}");
        assert!(calls.contains(&"bootout gui/501/dev.xenonet.xenolith.curator".to_string()), "a loaded job without its plist is booted out by label too");
        for l in LEGACY_LABELS {
            assert!(!dir.join(format!("{l}.plist")).exists(), "{l} removed");
        }
        assert!(dir.join("com.example.other.plist").exists(), "other agents are left alone");
        for l in [SERVER_LABEL, CURATOR_LABEL] {
            let text = std::fs::read_to_string(dir.join(format!("{l}.plist"))).unwrap();
            assert!(text.contains(&format!("<key>Label</key><string>{l}</string>")));
            assert!(calls.contains(&format!("bootstrap gui/501 {}", dir.join(format!("{l}.plist")).display())));
        }
        assert_eq!(done[0], "dev.xenonet.xenolith.tray: stopped and removed (now dev.xenolith.tray)");
        assert_eq!(done.iter().filter(|d| d.ends_with(": loaded")).count(), 2);

        // Run again (a re-install): nothing old is left, so nothing is retired.
        let mut calls = vec![];
        let done = load_launch_agents(&dir, "gui/501", &jobs, "/usr/bin", &mut fake(&mut calls, &[])).unwrap();
        assert!(done.iter().all(|d| d.ends_with(": loaded")), "{done:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
