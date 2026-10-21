//! core/doctor/index.js's checks of the service started at login
//! (docs/design/always-on.md): whether it's registered, whether the
//! registration is what `xenonet service install` writes now, whether
//! launchd, systemd or Task Scheduler has it, and on Linux whether
//! lingering starts it at boot, which a host with no graphical session
//! needs. They read `ctx.login()`, the registration as xenonet-components'
//! supervisor::review sees it.

use crate::checks::{fail, info, ok, skip, warn, Ctx};
use serde_json::{json, Value};
use xenonet_app::jsv::truthy;
use xenonet_app::task::Failure;
use xenonet_base::text::js_string;

/// The checks, in order: id, group and title.
pub const CHECKS: [(&str, &str, &str); 4] = [
    ("service.login", "Background service", "The service starts at login"),
    ("service.file", "Background service", "The service's registration is current"),
    ("service.loaded", "Background service", "The system has the service loaded"),
    ("service.boot", "Background service", "The service starts at boot"),
];

const NOT_REGISTERED: &str = "the service doesn't start at login";

// The registration, or why there's none to read.
async fn registration(c: &Ctx<'_>) -> Result<Result<Value, &'static str>, Failure> {
    let l = c.call("login", json!([])).await.map_err(|_| Failure::Rethrow)?;
    let Some(l) = l.filter(|v| truthy(Some(v))) else { return Ok(Err("XenoNet can't look at how its service starts from here")) };
    if !truthy(l.get("supported")) {
        return Ok(Err("this system has no supervisor XenoNet registers with"));
    }
    if !truthy(l.get("registered")) {
        return Ok(Err(NOT_REGISTERED));
    }
    Ok(Ok(l))
}

fn text(l: &Value, k: &str) -> String {
    l.get(k).filter(|v| !v.is_null()).map(js_string).unwrap_or_default()
}

// Where each supervisor says more, and what loads the service again.
fn look(by: &str) -> Option<&'static str> {
    match by {
        "launchd" => Some("launchctl print gui/$(id -u)/dev.xenonet.service says more."),
        "systemd" => Some("systemctl --user status xenonet says more."),
        "Task Scheduler" => Some("schtasks /query /tn \"XenoNet Service\" /v /fo list says more."),
        _ => None,
    }
}

fn unloaded(by: &str) -> &'static str {
    match by {
        "launchd" => "If you turned it off in System Settings › General › Login Items & Extensions, turn it on there. Otherwise run xenonet service install again, which loads it.",
        "systemd" => "Run xenonet service install again, which enables it, or systemctl --user enable --now xenonet.service.",
        "Task Scheduler" => "Run xenonet service install again, which creates the task again.",
        _ => "Run xenonet service install again.",
    }
}

