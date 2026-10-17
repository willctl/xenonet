//! The drift report (core/vendor.js's driftReport and reportLines), as
//! xenonet drift shows it: from drift.json when no XenoNet is running, and
//! as lines of text by source either way.

use crate::clock;
use crate::out::{c, plural};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use xenonet_base::json::number;
use xenonet_base::text::{js_string, truthy};
use xenonet_base::units::locale;
use xenonet_schema::drift::{describe, source_name, DriftLog, ADDITIONS};
use xenonet_schema::lenient::{load_vendor, vendor_sources};

/// XenoNet's schemas/ folder, beside the command: the first of the folders
/// the binary is in, and those above it, that has one (a checkout's, for
/// target/debug/xenonet). None when there's none.
pub fn schemas_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().skip(1).take(5).map(|d| d.join("schemas")).find(|d| d.join("vendor").is_dir())
}

fn first_seen(r: &Value) -> f64 {
    r.get("firstSeen").and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn is_addition(r: &Value) -> bool {
    r.get("kind").and_then(Value::as_str).is_some_and(|k| ADDITIONS.contains(&k))
}

/// driftReport({ log, all, readers: [], home, now }): the report from what
/// `log` keeps, nothing having been read in this process.
pub fn drift_report(log: &mut DriftLog, all: bool, dir: Option<&Path>, home: Option<&str>, now: f64) -> Value {
    let (file, looked_at, records) = log.view();
    let newer = |r: &Value| looked_at.is_none_or(|l| first_seen(r) > l);
    let sources: Vec<Value> = dir
        .map(vendor_sources)
        .unwrap_or_default()
        .into_iter()
        .map(|source| {
            let mine: Vec<&Value> = records.iter().filter(|r| r.get("source").and_then(Value::as_str) == Some(source.as_str())).collect();
            let schemas: Vec<Value> =
                dir.map(|d| load_vendor(&source, d)).unwrap_or_default().into_iter().map(|m| json!({ "id": m.id, "major": m.major, "range": m.range })).collect();
            json!({
                "source": source,
                "name": source_name(&source),
                "schemas": schemas,
                "read": null,
                "drift": mine.iter().filter(|r| all || newer(r)).map(|r| (*r).clone()).collect::<Vec<_>>(),
                "known": mine.len(),
            })
        })
        .collect();
    let fresh: Vec<&Value> = sources.iter().flat_map(|s| s["drift"].as_array().into_iter().flatten()).filter(|r| newer(r)).collect();
    let (new, changes) = (fresh.len(), fresh.iter().filter(|r| !is_addition(r)).count());
    let known: u64 = sources.iter().map(|s| s["known"].as_u64().unwrap_or(0)).sum();
    let mut out = Map::new();
    out.insert("at".into(), number(now));
    out.insert("file".into(), file.map_or(Value::Null, |f| Value::String(home.filter(|h| !h.is_empty()).map_or_else(|| f.clone(), |h| xenonet_base::paths::tilde(&f, h)))));
    out.insert("since".into(), looked_at.map_or(Value::Null, number));
    out.insert("all".into(), Value::Bool(all));
    out.insert("sources".into(), Value::Array(sources));
    out.insert("new".into(), json!(new));
    out.insert("changes".into(), json!(changes));
    out.insert("known".into(), json!(known));
    Value::Object(out)
}

fn n(v: Option<&Value>) -> f64 {
    xenonet_base::text::js_number(v)
}

fn s(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), js_string)
}

fn list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

// The versions a source's records said, briefly.
fn versions_of(versions: &[Value]) -> String {
    let all: Vec<String> = versions.iter().map(js_string).collect();
    match all.len() {
        0 => "no version in the data".into(),
        1..=3 => format!("version{} {}", if all.len() == 1 { "" } else { "s" }, all.join(", ")),
        k => format!("versions {} to {} ({k})", all[0], all[k - 1]),
    }
}

fn times_of(r: &Value) -> String {
    let mut parts = Vec::new();
    if truthy(r.get("vendorVersion")) {
        parts.push(format!("since {}", s(r.get("vendorVersion"))));
    }
    parts.push(plural(n(r.get("count")), "time"));
    parts.join("; ")
}

