//! The macOS tray app at login (docs/design/always-on.md, "The tray app"):
//! `xenonet service install` copies XenoNet Tray.app from the installed
//! tray component to the home folder's Applications, takes the quarantine
//! off the copy, and writes a launch agent, dev.xenonet.tray, beside the
//! service's, that starts it at login. `uninstall` takes both away. With no
//! tray component installed (a build of your own, the tests' binary)
//! there's nothing to copy, and it's skipped.
//!
//! Everything goes through `Files`, `Runner` and `Apps`, so the tests use
//! a made-up folder, launchctl and xattr, never the machine's own.

use crate::packages::TRAY_APP;
use crate::supervisor::{parse_print, Files, Job, Kind, Place, Ran, Runner};
use serde_json::{json, Value};
use xenonet_base::nodepath::Style;

/// The tray's launch agent's label, and its plist's name with `.plist`.
pub const LABEL: &str = "dev.xenonet.tray";
/// The tray's bundle id, which its Info.plist says.
pub const BUNDLE_ID: &str = "dev.xenonet.tray";
/// The settings the tray keeps from the service's: where to find the
/// service and its connection file. The rest are the service's own.
pub const KEPT: &[&str] = &["XENONET_PORT", "XENONET_RUNTIME_DIR", "XDG_STATE_HOME"];
/// launchd starts it again after a crash at most this often, in seconds.
pub const THROTTLE_S: u32 = 10;

/// The app bundles: looked at, compared, copied and removed.
pub trait Apps {
    /// Whether a bundle has the tray's executable.
    fn exists(&self, app: &str) -> bool;
    /// Whether two bundles hold the same executable and Info.plist.
    fn same(&self, a: &str, b: &str) -> bool;
    /// Copies a bundle over `to`, through a temporary folder beside it.
    fn copy(&mut self, from: &str, to: &str) -> Result<(), String>;
    /// Removes a bundle that's XenoNet's tray, and refuses any other.
    fn remove(&mut self, app: &str) -> Result<(), String>;
}

/// Where the tray goes: its app in the home folder's Applications, its
/// launch agent beside the service's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayPlace {
    pub app: String,
    pub file: String,
    pub supervisor: String,
    pub uid: String,
}

impl TrayPlace {
    /// Beside the service's place, the app in `applications` (the home
    /// folder's Applications); None where it isn't launchd's.
    pub fn of(service: &Place, applications: &str) -> Option<TrayPlace> {
        if service.kind != Kind::Launchd || applications.is_empty() {
            return None;
        }
        let posix = Style::of("linux");
        Some(TrayPlace {
            app: posix.join(&[applications, TRAY_APP]),
            file: posix.join(&[&posix.dirname(&service.file), &format!("{LABEL}.plist")]),
            supervisor: service.supervisor.clone(),
            uid: service.uid.clone(),
        })
    }

    /// The executable the launch agent runs.
    pub fn exe(&self) -> String {
        Style::of("linux").join(&[&self.app, "Contents", "MacOS", "XenoNet Tray"])
    }