/// One of CHECKS' verdicts.
pub(crate) async fn run(c: &Ctx<'_>, id: &str) -> Result<Value, Failure> {
    let f = c.f;
    if id == "service.boot" && f.platform() != "linux" {
        return Ok(skip("for Linux, where lingering starts a user's services at boot"));
    }
    let l = match registration(c).await? {
        Ok(l) => l,
        Err(why) if id == "service.login" && why == NOT_REGISTERED => {
            return Ok(info(
                "off: Desktop starts the service when it opens, and it runs until you log out",
                Some("To keep sessions coordinating while Desktop is closed, turn on Background service in XenoNet's palette, or run xenonet service install."),
            ))
        }
        Err(why) => return Ok(skip(why)),
    };
    let by = text(&l, "supervisor");
    let file = c.tilde(&text(&l, "file"));
    match id {
        "service.login" => {
            let me = f.data.get("self").filter(|v| truthy(Some(v)));
            let port = l.get("port").filter(|p| !p.is_null());
            let other = match (me, port) {
                (Some(me), Some(p)) if js_string(p) != me.get("port").map_or_else(|| "undefined".into(), js_string) => format!(", for the service on port {}", js_string(p)),
                _ => String::new(),
            };
            Ok(ok(format!("on, through {by}: {file}{other}")))
        }
        "service.file" => {
            if l.get("programFound") == Some(&json!(false)) {
                return Ok(fail(
                    format!("{file} runs {}, which isn't there", c.tilde(&text(&l, "program"))),
                    Some("Run xenonet service install again, which points it at XenoNet's launcher."),
                ));
            }
            match l.get("current") {
                Some(Value::Bool(false)) => Ok(warn(
                    format!("{file} isn't what xenonet service install writes now, so the service may not start the way this XenoNet expects"),
                    Some("Run xenonet service install again, which writes it afresh and reloads it. That restarts the service, so close its terminals first."),
                )),
                Some(Value::Bool(true)) => Ok(ok(format!("{file} is what xenonet service install writes"))),
                _ => {
                    let why = l.get("expected").filter(|v| !v.is_null()).map_or_else(|| "it couldn't work out what install writes here".into(), js_string);
                    Ok(info(format!("XenoNet couldn't tell whether {file} is current: {why}"), None))
                }
            }
        }
        "service.loaded" => {
            if let Some(e) = l.get("error").filter(|v| truthy(Some(v))) {
                return Ok(warn(format!("XenoNet couldn't ask {by} about it: {}", js_string(e)), look(&by)));
            }
            let state = text(&l, "state");
            if state == "disabled" {
                return Ok(warn(
                    "it's turned off in Task Scheduler, so it doesn't start when you sign in",
                    Some("Turn XenoNet Service on in Task Scheduler, or run xenonet service install, which turns it on again."),
                ));
            }
            if !truthy(l.get("loaded")) {
                return Ok(warn(format!("{by} doesn't have it, so it doesn't start at login"), Some(unloaded(&by))));
            }
            let last = l.get("lastExit").filter(|v| truthy(Some(v))).map(js_string);
            let exited = last.filter(|e| e != "0" && e != "(never exited)").map_or(String::new(), |e| format!(" (it last exited with {e})"));
            if state == "failed" {
                return Ok(warn(
                    format!("{by} gave up on it after it kept stopping{exited}"),
                    Some("xenonet logs --errors says why it stopped. Then xenonet service start starts it again."),
                ));
            }
            if let Some(pid) = l.get("pid").filter(|v| truthy(Some(v))) {
                return Ok(ok(format!("{by} has it, running as process {}", js_string(pid))));
            }
            Ok(info(
                format!("{by} has it, but it isn't running now{exited}"),
                Some("xenonet service start starts it. If it didn't stop on purpose, xenonet logs --errors says why."),
            ))
        }
        "service.boot" => {
            let user = Some(text(&l, "user")).filter(|u| !u.is_empty()).unwrap_or_else(|| "$USER".into());
            match l.get("linger") {
                Some(Value::Bool(true)) => Ok(ok("lingering is on, so it starts at boot and keeps running after you log out")),
                Some(Value::Bool(false)) if l.get("graphical") == Some(&json!(false)) => Ok(warn(
                    "lingering is off, and you have no graphical session here, as on a host reached over SSH, so the service runs only while you're logged in",
                    Some(&format!(
                        "Run loginctl enable-linger {user} once, which starts it at boot and keeps it running. It may need an administrator: sudo loginctl enable-linger {user}."
                    )),
                )),
                Some(Value::Bool(false)) => Ok(info(
                    "lingering is off: it starts when you log in, and stops when you log out",
                    Some(&format!("To start it at boot instead, before anyone logs in, as a host needs, run loginctl enable-linger {user} once. It may need an administrator.")),
                )),
                _ => Ok(info("XenoNet couldn't ask loginctl whether lingering is on", Some(&format!("loginctl show-user {user} --property=Linger says.")))),
            }
        }
        _ => Err(Failure::msg(format!("no such check: {id}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{self, Facts};
    use xenonet_app::task::{Outcome, Task};

    // One check's verdict on `platform`, with `login` as ctx.login()
    // answers (None: a ctx without it).
    fn verdict(id: &str, platform: &str, login: Option<Value>) -> Value {
        let facts = Facts { data: json!({ "platform": platform, "home": "/Users/ada", "self": { "port": 47840 } }) };
        let owned = id.to_string();
        let mut task = Task::new(move |io| async move { checks::run(io, owned, facts).await });
        let mut reply = None;
        loop {
            match task.step(reply.take()) {
                Outcome::Ask(q) => {
                    assert_eq!(q["fn"], json!("login"), "{q}");
                    reply = Some(Ok(login.as_ref().map_or_else(|| json!({ "missing": true }), |l| json!({ "value": l }))));
                }
                Outcome::Done(v) => return v,
                Outcome::Failed(f) => panic!("{id}: {}", f.message()),
            }
        }
    }

    fn on(over: Value) -> Value {
        let mut l = json!({ "supported": true, "supervisor": "launchd", "file": "/Users/ada/Library/LaunchAgents/dev.xenonet.service.plist", "registered": true, "port": "47840", "current": true, "program": "/Users/ada/x", "programFound": true, "loaded": true, "state": "running", "pid": 4242, "lastExit": "(never exited)", "user": "ada" });
        for (k, v) in over.as_object().into_iter().flatten() {
            l[k] = v.clone();
        }
        l
    }

    #[test]
    fn a_registration_in_place_passes() {
        assert_eq!(
            verdict("service.login", "darwin", Some(on(json!({})))),
            json!({ "status": "ok", "detail": "on, through launchd: ~/Library/LaunchAgents/dev.xenonet.service.plist" })
        );
        assert_eq!(verdict("service.file", "darwin", Some(on(json!({}))))["status"], json!("ok"));
        assert_eq!(verdict("service.loaded", "darwin", Some(on(json!({}))))["detail"], json!("launchd has it, running as process 4242"));
        assert_eq!(verdict("service.boot", "darwin", Some(on(json!({}))))["status"], json!("skip"));
        assert_eq!(
            verdict("service.login", "darwin", Some(on(json!({ "port": "47850" }))))["detail"],
            json!("on, through launchd: ~/Library/LaunchAgents/dev.xenonet.service.plist, for the service on port 47850")
        );
    }

    #[test]
    fn off_is_a_note_and_the_rest_skip() {
        let off = json!({ "supported": true, "supervisor": "systemd", "registered": false });
        assert_eq!(verdict("service.login", "linux", Some(off.clone()))["status"], json!("info"));
        for id in ["service.file", "service.loaded", "service.boot"] {
            assert_eq!(verdict(id, "linux", Some(off.clone())), json!({ "status": "skip", "detail": "the service doesn't start at login" }), "{id}");
        }
        assert_eq!(verdict("service.login", "linux", None)["detail"], json!("XenoNet can't look at how its service starts from here"));
        assert_eq!(verdict("service.login", "freebsd", Some(json!({ "supported": false })))["status"], json!("skip"));
    }

    #[test]
    fn what_needs_fixing_says_how() {
        let gone = verdict("service.file", "darwin", Some(on(json!({ "programFound": false, "program": "/Users/ada/gone/xenonet", "current": false }))));
        assert_eq!(
            (gone["status"].clone(), gone["detail"].clone()),
            (json!("fail"), json!("~/Library/LaunchAgents/dev.xenonet.service.plist runs ~/gone/xenonet, which isn't there"))
        );
        assert_eq!(verdict("service.file", "darwin", Some(on(json!({ "current": false }))))["status"], json!("warn"));
        assert_eq!(verdict("service.file", "win32", Some(on(json!({ "current": null, "expected": "no account" }))))["status"], json!("info"));
        let unloaded = verdict("service.loaded", "darwin", Some(on(json!({ "loaded": false, "pid": null, "state": null }))));
        assert_eq!(unloaded["status"], json!("warn"));
        assert!(unloaded["fix"].as_str().unwrap().contains("Login Items"));
        let off = verdict("service.loaded", "win32", Some(on(json!({ "supervisor": "Task Scheduler", "loaded": false, "state": "disabled" }))));
        assert!(off["fix"].as_str().unwrap().starts_with("Turn XenoNet Service on in Task Scheduler"));
        let failed = verdict("service.loaded", "linux", Some(on(json!({ "supervisor": "systemd", "state": "failed", "pid": null, "lastExit": "1" }))));
        assert_eq!(failed["detail"], json!("systemd gave up on it after it kept stopping (it last exited with 1)"));
        let asked = verdict("service.loaded", "linux", Some(on(json!({ "supervisor": "systemd", "error": "Failed to connect to bus" }))));
        assert_eq!(asked["fix"], json!("systemctl --user status xenonet says more."));
    }

    #[test]
    fn lingering_matters_where_nobody_sits_at_a_screen() {
        let linux = |over: Value| {
            verdict(
                "service.boot",
                "linux",
                Some(on(json!({ "supervisor": "systemd" })
                    .as_object()
                    .into_iter()
                    .flatten()
                    .chain(over.as_object().into_iter().flatten())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect())),
            )
        };
        assert_eq!(linux(json!({ "linger": true }))["status"], json!("ok"));
        assert_eq!(linux(json!({ "linger": false, "graphical": true }))["status"], json!("info"));
        let host = linux(json!({ "linger": false, "graphical": false }));
        assert_eq!(host["status"], json!("warn"));
        assert!(host["fix"].as_str().unwrap().contains("sudo loginctl enable-linger ada"));
        assert_eq!(linux(json!({ "linger": null }))["detail"], json!("XenoNet couldn't ask loginctl whether lingering is on"));
    }
}