/// reportLines(report, { unread, paint, quiet }), painted as xenonet drift
/// paints it: by source, its schema, what was read of it (or `unread`,
/// said of a source nothing was read of; None leaves the line out), its
/// drift, each a change or an addition, and a summary. `quiet` leaves out
/// sources with nothing to say.
pub fn report_lines(report: &Value, unread: Option<&str>, quiet: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let sources = list(report.get("sources"));
    for src in sources {
        let drift = list(src.get("drift"));
        let read = src.get("read").filter(|r| truthy(Some(r)));
        if quiet && read.is_none() && drift.is_empty() && !truthy(src.get("known")) {
            continue;
        }
        let schema = list(src.get("schemas")).first();
        let what = schema.map_or("no schema file".to_string(), |sc| format!("{} ({})", s(sc.get("id")), s(sc.get("range"))));
        lines.push(format!("  {}  {}", c::bold(s(src.get("name"))), c::dim(what)));
        if let Some(r) = read {
            let dropped = if truthy(r.get("dropped")) { format!(", {} with nothing to read", s(r.get("dropped"))) } else { String::new() };
            lines.push(format!(
                "    read     {} ({} checked whole{dropped}), {}",
                plural(n(r.get("records")), "record"),
                locale(n(r.get("checked")), 0, 3),
                versions_of(list(r.get("versions")))
            ));
        } else if let Some(u) = unread {
            lines.push(format!("    read     {u}"));
        }
        for r in drift {
            let label = if is_addition(r) { c::cyan("added   ") } else { c::yellow("changed ") };
            lines.push(format!("    {label} {}  {}", describe(r), c::dim(format!("({})", times_of(r)))));
        }
        if drift.is_empty() && truthy(src.get("known")) {
            lines.push(format!("    {}", c::dim(format!("no drift {} ({} known)", if truthy(report.get("all")) { "kept" } else { "since you last looked" }, s(src.get("known"))))));
        }
    }
    let listed: Vec<&Value> = sources.iter().flat_map(|x| list(x.get("drift"))).collect();
    let when = if truthy(report.get("since")) { format!("you last looked ({})", clock::date_time(n(report.get("since")))) } else { "XenoNet started keeping them".into() };
    lines.push(if truthy(report.get("all")) {
        format!("  {} kept, {} to what XenoNet reads", plural(n(report.get("known")), "difference"), plural(listed.iter().filter(|r| !is_addition(r)).count() as f64, "change"))
    } else {
        format!(
            "  {} new since {when}, {} to what XenoNet reads; {} known in all",
            plural(n(report.get("new")), "difference"),
            plural(n(report.get("changes")), "change"),
            s(report.get("known"))
        )
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_by_source() {
        let report = json!({
            "since": null, "all": false, "new": 1, "changes": 1, "known": 2,
            "sources": [
                { "name": "Codex's rollouts", "schemas": [{ "id": "vendor/openai/codex/rollout-line/v1", "range": "<1.0.0" }], "read": null, "known": 2,
                  "drift": [{ "kind": "missing-field", "path": "payload.cli_version", "vendorVersion": "0.151.0", "count": 1 }] },
                { "name": "Claude desktop's", "schemas": [], "read": null, "known": 0, "drift": [] },
            ],
        });
        assert_eq!(
            report_lines(&report, Some("nothing since XenoNet started"), false),
            [
                "  Codex's rollouts  vendor/openai/codex/rollout-line/v1 (<1.0.0)",
                "    read     nothing since XenoNet started",
                "    changed  payload.cli_version is missing  (since 0.151.0; 1 time)",
                "  Claude desktop's  no schema file",
                "    read     nothing since XenoNet started",
                "  1 difference new since XenoNet started keeping them, 1 change to what XenoNet reads; 2 known in all",
            ]
        );
        assert_eq!(report_lines(&report, None, true).len(), 3, "quiet leaves out what has nothing to say");
        assert_eq!(versions_of(&[json!("1"), json!("2"), json!("3"), json!("4")]), "versions 1 to 4 (4)");
    }
}