    fn target(&self) -> String {
        format!("gui/{}/{LABEL}", self.uid)
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The tray's launch agent: started at login, in your graphical session
/// only, and again after a crash (at most every THROTTLE_S seconds), but
/// not after you quit it; with the settings that say where the service is.
pub fn plist(place: &TrayPlace, env: &[(String, String)]) -> String {
    let mut l: Vec<String> = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".into(),
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">".into(),
        "<!-- XenoNet's tray, written by xenonet service install. xenonet service uninstall removes it. -->".into(),
        "<plist version=\"1.0\">".into(),
        "<dict>".into(),
        "\t<key>Label</key>".into(),
        format!("\t<string>{LABEL}</string>"),
        "\t<key>ProgramArguments</key>".into(),
        "\t<array>".into(),
        format!("\t\t<string>{}</string>", xml(&place.exe())),
        "\t</array>".into(),
    ];
    if !env.is_empty() {
        l.push("\t<key>EnvironmentVariables</key>".into());
        l.push("\t<dict>".into());
        for (k, v) in env {
            l.push(format!("\t\t<key>{}</key>", xml(k)));
            l.push(format!("\t\t<string>{}</string>", xml(v)));
        }
        l.push("\t</dict>".into());
    }
    l.extend(
        [
            "\t<key>RunAtLoad</key>",
            "\t<true/>",
            "\t<key>KeepAlive</key>",
            "\t<dict>",
            "\t\t<key>SuccessfulExit</key>",
            "\t\t<false/>",
            "\t</dict>",
            "\t<key>LimitLoadToSessionType</key>",
            "\t<string>Aqua</string>",
            "\t<key>ProcessType</key>",
            "\t<string>Interactive</string>",
        ]
        .map(String::from),
    );
    l.push("\t<key>ThrottleInterval</key>".into());
    l.push(format!("\t<integer>{THROTTLE_S}</integer>"));
    l.extend(["</dict>", "</plist>", ""].map(String::from));
    l.join("\n")
}

fn failed(what: &str, ran: &Ran) -> String {
    format!("{what} failed: {}", ran.said())
}

// launchctl bootout's answers when there was nothing to stop.
fn gone(r: &Ran) -> bool {
    r.ok() || r.code == Some(3) || r.code == Some(113)
}

/// Puts the tray in Applications and starts it at login: "installed",
/// "updated" (its app or its launch agent changed), "already" (as it was,
/// and loaded) or "absent" (no tray to install). `source` is the installed
/// tray component's XenoNet Tray.app; `job` is the service's, whose
/// settings the tray keeps what it needs of.
pub fn install(place: &TrayPlace, source: Option<&str>, job: &Job, files: &mut dyn Files, apps: &mut dyn Apps, run: &mut dyn Runner) -> Result<Value, String> {
    let Some(source) = source.filter(|s| apps.exists(s)) else {
        return Ok(json!({ "status": "absent", "app": null, "file": null }));
    };
    let env: Vec<(String, String)> = job.env.iter().filter(|(k, _)| KEPT.contains(&k.as_str())).cloned().collect();
    let text = plist(place, &env);
    let had = files.read(&place.file);
    let before = parse_print(&run.run(&place.supervisor, &["print", &place.target()]));
    let there = apps.exists(&place.app);
    let copy = !there || !apps.same(source, &place.app);
    if !copy && had.as_deref() == Some(text.as_str()) && before.loaded {
        return Ok(json!({ "status": "already", "app": place.app, "file": place.file }));
    }
    // A label turned off with launchctl disable stays off until it's
    // enabled again. The running tray goes before its app is replaced.
    run.run(&place.supervisor, &["enable", &place.target()]);
    if before.loaded {
        let r = run.run(&place.supervisor, &["bootout", &place.target()]);
        if !gone(&r) {
            return Err(failed("launchctl bootout", &r));
        }
    }
    if copy {
        apps.copy(source, &place.app)?;
    }
    // A copy of a downloaded app keeps the download's quarantine, and
    // macOS would ask about it again; the copy is XenoNet's own.
    run.run("/usr/bin/xattr", &["-dr", "com.apple.quarantine", &place.app]);
    files.write(&place.file, &text)?;
    let r = run.run(&place.supervisor, &["bootstrap", &place.domain(), &place.file]);
    if !r.ok() {
        return Err(failed("launchctl bootstrap", &r));
    }
    Ok(json!({ "status": if there || had.is_some() { "updated" } else { "installed" }, "app": place.app, "file": place.file }))
}

/// Stops the tray starting at login, quits it, and removes its app:
/// "removed", or "absent" when it wasn't set up. The app goes only with
/// the launch agent install wrote: one you put in Applications yourself,
/// with no agent of XenoNet's, stays.
pub fn uninstall(place: &TrayPlace, files: &mut dyn Files, apps: &mut dyn Apps, run: &mut dyn Runner) -> Result<Value, String> {
    let had = files.read(&place.file).is_some();
    let before = parse_print(&run.run(&place.supervisor, &["print", &place.target()]));
    if !had && !before.loaded {
        return Ok(json!({ "status": "absent", "app": place.app, "file": place.file }));
    }
    if before.loaded {
        let r = run.run(&place.supervisor, &["bootout", &place.target()]);
        if !gone(&r) {
            return Err(failed("launchctl bootout", &r));
        }
    }
    files.remove(&place.file)?;
    if apps.exists(&place.app) {
        apps.remove(&place.app)?;
    }
    Ok(json!({ "status": "removed", "app": place.app, "file": place.file }))
}

/// The bundles on disk.
pub struct Bundles;

fn exe_of(app: &str) -> std::path::PathBuf {
    std::path::Path::new(app).join("Contents").join("MacOS").join("XenoNet Tray")
}

fn plist_of(app: &str) -> std::path::PathBuf {
    std::path::Path::new(app).join("Contents").join("Info.plist")
}

/// Whether a bundle says it's XenoNet's tray, by its Info.plist.
fn ours(app: &str) -> bool {
    std::fs::read_to_string(plist_of(app)).is_ok_and(|t| t.contains(&format!("<string>{BUNDLE_ID}</string>")))
}

/// Copies a folder: files and folders only, each file with its mode, as a
/// clone where the disk can (APFS). A bundle or package of XenoNet's has no
/// links.
pub(crate) fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else if kind.is_file() {
            std::fs::copy(&src, &dst)?;
        } else {
            return Err(std::io::Error::other(format!("{} isn't a file or a folder", src.display())));
        }
    }
    Ok(())
}

