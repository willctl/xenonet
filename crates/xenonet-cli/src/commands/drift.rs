//! xenonet drift (cli/commands/drift.js): what's changed in the other apps'
//! data XenoNet reads, by source, since you last looked (core/drift.js,
//! core/vendor.js): from a running XenoNet, or with none running, from
//! drift.json in the cache folder. Then it marks that seen, which quiets
//! the doctor's contracts.drift check. Only this and the console mark
//! drift seen: npm run contracts reports it too, but agents run that, and
//! you'd miss what they saw. With --exit-code it's left unmarked, so a
//! script goes on exiting 1 until you've looked.

use crate::args::Flags;
use crate::config::{config, instance_dir, stands_for};
use crate::js::{on, t};
use crate::out::{c, json, now, print, Done, Fail};
use crate::reach::find;
use crate::report::{drift_report, report_lines, schemas_dir};
use serde_json::{json, Value};
use xenonet_base::client::name_of;
use xenonet_schema::drift::{DriftLog, DRIFT_FILE};

pub const HELP: &str = include_str!("help/drift.txt");

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let all = flags.on("all");
    let mark = !flags.on("exitCode");
    let s = find(flags);
    let mut report = None;
    if let Some(s) = &s {
        match s.get(&format!("/api/drift{}", if all { "?all=1" } else { "" })) {
            Ok(r) => report = Some(r),
            // A XenoNet from before the route: its drift.json is read instead.
            Err(e) if e.status == Some(404) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let report = match (report, &s) {
        (Some(mut r), Some(s)) => {
            if let Value::Object(m) = &mut r {
                m.insert("by".into(), json!(name_of(s)));
            }
            r
        }
        _ => from_file(all, flags, mark)?,
    };
    let by = report.get("by").and_then(Value::as_str).map(str::to_string);

    if flags.on("json") {
        json(&report);
    } else {
        print(match &by {
            Some(by) => format!("{} {}", c::bold(by), c::dim(format!("· drift kept in {}", t(&report, "file")))),
            None => c::dim(format!("XenoNet isn't running, so this is the drift it kept in {}", t(&report, "file"))),
        });
        print("");
        // A running XenoNet says what it has read since it started; from the
        // file nothing was read, so only the sources with drift show.
        let lines = if by.is_some() { report_lines(&report, Some("nothing since XenoNet started"), false) } else { report_lines(&report, None, true) };
        for line in lines {
            print(line);
        }
        print("");
    }
    // Seen once it's shown, up to when it was made: what turned up since
    // stays new. (The file was marked as it was read.)
    if let (Some(s), true, true) = (&s, by.is_some(), mark) {
        s.post("/api/drift/seen", &json!({ "at": report.get("at").cloned().unwrap_or(Value::Null) }), &[])?;
    }
    let fresh = on(&report, "new");
    if !flags.on("json") {
        print(c::dim(if !fresh {
            "Nothing new since you last looked."
        } else if mark {
            "Marked seen: the doctor says nothing more of these until something new turns up."
        } else {
            "Left new (--exit-code): plain xenonet drift or the console marks it seen."
        }));
    }
    // New as of the report, before it was marked seen.
    Ok(if flags.on("exitCode") && fresh { 1 } else { 0 })
}

/// The report from drift.json itself, which is then marked seen there
/// (with `mark`): in the cache folder of the XenoNet asked about (XenoNet
/// Dev keeps its own).
fn from_file(all: bool, flags: &Flags, mark: bool) -> Result<Value, Fail> {
    let cfg = config()?;
    let dir = match stands_for(&flags.pick()) {
        Some(name) => instance_dir("cache", name),
        None => cfg.cache_dir.clone(),
    };
    let mut log = DriftLog::default();
    log.open(&xenonet_base::paths::join(&dir, DRIFT_FILE));
    let mut report = drift_report(&mut log, all, schemas_dir().as_deref(), Some(&cfg.home_dir), now());
    if mark {
        log.looked(report["at"].as_f64().unwrap_or(0.0));
        log.flush();
    }
    if let Value::Object(m) = &mut report {
        m.insert("by".into(), Value::Null);
    }
    Ok(report)
}
