//! xenonet spend (cli/commands/spend.js): about what today's work would
//! cost at API prices, by app and by actor, as Token use in the console
//! says it (core/live.js costs, core/actors/spend.js).

use crate::args::Flags;
use crate::js::{amount, have, list, num, on, t};
use crate::out::{c, json, plural, print, table, Done, Layout};
use crate::reach::need;
use serde_json::{json, Value};
use xenonet_base::text::js_string;
use xenonet_base::units::money;

pub const HELP: &str = include_str!("help/spend.txt");

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let live = s.get("/api/live").ok();
    // A XenoNet from before spend by actor, or with actors off, has none.
    let by_actor = match s.get("/api/spend/actors") {
        Ok(v) => Some(v).filter(|v| !v.is_null()),
        Err(e) if e.status == Some(404) => None,
        Err(e) => return Err(e.into()),
    };
    let costs = live.as_ref().and_then(|l| have(l, "costs")).cloned();
    if flags.on("json") {
        json(&json!({ "costs": costs, "byActor": by_actor }));
        return Ok(0);
    }
    if costs.is_none() && by_actor.is_none() {
        print(c::dim("This XenoNet doesn't say what the day's work would cost."));
        return Ok(0);
    }

    if let Some(costs) = &costs {
        let per_hour = num(costs, "perHour");
        print(format!(
            "About {} today at API prices{}",
            c::bold(money(amount(costs, "today"))),
            if per_hour > 0.0 { format!(", {} an hour at this pace", money(amount(costs, "perHour"))) } else { String::new() }
        ));
        let by_app = costs.get("byApp").cloned().unwrap_or(Value::Null);
        print(table(&[vec![c::dim("Claude Code"), money(amount(&by_app, "claude"))], vec![c::dim("Codex"), money(amount(&by_app, "codex"))]], Layout::indent(2)));
    } else if let Some(b) = &by_actor {
        print(format!("About {} today at API prices", c::bold(money(amount(b, "total")))));
    }

    if let Some(b) = &by_actor {
        print(format!("\n{}", c::bold("By actor")));
        let mut rows: Vec<Vec<String>> = list(b, "actors")
            .iter()
            .map(|r| {
                let mut said = Vec::new();
                if on(r, "sessions") {
                    said.push(plural(num(r, "sessions"), "session"));
                }
                if on(r, "elsewhere") {
                    said.push(format!("{} elsewhere, not priced", plural(num(r, "elsewhere"), "run")));
                }
                vec![t(r, "name"), if on(r, "sessions") { money(amount(r, "cost")) } else { c::dim("—") }, c::dim(said.join(", "))]
            })
            .collect();
        let u = b.get("unattributed").cloned().unwrap_or(Value::Null);
        rows.push(vec![
            c::dim("Unattributed"),
            money(amount(&u, "cost")),
            c::dim(format!("{} not tied to an actor, such as the ones you start", plural(num(&u, "sessions"), "session"))),
        ]);
        print(table(&rows, Layout::wrapped(2)));
        if list(b, "actors").is_empty() {
            print(c::dim("  No actor started a session that ran today."));
        }
    } else {
        print(c::dim("\nThis XenoNet doesn't break the day down by actor."));
    }

    let mut unpriced: Vec<String> = Vec::new();
    let costs_unpriced = costs.as_ref().map(|c| list(c, "unpriced").to_vec()).unwrap_or_default();
    let un_unpriced = by_actor.as_ref().and_then(|b| b.get("unattributed")).map(|u| list(u, "unpriced").to_vec()).unwrap_or_default();
    let actors_unpriced: Vec<Value> = by_actor.as_ref().map(|b| list(b, "actors").iter().flat_map(|r| list(r, "unpriced").to_vec()).collect()).unwrap_or_default();
    for v in costs_unpriced.iter().chain(&un_unpriced).chain(&actors_unpriced) {
        let name = js_string(v);
        if !unpriced.contains(&name) {
            unpriced.push(name);
        }
    }
    unpriced.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let checked = costs.as_ref().and_then(|c| have(c, "checked")).or_else(|| by_actor.as_ref().and_then(|b| have(b, "checked")));
    let none = if unpriced.is_empty() { String::new() } else { format!("Not counted, with no price: {}. ", unpriced.join(", ")) };
    print(c::dim(format!("\n{none}Prices checked {}. On a plan, the work costs the plan.", crate::js::text(checked))));
    Ok(0)
}
