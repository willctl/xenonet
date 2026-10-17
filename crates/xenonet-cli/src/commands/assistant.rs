//! xenonet assistant (cli/commands/assistant.js): your remote actors'
//! connections, from your own terminal (docs/guide/cloud-assistants.md).
//! Pairing one, and turning cloud access back on, happen in XenoNet's
//! window. Revoking one and turning them all off work with XenoNet stopped
//! too: they leave a tombstone in XenoNet's state folder, which every
//! version of XenoNet honors, and XenoNet is told as well when it's
//! running.

use crate::args::Flags;
use crate::config::instance_dir;
use crate::js::{have, list, num, on, str, t};
use crate::out::{ago, c, json, now, print, table, Done, Fail, Layout};
use crate::pending;
use crate::reach::find;
use crate::sys;
use serde_json::{json, Value};
use xenonet_base::client::{name_of, Server};
use xenonet_base::json::number_string;

pub const HELP: &str = include_str!("help/assistant.txt");

/// A grant's id, gr_ and 24 hex digits (core/schema/assistant.js's
/// GRANT_ID).
pub fn is_grant_id(s: &str) -> bool {
    s.len() == 27 && s.starts_with("gr_") && s[3..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// XenoNet's state folder, for the tombstones: XenoNet Dev's with --dev.
fn state_dir(flags: &Flags) -> String {
    if flags.on("dev") {
        instance_dir("state", "XenoNet Dev")
    } else {
        sys::state_dir(false)
    }
}

/// assistantDir (core/assistant/credentials.js): the grants' folder.
fn assistant_dir(state: &str) -> String {
    sys::join(state, "assistant")
}

fn state_words(state: &str) -> String {
    match state {
        "active" => c::green("active"),
        "expired" => c::yellow("expired"),
        "revoked" => c::dim("revoked"),
        "disabled" => c::yellow("off"),
        s => s.into(),
    }
}

// The first ten characters, a date.
fn day(v: &Value, k: &str) -> String {
    xenonet_base::log::js_slice(&t(v, k), 10).to_string()
}

fn show(data: &Value, s: &Server) {
    let device = data.get("device").cloned().unwrap_or(Value::Null);
    print(format!(
        "{} {} cloud access is {} {}",
        c::bold(name_of(s)),
        c::dim("·"),
        if on(data, "disabled") { c::yellow("off") } else { c::green("on") },
        c::dim(format!("· device {}", t(&device, "id")))
    ));
    if let Some(d) = crate::js::some(data, "diagnostics") {
        let endpoint = d.get("endpoint").filter(|e| !e.is_null());
        let listening = match endpoint.filter(|e| on(e, "listening")) {
            Some(e) => format!("listening, {} adapter{} connected", t(e, "adapters"), if num(e, "adapters") == 1.0 { "" } else { "s" }),
            None => "not listening".into(),
        };
        let sources: Vec<String> = list(d, "sources").iter().map(|x| format!("{} {}", t(x, "source"), t(x, "state"))).collect();
        print(c::dim(format!("service {}, interface {} · endpoint {listening} · {}", t(d, "service_version"), t(d, "assistant_api"), sources.join(", "))));
    }
    let connections = list(data, "connections");
    if connections.is_empty() {
        print(c::dim("No remote connections. Pair one in XenoNet's window: Actors, then Remote connections."));
        return;
    }
    print("");
    let rows: Vec<Vec<String>> = connections
        .iter()
        .map(|x| {
            let projects = list(x, "projects").len();
            let what = format!(
                "{projects} project{}{}{}",
                if projects == 1 { "" } else { "s" },
                if on(x, "labels") { ", labels" } else { "" },
                if str(x, "data_mode") == Some("fixture") { ", made-up data" } else { "" }
            );
            let until = if str(x, "state") == Some("active") {
                format!("until {}", day(x, "expires_at"))
            } else if on(x, "revoked_at") {
                format!("revoked {}", day(x, "revoked_at"))
            } else {
                String::new()
            };
            let used =
                if on(x, "last_used_at") { format!("used {}", ago(xenonet_base::time::parse_iso(&t(x, "last_used_at")).unwrap_or(f64::NAN), now())) } else { c::dim("never used") };
            vec![state_words(&t(x, "state")), t(x, "grant_id"), t(x, "actor_id"), t(x, "label"), what, until, used]
        })
        .collect();
    print(table(&rows, Layout::default()));
}

fn out(data: &Value, words: &str, flags: &Flags) -> Done {
    if flags.on("json") {
        json(data);
    } else {
        print(words);
    }
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let what = args.first().map_or("list", String::as_str);
    let grant = args.get(1).cloned();
    let s = find(flags);
    match what {
        "list" | "status" => {
            let Some(s) = s else {
                let dir = assistant_dir(&state_dir(flags));
                let disabled = pending::is_disabled(&dir)?;
                let revoked = pending::marked_revoked(&dir)?;
                let words = format!(
                    "XenoNet isn't running. Cloud access is {} as XenoNet's state folder says{}.",
                    if disabled { "off" } else { "on, for its active grants," },
                    if revoked.is_empty() { String::new() } else { format!("; {} revoked", revoked.len()) }
                );
                return out(&json!({ "running": false, "disabled": disabled, "revoked": revoked }), &words, flags);
            };
            let data = s.get("/api/assistant")?;
            if flags.on("json") {
                json(&data);
            } else {
                show(&data, &s);
            }
            Ok(0)
        }
        "audit" => {
            let Some(s) = s else { return Err(Fail::new("XenoNet isn't running, so its log can't be read: open XenoNet first.", 1)) };
            let given = flags.text("grant");
            if given.as_deref().is_some_and(|g| !is_grant_id(g)) {
                return Err(Fail::new("--grant takes a grant's id (gr_…).", 2));
            }
            let limit = flags.number("limit");
            let limit = if limit == 0.0 || limit.is_nan() { 50.0 } else { limit };
            let route = format!("/api/assistant/audit?limit={}{}", number_string(limit), given.map_or(String::new(), |g| format!("&grant={g}")));
            let answer = s.get(&route)?;
            let records = answer.get("records").cloned().unwrap_or(Value::Null);
            if flags.on("json") {
                json(&records);
                return Ok(0);
            }
            let records = records.as_array().cloned().unwrap_or_default();
            if records.is_empty() {
                print(c::dim("Nothing read in the last seven days."));
                return Ok(0);
            }
            let rows: Vec<Vec<String>> = records
                .iter()
                .map(|r| {
                    let at = xenonet_base::log::js_slice(&t(r, "at"), 19).replacen('T', " ", 1);
                    let who = if str(r, "kind") == Some("owner") { c::cyan("you") } else { have(r, "actor").map_or(String::new(), |a| crate::js::text(Some(a))) };
                    let result = if str(r, "result") == Some("ok") { c::green("ok") } else { c::yellow(t(r, "result")) };
                    let bytes = have(r, "bytes").map_or(String::new(), |b| format!("{} B", crate::js::text(Some(b))));
                    let latency = have(r, "latency_ms").map_or(String::new(), |l| format!("{} ms", crate::js::text(Some(l))));
                    vec![at, who, t(r, "operation"), result, bytes, latency]
                })
                .collect();
            print(table(&rows, Layout::default()));
            Ok(0)
        }
        "disable" => {
            // The tombstone first, so it holds whatever happens next.
            pending::mark_disabled(&assistant_dir(&state_dir(flags)))?;
            if let Some(s) = &s {
                if let Err(e) = s.post("/api/assistant/disable", &json!({}), &[]) {
                    print(c::yellow(format!("XenoNet didn't take it at once ({}); it will within two seconds.", e.message)));
                }
            }
            let words = format!(
                "Cloud access is off{}. Every remote connection stopped; turn it on again in XenoNet's window. Revoke each endpoint with its provider too: what was read already isn't taken back.",
                if s.is_some() { "" } else { ", for when XenoNet starts" }
            );
            out(&json!({ "disabled": true }), &words, flags)
        }
        "revoke" => {
            let grant = grant.filter(|g| is_grant_id(g)).ok_or_else(|| Fail::new("xenonet assistant revoke takes a grant's id (gr_…), as xenonet assistant lists them.", 2))?;
            let dir = assistant_dir(&state_dir(flags));
            pending::mark_revoked(&dir, &grant)?;
            pending::delete_credential(&dir, &grant)?;
            if let Some(s) = &s {
                match s.post(&format!("/api/assistant/{grant}/revoke"), &json!({}), &[]) {
                    Err(e) if e.status != Some(404) => return Err(e.into()),
                    _ => {}
                }
            }
            out(&json!({ "revoked": grant }), &format!("Revoked {grant}. Revoke its endpoint with its provider too: what was read already isn't taken back."), flags)
        }
        "enable" => Err(Fail::new("Turn cloud access on again in XenoNet's window: Actors, then Remote connections. It widens access, so it's the window's to do.", 1)),
        "pair" => Err(Fail::new("Pair a remote actor in XenoNet's window: Actors, then Remote connections. It shows exactly what would leave this computer first.", 1)),
        _ => Err(Fail::new(format!("no xenonet assistant {what}. xenonet assistant --help lists what there is."), 2)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn grant_ids() {
        assert!(super::is_grant_id("gr_0a1b2c3d4e5f60718293a4b5"));
        assert!(!super::is_grant_id("gr_0A1B2C3D4E5F60718293A4B5"));
        assert!(!super::is_grant_id("gr_0a1b"));
    }
}
