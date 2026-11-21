//! The doctor's view of Adytum and Eidolon, the two tools that keep cells
//! and identities beside XenoNet, each in its own repository. Adytum's own
//! checks run in this process, through Adytum's doctor as a library, with a
//! short deadline. Eidolon is driven through its command, never its crates:
//! the doctor looks for `eidolon` on the PATH it uses for tools, asks it for
//! its own `--json` answers with an argument list and a short deadline, and
//! turns what comes back into a verdict. Eidolon not being there is an info
//! with the line that builds it, never a failure of XenoNet's doctor.
//! Nothing secret is read or shown: `eidolon ls` gives only public keys,
//! and only target names and holder names are used here.

use crate::checks::{info, ok, unreadable, warn, Ctx};
use adytum::doctor::{Finding, Verdict};
use serde_json::{json, Value};
use xenonet_os::error::Failure;
use xenonet_text::units::plural;

/// The checks, in order: id, group and title.
pub const CHECKS: [(&str, &str, &str); 2] = [("cells.adytum", "Integrations", "Adytum can keep cells"), ("cells.eidolon", "Integrations", "Eidolon keeps this machine's targets")];

/// How to get the tools' programs, from checkouts of their repositories.
const BUILD: &str =
    "From a checkout of github.com/willctl/adytum run cargo install --path crates/adytum-cli, and from github.com/willctl/eidolon run cargo install --path crates/eidolon-cli.";

const ADYTUM_MS: f64 = 6000.0;
const EIDOLON_MS: f64 = 3000.0;
/// What one finding or one target may say, and how many are listed.
const SAID_MAX: usize = 160;
const LISTED: usize = 5;

/// The demo's made-up answers, in the tools' own shapes: one warning from
/// Adytum, and a machine called `desk` holding one target with another
/// moving to `tower`.
fn demo_adytum() -> Vec<Finding> {
    vec![
        Finding { id: "limactl", message: "limactl 2.1.2 is installed.".into(), verdict: Verdict::Ok },
        Finding {
            id: "disk",
            message: "45 GB are free; a VM wants 20 GB to start with room to spare.".into(),
            verdict: Verdict::Warn { fix: "Free some space before making another cell.".into() },
        },
    ]
}
const DEMO_LS: &str = r#"{"key":"ssh-ed25519 AAAAdemo","targets":[
  {"target":"firefox","holder":"desk","held_here":true,"generation":4,"pending_to":null},
  {"target":"notes","holder":"desk","held_here":true,"generation":2,"pending_to":"tower"}]}"#;

pub(crate) async fn run(c: &Ctx<'_>, id: &str) -> Result<Value, Failure> {
    let demo = c.f.data.get("demo").and_then(Value::as_bool) == Some(true);
    match id {
        "cells.adytum" => Ok(adytum(c, demo).await),
        "cells.eidolon" => Ok(eidolon(c, demo).await),
        other => Err(Failure::msg(format!("no such check: {other}"))),
    }
}

/// One program's answer: its output, or why there is none.
enum Said {
    Out {
        stdout: String,
        stderr: String,
    },
    /// It started and didn't finish in time, or couldn't start.
    Late,
    Gone,
}

async fn ask(c: &Ctx<'_>, file: &str, args: &[&str], ms: f64) -> Said {
    let r = c.run(file, args, ms).await;
    if r.get("timedOut").and_then(Value::as_bool) == Some(true) {
        return Said::Late;
    }
    let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    // A program that can't start answers with a code that is a word.
    if r.get("code").is_some_and(Value::is_string) {
        return Said::Gone;
    }
    Said::Out { stdout: text("stdout"), stderr: text("stderr") }
}

