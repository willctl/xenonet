//! The guest setup is data: its argument lists and units are pinned in
//! golden files, and it never builds a shell string.

use crate::support::{cell, failed, ok, FakeDriver};
use eidolon_browser::units::{self, Ports};
use eidolon_browser::{provision_plan, start_browser_plan, start_view_plan, stream_argv, BrowserError, Plan, MOZILLA_KEY_FINGERPRINT};
use std::path::PathBuf;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR")))
}

/// Compares with the golden file; `UPDATE_GOLDEN=1` rewrites it.
fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(actual, want, "{name} differs from the golden file; run with UPDATE_GOLDEN=1 if the change is meant");
}

#[test]
fn the_provisioning_plan_matches_its_golden_file() {
    let plan = provision_plan(Ports::default());
    let mut json = serde_json::to_string_pretty(&plan).unwrap();
    json.push('\n');
    check_golden("provision.json", &json);
}

#[test]
fn the_units_match_their_golden_files() {
    for (name, text) in units::units(Ports::default()) {
        check_golden(&format!("units/{name}"), &text);
    }
    check_golden("units/user.js", units::USER_JS);
}

#[test]
fn no_argument_is_a_shell_string() {
    let mut plans = vec![provision_plan(Ports::default()), start_browser_plan(), start_view_plan("a-secret")];
    plans.push(Plan { steps: vec![] });
    for plan in plans {
        for step in &plan.steps {
            for arg in step.argv.iter().chain(step.skip_if.iter().flatten()) {
                for bad in ["&&", ";", "|", "$(", "`", ">", "<"] {
                    assert!(!arg.contains(bad), "{:?} has {bad:?}: {arg:?}", step.why);
                }
            }
        }
    }
    assert!(stream_argv(9222).iter().all(|a| !a.contains(';') && !a.contains('|')));
}

#[test]
fn every_command_runs_as_root_through_sudo_or_is_a_read() {
    for step in provision_plan(Ports::default()).steps {
        let first = step.argv.first().map(String::as_str);
        assert!(first == Some("sudo") || first == Some("gpg"), "{:?}", step.argv);
    }
}

#[test]
fn firefox_comes_from_mozillas_repository_not_the_snap() {
    let plan = provision_plan(Ports::default());
    let text = serde_json::to_string(&plan).unwrap();
    assert!(text.contains("https://packages.mozilla.org/apt"));
    assert!(text.contains(MOZILLA_KEY_FINGERPRINT));
    assert!(!text.contains("snap"), "nothing installs a snap");
}

#[test]
fn the_view_password_is_stdin_and_never_an_argument() {
    let plan = start_view_plan("hunter-two");
    for step in &plan.steps {
        assert!(!step.argv.iter().any(|a| a.contains("hunter-two")), "{:?}", step.argv);
    }
    assert_eq!(plan.steps.iter().filter(|s| s.stdin.as_deref() == Some("hunter-two")).count(), 1);
    // And a step's debug text, which may reach a log, does not show it.
    assert!(!format!("{plan:?}").contains("hunter-two"));
}

#[test]
fn nothing_hides_automation() {
    let all = format!("{}{}", units::USER_JS, units::units(Ports::default()).into_iter().map(|(_, t)| t).collect::<String>()).to_lowercase();
    for word in ["webdriver", "useragent", "user-agent", "stealth", "resistfingerprinting", "marionette"] {
        assert!(!all.contains(word), "{word}");
    }
}

