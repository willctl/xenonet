//! xenonet update (cli/commands/update.js): service and tools updates,
//! checked or installed (core/components/updates.js). The Tools go first,
//! then the Service, as the release notes ask; Desktop updates itself in
//! its Updates panel. A bare `xenonet update` shows what would change and
//! asks first, unless --yes; naming what to update (tools, service or all)
//! is the go-ahead.

use crate::args::Flags;
use crate::cores;
use crate::js::str;
use crate::out::{c, print, Done, Fail};
use crate::prompt::confirm;
use serde_json::Value;
use xenonet_base::json::stringify;

pub const HELP: &str = include_str!("help/update.txt");

/// Where a component goes in the order updates install: the Tools, then
/// the Service, then anything else.
fn rank(item: &Value) -> u8 {
    match str(item, "component") {
        Some("tools") => 0,
        Some("service") => 1,
        _ => 2,
    }
}

/// The updates for `component` ("all" for every one), Tools first.
pub fn in_order(updates: &[Value], component: &str) -> Vec<Value> {
    let mut out: Vec<Value> = updates.iter().filter(|i| component == "all" || str(i, "component") == Some(component)).cloned().collect();
    out.sort_by_key(rank);
    out
}

/// What an update would change, a line each: "Tools 0.5.0 → 0.5.3", or
/// "Service (not installed) → 0.5.3". Those that are current say nothing.
pub fn changes(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.get("available").and_then(Value::as_bool) == Some(true))
        .map(|i| {
            let name = match str(i, "component") {
                Some("tools") => "Tools".to_string(),
                Some("service") => "Service".to_string(),
                other => other.unwrap_or("").to_string(),
            };
            let from = str(i, "current").map_or_else(|| "(not installed)".to_string(), str::to_string);
            format!("{name} {from} → {}", str(i, "version").unwrap_or("?"))
        })
        .collect()
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let named = args.first().map(String::as_str).filter(|a| !a.is_empty());
    let component = named.unwrap_or("all");
    if !["service", "tools", "all"].contains(&component) {
        return Err(Fail::error(HELP));
    }
    let items = in_order(&cores::component_updates()?, component);
    if !flags.on("check") && named.is_none() {
        let lines = changes(&items);
        if !lines.is_empty() {
            print("XenoNet would update:");
            for l in &lines {
                print(format!("  {l}"));
            }
            print(c::dim("  Desktop updates itself, in its Updates panel."));
            if !confirm("Update them?", flags)? {
                print(c::dim("Nothing changed."));
                return Ok(0);
            }
        }
    }
    for item in &items {
        let out = if flags.on("check") { item.clone() } else { cores::update_component(item)? };
        print(stringify(&out, 0));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_tools_update_before_the_service_and_only_what_changes_is_asked_about() {
        let service = json!({ "component": "service", "version": "0.5.3", "current": "0.5.2", "available": true });
        let tools = json!({ "component": "tools", "version": "0.5.3", "current": "0.5.0", "available": true });
        let listed = [service.clone(), tools.clone()];
        let all = in_order(&listed, "all");
        assert_eq!(all, [tools, service.clone()]);
        assert_eq!(in_order(&listed, "service"), std::slice::from_ref(&service));
        assert_eq!(changes(&all), ["Tools 0.5.0 → 0.5.3", "Service 0.5.2 → 0.5.3"]);
        let current = json!({ "component": "tools", "version": "0.5.3", "current": "0.5.3", "available": false });
        let missing = json!({ "component": "service", "version": "0.5.3", "current": null, "available": true });
        assert_eq!(changes(&[current, missing]), ["Service (not installed) → 0.5.3"]);
    }
}