async fn adytum(c: &Ctx<'_>, demo: bool) -> Value {
    if demo {
        return adytum_verdict(&demo_adytum());
    }
    // Its checks are linked in, but they only matter where Adytum is used,
    // and a machine without its command has no cells to check.
    if c.which("adytum").await.is_none() {
        return absent("Adytum");
    }
    match tokio::time::timeout(std::time::Duration::from_millis(ADYTUM_MS as u64), adytum::doctor::gather()).await {
        Ok(machine) => adytum_verdict(&adytum::doctor::checks(&machine)),
        Err(_) => warn(format!("adytum didn't answer within {} seconds", ADYTUM_MS / 1000.0), Some("Run adytum doctor yourself to see where it waits.")),
    }
}

async fn eidolon(c: &Ctx<'_>, demo: bool) -> Value {
    if demo {
        return eidolon_verdict(DEMO_LS, "");
    }
    let Some(file) = c.which("eidolon").await else { return absent("Eidolon") };
    match ask(c, &file, &["ls", "--json"], EIDOLON_MS).await {
        Said::Out { stdout, stderr } => eidolon_verdict(&stdout, &stderr),
        Said::Late => warn(format!("eidolon didn't answer within {} seconds", EIDOLON_MS / 1000.0), Some("Run eidolon ls yourself to see where it waits.")),
        Said::Gone => warn("eidolon is on the PATH but doesn't start", Some(BUILD)),
    }
}

fn absent(tool: &str) -> Value {
    info(format!("{tool} isn't installed here, so XenoNet shows nothing of it"), Some(BUILD))
}