#[test]
fn the_browser_unit_has_automation_and_the_view_unit_has_none() {
    let units = units::units(Ports { automation: 9333, vnc: 5911 });
    let text = |n: &str| units.iter().find(|(name, _)| *name == n).unwrap().1.clone();
    let browser = text(units::BROWSER_UNIT);
    assert!(browser.contains("--remote-debugging-port 9333"));
    assert!(browser.contains("Restart=always"));
    assert!(browser.contains("Conflicts=eidolon-view.service"));
    let view = text(units::VIEW_UNIT);
    assert!(!view.contains("remote-debugging") && !view.contains("--headless"));
    assert!(view.contains("Conflicts=eidolon-browser.service"));
    let vnc = text(units::VNC_UNIT);
    assert!(vnc.contains("-localhost") && vnc.contains("-rfbport 5911") && vnc.contains("-passwdfile /run/eidolon/vnc.pass"));
    assert!(units.iter().all(|(_, t)| !t.contains('@')), "every placeholder is filled");
}

#[test]
fn the_stream_reaches_the_port_through_socat_on_the_guests_loopback() {
    assert_eq!(stream_argv(9222), ["socat", "-", "TCP:127.0.0.1:9222"]);
}

#[tokio::test]
async fn running_the_plan_sends_each_file_as_stdin() {
    let driver = FakeDriver::answering(|argv| {
        if argv.first().map(String::as_str) == Some("gpg") {
            ok(format!("pub:-:2048:1:C0BA5CE6DC6315A3:1620162483:::-:::scSC::::::23::0:\nfpr:::::::::{MOZILLA_KEY_FINGERPRINT}:\n").as_bytes())
        } else if argv.first().map(String::as_str) == Some("id") {
            failed(1, "")
        } else {
            ok(b"")
        }
    });
    provision_plan(Ports::default()).run(driver.as_ref(), &cell()).await.unwrap();
    let execs = driver.execs();
    let tee =
        |path: &str| execs.iter().find(|e| e.argv == ["sudo", "tee", path]).map(|e| String::from_utf8(e.stdin.clone().unwrap_or_default()).unwrap_or_default()).unwrap_or_default();
    assert_eq!(tee("/etc/systemd/system/eidolon-browser.service"), units::units(Ports::default())[0].1);
    assert!(tee("/etc/apt/sources.list.d/mozilla.sources").contains("Signed-By: /etc/apt/keyrings/packages.mozilla.org.asc"));
    // The user was missing, so the guard failed and the user was made.
    assert!(execs.iter().any(|e| e.argv.contains(&"useradd".to_owned())));
}

#[tokio::test]
async fn a_step_whose_guard_succeeds_is_skipped() {
    let driver =
        FakeDriver::answering(|argv| if argv.first().map(String::as_str) == Some("gpg") { ok(format!("fpr:::::::::{MOZILLA_KEY_FINGERPRINT}:\n").as_bytes()) } else { ok(b"") });
    provision_plan(Ports::default()).run(driver.as_ref(), &cell()).await.unwrap();
    assert!(!driver.execs().iter().any(|e| e.argv.contains(&"useradd".to_owned())), "id -u succeeded, so the user exists");
}

#[tokio::test]
async fn a_wrong_signing_key_stops_the_plan_before_the_repository_is_added() {
    let driver =
        FakeDriver::answering(|argv| if argv.first().map(String::as_str) == Some("gpg") { ok(b"fpr:::::::::0000000000000000000000000000000000000000:\n") } else { ok(b"") });
    let err = provision_plan(Ports::default()).run(driver.as_ref(), &cell()).await.unwrap_err();
    assert!(matches!(err, BrowserError::Fingerprint { .. }), "{err}");
    assert!(!driver.execs().iter().any(|e| e.argv.iter().any(|a| a.contains("mozilla.sources"))));
}

#[tokio::test]
async fn a_failed_step_says_which_and_never_quotes_its_stdin() {
    let driver =
        FakeDriver::answering(
            |argv| if argv.contains(&"/run/eidolon/vnc.pass".to_owned()) || argv.iter().any(|a| a.starts_with("of=")) { failed(1, "disk full") } else { ok(b"") },
        );
    let err = start_view_plan("hunter-two").run(driver.as_ref(), &cell()).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("give the window its password") && text.contains("disk full"), "{text}");
    assert!(!text.contains("hunter-two"));
}