impl Apps for Bundles {
    fn exists(&self, app: &str) -> bool {
        exe_of(app).is_file()
    }

    fn same(&self, a: &str, b: &str) -> bool {
        let read = |p: std::path::PathBuf| std::fs::read(p).ok();
        read(exe_of(a)).is_some_and(|x| Some(x) == read(exe_of(b))) && read(plist_of(a)).is_some_and(|x| Some(x) == read(plist_of(b)))
    }

    fn copy(&mut self, from: &str, to: &str) -> Result<(), String> {
        let dest = std::path::Path::new(to);
        if dest.exists() && !ours(to) {
            return Err(format!("{to} is there and isn't XenoNet's tray, so it's left as it is"));
        }
        let parent = dest.parent().ok_or_else(|| format!("{to} has no folder"))?;
        std::fs::create_dir_all(parent).map_err(|e| format!("couldn't make {}: {e}", parent.display()))?;
        let tmp = parent.join(format!(".{TRAY_APP}.{}.tmp", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let copied = copy_tree(std::path::Path::new(from), &tmp).and_then(|()| {
            if dest.exists() {
                std::fs::remove_dir_all(dest)?;
            }
            std::fs::rename(&tmp, dest)
        });
        copied.map_err(|e| {
            let _ = std::fs::remove_dir_all(&tmp);
            format!("couldn't copy the tray to {to}: {e}")
        })
    }

    fn remove(&mut self, app: &str) -> Result<(), String> {
        if !ours(app) {
            return Err(format!("{app} isn't XenoNet's tray, so it's left as it is"));
        }
        std::fs::remove_dir_all(app).map_err(|e| format!("couldn't remove {app}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(HashMap<String, String>);

    impl Files for Mem {
        fn read(&self, file: &str) -> Option<String> {
            self.0.get(file).cloned()
        }
        fn write(&mut self, file: &str, text: &str) -> Result<(), String> {
            self.0.insert(file.into(), text.into());
            Ok(())
        }
        fn remove(&mut self, file: &str) -> Result<(), String> {
            self.0.remove(file);
            Ok(())
        }
    }

    /// Bundles in memory: each app's build, by its path.
    #[derive(Default)]
    struct Shelf(HashMap<String, &'static str>);

    impl Apps for Shelf {
        fn exists(&self, app: &str) -> bool {
            self.0.contains_key(app)
        }
        fn same(&self, a: &str, b: &str) -> bool {
            self.0.contains_key(a) && self.0.get(a) == self.0.get(b)
        }
        fn copy(&mut self, from: &str, to: &str) -> Result<(), String> {
            let build = *self.0.get(from).ok_or("nothing to copy")?;
            self.0.insert(to.into(), build);
            Ok(())
        }
        fn remove(&mut self, app: &str) -> Result<(), String> {
            self.0.remove(app);
            Ok(())
        }
    }

    /// A made-up launchctl and xattr: each call kept, the tray's agent
    /// loaded or not.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        loaded: bool,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[&str]) -> Ran {
            self.calls.push(format!("{program} {}", args.join(" ")));
            let ok = Ran { code: Some(0), out: String::new(), err: String::new() };
            match args {
                ["print", _] if self.loaded => Ran { out: "gui/501/dev.xenonet.tray = {\n\tstate = running\n\tpid = 77\n}\n".into(), ..ok },
                ["print", _] => Ran { code: Some(113), out: String::new(), err: "Could not find service \"dev.xenonet.tray\" in domain for user gui: 501".into() },
                ["bootstrap", ..] => {
                    self.loaded = true;
                    ok
                }
                ["bootout", _] if self.loaded => {
                    self.loaded = false;
                    ok
                }
                ["bootout", _] => Ran { code: Some(3), out: String::new(), err: "Boot-out failed: 3: No such process".into() },
                _ => ok,
            }
        }
    }

    const SOURCE: &str = "/Users/me/Library/Application Support/XenoNet/runtime/components/tray/0.4.1-abc/XenoNet Tray.app";

    fn place() -> TrayPlace {
        let service = Place::of(&crate::supervisor::Where { platform: "darwin".into(), home: "/Users/me".into(), uid: "501".into(), ..Default::default() }).unwrap();
        TrayPlace::of(&service, "/Users/me/Applications").unwrap()
    }

    fn job() -> Job {
        Job {
            program: "/x/xenonet".into(),
            args: vec!["service".into(), "run".into()],
            env: vec![("XENONET_PORT".into(), "48321".into()), ("XENONET_PROJECTS_DIR".into(), "/p".into()), ("XENONET_RUNTIME_DIR".into(), "/r".into())],
            dir: "/Users/me".into(),
            log: "/r/service-48321.log".into(),
        }
    }

    #[test]
    fn the_tray_goes_beside_the_service_in_your_own_applications_and_only_on_a_mac() {
        let p = place();
        assert_eq!(p.app, "/Users/me/Applications/XenoNet Tray.app");
        assert_eq!(p.file, "/Users/me/Library/LaunchAgents/dev.xenonet.tray.plist");
        assert_eq!(p.exe(), "/Users/me/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray");
        let linux = Place::of(&crate::supervisor::Where { platform: "linux".into(), home: "/home/me".into(), ..Default::default() }).unwrap();
        assert_eq!(TrayPlace::of(&linux, "/home/me/Applications"), None);
        let text = plist(&p, &[("XENONET_PORT".into(), "48321".into())]);
        for want in [
            "<string>dev.xenonet.tray</string>",
            "<string>/Users/me/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray</string>",
            "<key>XENONET_PORT</key>",
            "<key>SuccessfulExit</key>\n\t\t<false/>",
            "<string>Aqua</string>",
        ] {
            assert!(text.contains(want), "{want}\n{text}");
        }
    }

    #[test]
    fn install_copies_the_app_takes_its_quarantine_off_and_starts_it_at_login() {
        let p = place();
        let (mut files, mut run) = (Mem::default(), Fake::default());
        let mut apps = Shelf::default();
        apps.0.insert(SOURCE.into(), "0.4.1");
        let out = install(&p, Some(SOURCE), &job(), &mut files, &mut apps, &mut run).unwrap();
        assert_eq!(out["status"], "installed");
        assert_eq!(apps.0.get(&p.app), Some(&"0.4.1"));
        let text = files.read(&p.file).unwrap();
        assert!(text.contains("<key>XENONET_PORT</key>") && text.contains("<key>XENONET_RUNTIME_DIR</key>"), "{text}");
        assert!(!text.contains("XENONET_PROJECTS_DIR"), "only where to find the service");
        assert_eq!(
            run.calls,
            [
                "/bin/launchctl print gui/501/dev.xenonet.tray",
                "/bin/launchctl enable gui/501/dev.xenonet.tray",
                "/usr/bin/xattr -dr com.apple.quarantine /Users/me/Applications/XenoNet Tray.app",
                "/bin/launchctl bootstrap gui/501 /Users/me/Library/LaunchAgents/dev.xenonet.tray.plist",
            ]
        );
        // Again, as it is: nothing changes.
        run.calls.clear();
        assert_eq!(install(&p, Some(SOURCE), &job(), &mut files, &mut apps, &mut run).unwrap()["status"], "already");
        assert_eq!(run.calls, ["/bin/launchctl print gui/501/dev.xenonet.tray"]);
        // A newer tray: the running one goes, the app is replaced, and it starts again.
        apps.0.insert(SOURCE.into(), "0.4.2");
        run.calls.clear();
        assert_eq!(install(&p, Some(SOURCE), &job(), &mut files, &mut apps, &mut run).unwrap()["status"], "updated");
        assert_eq!(apps.0.get(&p.app), Some(&"0.4.2"));
        assert!(run.calls.iter().position(|c| c.contains("bootout")) < run.calls.iter().position(|c| c.contains("bootstrap")), "{:?}", run.calls);
    }

    #[test]
    fn with_no_tray_installed_theres_nothing_to_do() {
        let p = place();
        let (mut files, mut run, mut apps) = (Mem::default(), Fake::default(), Shelf::default());
        for source in [None, Some(SOURCE)] {
            assert_eq!(install(&p, source, &job(), &mut files, &mut apps, &mut run).unwrap()["status"], "absent");
        }
        assert!(run.calls.is_empty() && files.0.is_empty() && apps.0.is_empty());
    }

    #[test]
    fn uninstall_quits_the_tray_and_removes_its_agent_and_app() {
        let p = place();
        let (mut files, mut run) = (Mem::default(), Fake::default());
        let mut apps = Shelf::default();
        apps.0.insert(SOURCE.into(), "0.4.1");
        install(&p, Some(SOURCE), &job(), &mut files, &mut apps, &mut run).unwrap();
        run.calls.clear();
        assert_eq!(uninstall(&p, &mut files, &mut apps, &mut run).unwrap()["status"], "removed");
        assert!(files.read(&p.file).is_none() && !apps.exists(&p.app) && !run.loaded);
        assert!(apps.exists(SOURCE), "the installed component stays");
        assert_eq!(run.calls, ["/bin/launchctl print gui/501/dev.xenonet.tray", "/bin/launchctl bootout gui/501/dev.xenonet.tray"]);
        assert_eq!(uninstall(&p, &mut files, &mut apps, &mut run).unwrap()["status"], "absent");
        // A tray you put in Applications yourself, with no agent of
        // XenoNet's, stays.
        apps.0.insert(p.app.clone(), "yours");
        assert_eq!(uninstall(&p, &mut files, &mut apps, &mut run).unwrap()["status"], "absent");
        assert!(apps.exists(&p.app));
    }

    #[test]
    fn on_disk_a_copy_replaces_only_xenonets_own_tray() {
        let dir = std::env::temp_dir().join(format!("xenonet-tray-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let make = |app: &std::path::Path, id: &str, exe: &str| {
            std::fs::create_dir_all(app.join("Contents").join("MacOS")).unwrap();
            std::fs::write(app.join("Contents").join("Info.plist"), format!("<plist><dict><key>CFBundleIdentifier</key><string>{id}</string></dict></plist>")).unwrap();
            std::fs::write(app.join("Contents").join("MacOS").join("XenoNet Tray"), exe).unwrap();
        };
        let source = dir.join("component").join(TRAY_APP);
        make(&source, BUNDLE_ID, "0.4.1");
        let to = dir.join("Applications").join(TRAY_APP);
        let (src, dst) = (source.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
        let mut b = Bundles;
        assert!(b.exists(&src) && !b.exists(&dst));
        b.copy(&src, &dst).unwrap();
        assert!(b.same(&src, &dst));
        make(&source, BUNDLE_ID, "0.4.2");
        assert!(!b.same(&src, &dst));
        b.copy(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("Contents").join("MacOS").join("XenoNet Tray")).unwrap(), "0.4.2");
        assert_eq!(std::fs::read_dir(dir.join("Applications")).unwrap().count(), 1, "no temporary copy left");
        // Something else by that name is left alone.
        make(&to, "com.example.other", "x");
        assert!(b.copy(&src, &dst).unwrap_err().contains("isn't XenoNet's tray"));
        assert!(b.remove(&dst).unwrap_err().contains("isn't XenoNet's tray"));
        make(&to, BUNDLE_ID, "0.4.2");
        b.remove(&dst).unwrap();
        assert!(!to.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
