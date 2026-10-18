//! xenonet live (cli/commands/live.js): messages that reach sessions as
//! they're sent, and their controls: pause them all, let XenoNet wake, type
//! into or resume a session, grant an actor's token a scope, and read the
//! audit log (server/routes/delivery.js).

use crate::args::Flags;
use crate::clock;
use crate::js::{list, num, on, t};
use crate::out::{c, json, json_at, print, table, Done, Fail, Layout};
use crate::reach::need;
use serde_json::{Map, Value};
use xenonet_base::json::{number, number_string, stringify};
use xenonet_base::text::js_string;
use xenonet_base::units::{js_round, to_fixed};

pub const HELP: &str = include_str!("help/live.txt");

fn actor_id(a: &str) -> String {
    if a.starts_with("actor:") {
        a.to_string()
    } else {
        format!("actor:{a}")
    }
}

// The answer, as JSON with --json, or as it is.
#[expect(clippy::unnecessary_wraps, reason = "it ends a command, whose answer is a Done like every other command's")]
fn say(out: &Value, flags: &Flags) -> Done {
    if flags.on("json") {
        json(out);
    } else {
        print(out.as_str().map_or_else(|| stringify(out, 0), str::to_string));
    }
    Ok(0)
}

fn body(pairs: Vec<(&str, Option<Value>)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    }
    Value::Object(m)
}

fn status(d: &Value) {
    print(format!("{} {}", c::bold("Live delivery"), if on(d, "paused") { c::red("paused") } else { c::green("on") }));
    let plans: Vec<(String, f64)> = d
        .get("usage")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, u)| !u.is_null())
        .map(|(app, u)| (app.clone(), xenonet_base::text::js_number(Some(u))))
        .collect();
    if !plans.is_empty() {
        let said: Vec<String> =
            plans.iter().map(|(app, u)| format!("{app} {}%{}", number_string(js_round(u * 100.0)), if *u >= 0.9 { c::red(" (no wakes)") } else { String::new() })).collect();
        print(format!("Plans used: {}{}", said.join(", "), c::dim(" · a provider at 90% takes no wakes")));
    }
    print(c::bold("\nActors' scopes"));
    let grants = list(d, "grants");
    print(if grants.is_empty() {
        c::dim("  none: actors see no sessions")
    } else {
        table(
            &grants
                .iter()
                .map(|g| vec![t(g, "actor"), t(g, "selector"), if on(g, "expired") { c::red("expired") } else { format!("until {}", clock::date_time(num(g, "expiresAt"))) }])
                .collect::<Vec<_>>(),
            Layout::indent(2),
        )
    });
    let listening = list(d, "listening");
    if !listening.is_empty() {
        print(c::bold("\nListening after a turn"));
        print(table(&listening.iter().map(|l| vec![t(l, "agent"), format!("until {}", clock::time(num(l, "until")))]).collect::<Vec<_>>(), Layout::indent(2)));
    }
    print(c::bold("\nSessions XenoNet may wake, type into or resume"));
    let consent: Vec<(&String, &Value)> = d.get("consent").and_then(Value::as_object).into_iter().flatten().collect();
    print(if consent.is_empty() {
        c::dim("  none")
    } else {
        table(
            &consent
                .iter()
                .map(|(id, x)| vec![(*id).clone(), format!("until {}", clock::date_time(num(x, "until"))), if on(x, "remote") { "remote too".into() } else { String::new() }])
                .collect::<Vec<_>>(),
            Layout::indent(2),
        )
    });
    print(c::bold("\nLast deliveries"));
    let recent = list(d, "recent");
    print(if recent.is_empty() {
        c::dim("  none yet")
    } else {
        let rows: Vec<Vec<String>> = recent[recent.len().saturating_sub(10)..]
            .iter()
            .map(|r| {
                let how = r.get("how").filter(|h| !h.is_null()).map_or(String::new(), |h| crate::js::text(Some(h)));
                let acked = if on(r, "ackedAt") { format!("acked in {} s", to_fixed((num(r, "ackedAt") - num(r, "at")) / 1000.0, 1)) } else { String::new() };
                vec![format!("#{}", t(r, "message")), t(r, "agent"), t(r, "state"), how, acked]
            })
            .collect();
        table(&rows, Layout::indent(2))
    });
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let sub = args.first().map(String::as_str).filter(|x| !x.is_empty());
    let rest = args.get(1..).unwrap_or(&[]);
    let hours = flags.on("hours").then(|| number(flags.number("hours")));
    match sub {
        None | Some("status") => {
            let d = s.get("/api/delivery")?;
            if flags.on("json") {
                json(&d);
            } else {
                status(&d);
            }
            Ok(0)
        }
        Some(sub @ ("pause" | "resume")) => say(&s.post("/api/delivery/pause", &body(vec![("paused", Some(Value::Bool(sub == "pause")))]), &[])?, flags),
        Some(sub @ ("allow" | "disallow")) => {
            let Some(session) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new(format!("say which session: xenonet live {sub} <session id>"), 1)) };
            say(
                &s.post(
                    &format!("/api/delivery/{sub}"),
                    &body(vec![("session", Some(Value::String(session.clone()))), ("hours", hours), ("remote", Some(Value::Bool(flags.on("remote"))))]),
                    &[],
                )?,
                flags,
            )
        }
        Some("grant") => {
            if rest.len() < 2 {
                return Err(Fail::new("say which actor and what scope: xenonet live grant hermes project:xenonet", 1));
            }
            say(
                &s.post(
                    "/api/delivery/grant",
                    &body(vec![("actor", Some(Value::String(actor_id(&rest[0])))), ("scope", Some(Value::String(rest[1..].join(" ")))), ("hours", hours)]),
                    &[],
                )?,
                flags,
            )
        }
        Some("revoke") => {
            let Some(actor) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new("say which actor: xenonet live revoke hermes", 1)) };
            say(&s.post("/api/delivery/revoke", &body(vec![("actor", Some(Value::String(actor_id(actor))))]), &[])?, flags)
        }
        Some("audit") => {
            let d = s.get("/api/delivery?audit=100")?;
            if flags.on("json") {
                json_at(d.get("audit"));
                return Ok(0);
            }
            for a in list(&d, "audit") {
                let rest: Vec<String> =
                    a.as_object().into_iter().flatten().filter(|(k, _)| k.as_str() != "at" && k.as_str() != "kind").map(|(k, v)| format!("{k}={}", js_string(v))).collect();
                print(format!("{} {} {}", c::dim(clock::date_time(num(a, "at"))), t(a, "kind"), rest.join(" ")));
            }
            Ok(0)
        }
        Some(sub) => Err(Fail::new(format!("xenonet live doesn't know \"{sub}\"; xenonet live --help lists what it does"), 1)),
    }
}