// Text from a tool, safe to show on one line: control characters become
// spaces and it is cut short.
fn tidy(s: &str) -> String {
    let flat: String = s.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SAID_MAX {
        let cut: String = flat.chars().take(SAID_MAX).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn parse(s: &str) -> Option<Value> {
    serde_json::from_str(s.trim()).ok()
}

fn more(n: usize) -> String {
    if n > LISTED {
        format!(" and {} more", n - LISTED)
    } else {
        String::new()
    }
}

/// Adytum's findings as a verdict: what is failing or worth a warning,
/// each with the tool's own fix.
pub fn adytum_verdict(findings: &[Finding]) -> Value {
    let fails = |f: &&Finding| matches!(f.verdict, Verdict::Fail { .. });
    // Failures first.
    let wrong: Vec<&Finding> = findings.iter().filter(fails).chain(findings.iter().filter(|f| matches!(f.verdict, Verdict::Warn { .. }))).collect();
    if wrong.is_empty() {
        return ok(format!("Adytum: {} passed", plural(findings.len() as f64, "check", None)));
    }
    let failing = wrong.iter().filter(|f| fails(f)).count();
    let warns = wrong.len() - failing;
    let mut head = Vec::new();
    if failing > 0 {
        head.push(plural(failing as f64, "check failing", Some("checks failing")));
    }
    if warns > 0 {
        head.push(plural(warns as f64, "warning", None));
    }
    let level = |f: &Finding| if fails(&f) { "fail" } else { "warn" };
    let said = |f: &&Finding| format!("{} ({}): {}", tidy(f.id), level(f), tidy(&f.message));
    let detail = format!("Adytum: {}. {}{}", head.join(", "), wrong.iter().take(LISTED).map(said).collect::<Vec<_>>().join("; "), more(wrong.len()));
    let fixes: Vec<String> = wrong.iter().filter_map(|f| f.fix()).map(tidy).filter(|f| !f.is_empty()).take(LISTED).collect();
    // Adytum being unwell doesn't stop XenoNet, so it is a warning here.
    warn(detail, (!fixes.is_empty()).then(|| fixes.join(" ")).as_deref())
}

/// `eidolon ls --json` as a verdict: whether this machine has a key, which
/// targets it holds and which moves wait for an ack. `ls` and `err` are its
/// stdout and stderr; with --json a refusal comes on stdout.
pub fn eidolon_verdict(ls: &str, err: &str) -> Value {
    let Some(v) = parse(ls).or_else(|| parse(err)) else { return warn(unreadable("eidolon ls"), None) };
    if let Some(e) = v.get("error") {
        let said = |k: &str| tidy(e.get(k).and_then(Value::as_str).unwrap_or_default());
        return warn(format!("Eidolon can't list its targets: {}", said("message")), Some(&said("fix")).filter(|f| !f.is_empty()).map(String::as_str));
    }
    let Some(rows) = v.get("targets").and_then(Value::as_array) else { return warn(unreadable("eidolon ls"), None) };
    if v.get("key").is_none_or(Value::is_null) {
        return info("Eidolon has no key on this machine yet", Some("Run eidolon key init, then add each machine with eidolon holders add."));
    }
    let name = |r: &Value| tidy(r.get("target").and_then(Value::as_str).unwrap_or("?"));
    let here: Vec<String> = rows.iter().filter(|r| r.get("held_here") == Some(&json!(true))).map(name).collect();
    let moving: Vec<String> = rows.iter().filter_map(|r| r.get("pending_to").and_then(Value::as_str).map(|to| format!("{} to {}", name(r), tidy(to)))).collect();
    let mut detail = format!("Eidolon holds {} of {} here", here.len(), plural(rows.len() as f64, "target", None));
    if !here.is_empty() {
        detail = format!("{detail}: {}{}", here.iter().take(LISTED).cloned().collect::<Vec<_>>().join(", "), more(here.len()));
    }
    if moving.is_empty() {
        return ok(detail);
    }
    let waits = plural(moving.len() as f64, "move waits", Some("moves wait"));
    warn(
        format!("{detail}. {waits} for an ack: {}{}", moving.iter().take(LISTED).cloned().collect::<Vec<_>>().join("; "), more(moving.len())),
        Some("Run the same eidolon send again, or eidolon settle with the receiver's ack."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{self, Facts};
    use crate::fake::{ready, Fake};

    const LS: &str = r#"{"key":"ssh-ed25519 AAAA","targets":[
      {"target":"firefox","holder":"desk","held_here":true,"pending_to":null},
      {"target":"notes","holder":"desk","held_here":true,"pending_to":"tower"},
      {"target":"work","holder":"tower","held_here":false,"pending_to":null}]}"#;

    fn finding(id: &'static str, message: &str, verdict: Verdict) -> Finding {
        Finding { id, message: message.into(), verdict }
    }

    #[test]
    fn adytum_lists_its_fails_and_warns_with_their_fixes() {
        let findings = [
            finding("disk", "Low disk.", Verdict::Warn { fix: "Free space.".into() }),
            finding("limactl", "limactl isn't installed.", Verdict::Fail { fix: "Install Lima.".into() }),
            finding("fine", "fine", Verdict::Ok),
        ];
        let v = adytum_verdict(&findings);
        assert_eq!(v["status"], "warn");
        assert_eq!(v["detail"], "Adytum: 1 check failing, 1 warning. limactl (fail): limactl isn't installed.; disk (warn): Low disk.");
        assert_eq!(v["fix"], "Install Lima. Free space.");
        let all_good = adytum_verdict(&findings[2..]);
        assert_eq!((all_good["status"].as_str(), all_good["detail"].as_str()), (Some("ok"), Some("Adytum: 1 check passed")));
    }

    #[test]
    fn eidolon_lists_what_it_holds_and_what_waits_for_an_ack() {
        let v = eidolon_verdict(LS, "");
        assert_eq!(v["status"], "warn");
        assert_eq!(v["detail"], "Eidolon holds 2 of 3 targets here: firefox, notes. 1 move waits for an ack: notes to tower");
        assert!(v["fix"].as_str().unwrap().contains("eidolon settle"));
        let settled = eidolon_verdict(r#"{"key":"ssh-ed25519 AAAA","targets":[{"target":"firefox","held_here":true,"pending_to":null}]}"#, "");
        assert_eq!((settled["status"].as_str(), settled["detail"].as_str()), (Some("ok"), Some("Eidolon holds 1 of 1 target here: firefox")));
        assert_eq!(eidolon_verdict(r#"{"key":"ssh-ed25519 AAAA","targets":[]}"#, "")["detail"], "Eidolon holds 0 of 0 targets here");
    }

    #[test]
    fn eidolon_without_a_key_is_an_info_with_the_command_that_makes_one() {
        let v = eidolon_verdict(r#"{"key":null,"targets":[]}"#, "");
        assert_eq!(v["status"], "info");
        assert!(v["fix"].as_str().unwrap().contains("eidolon key init"));
    }

    #[test]
    fn eidolon_answers_that_cannot_be_read_or_refuse_are_a_warning() {
        assert_eq!(eidolon_verdict("garbage", "")["status"], "warn");
        assert_eq!(eidolon_verdict("{}", "")["status"], "warn");
        let refused = r#"{"error":{"code":"busy","message":"Another eidolon command holds firefox.","fix":"Wait for it to finish."}}"#;
        let v = eidolon_verdict("", refused);
        assert_eq!((v["status"].as_str(), v["fix"].as_str()), (Some("warn"), Some("Wait for it to finish.")));
        assert_eq!(eidolon_verdict(refused, "")["status"], "warn", "stdout carries it with --json");
    }

    #[test]
    fn what_a_tool_says_is_flattened_and_cut_before_it_is_shown() {
        assert_eq!(tidy("a\u{1b}[31m\nb\tc"), "a [31m b c");
        assert_eq!(tidy(&"x".repeat(400)).chars().count(), SAID_MAX + 1);
    }

    // The doctor's output with the tools there or not, through the real
    // check runner and a made-up machine.
    fn drive(id: &str, which: bool, run: impl Fn(&str, &[&str]) -> Value + 'static, demo: bool) -> Value {
        let host = Fake { run: Box::new(run), missing: if which { vec![] } else { vec!["eidolon"] }, ..Fake::default() };
        ready(checks::run(&host, id.into(), Facts { data: json!({ "platform": "darwin", "home": "/Users/ada", "demo": demo }) })).unwrap_or_else(|f| panic!("{}", f.message()))
    }

    #[test]
    fn a_missing_eidolon_is_an_info_with_the_build_line() {
        let v = drive("cells.eidolon", false, |_, _| unreachable!("nothing runs when the tool is missing"), false);
        assert_eq!(v["status"], "info");
        assert!(v["fix"].as_str().unwrap().contains("cargo install --path crates/eidolon-cli"));
    }

    #[test]
    fn eidolon_is_asked_with_argument_lists() {
        let asked = |file: &str, args: &[&str]| -> Value {
            match args {
                ["ls", "--json"] => json!({ "code": 0, "stdout": LS, "stderr": "" }),
                other => panic!("{file} {other:?}"),
            }
        };
        assert!(drive("cells.eidolon", true, asked, false)["detail"].as_str().unwrap().starts_with("Eidolon holds 2 of 3 targets here"));
    }

    #[test]
    fn an_eidolon_that_hangs_or_cannot_start_is_a_warning() {
        let v = drive("cells.eidolon", true, |_, _| json!({ "code": null, "stdout": "", "stderr": "", "timedOut": true }), false);
        assert!(v["detail"].as_str().unwrap().contains("didn't answer within 3 seconds"), "{v}");
        let v = drive("cells.eidolon", true, |_, _| json!({ "code": "ENOENT", "stdout": "", "stderr": "", "timedOut": false }), false);
        assert_eq!(v["status"], "warn");
    }

    #[test]
    fn the_demo_shows_made_up_data_and_runs_nothing() {
        let a = drive("cells.adytum", true, |_, _| unreachable!("the demo runs no program"), true);
        assert_eq!(a["status"], "warn");
        let e = drive("cells.eidolon", false, |_, _| unreachable!("the demo runs no program"), true);
        assert!(e["detail"].as_str().unwrap().starts_with("Eidolon holds 2 of 2 targets here"), "{e}");
    }
}
