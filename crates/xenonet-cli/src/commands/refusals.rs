//! xenonet refusals (cli/commands/refusals.js): the provider refusals
//! XenoNet kept (core/refusals), from the files in its state folder, so it
//! works with XenoNet stopped: list them, show one, export them for a
//! dataset, add context to ones kept without it, turn keeping context on
//! or off, and scan transcripts for refusals from before XenoNet was
//! watching.

use super::logs::since_of;
use crate::args::Flags;
use crate::clock::date_hour_minute as when;
use crate::config;
use crate::cores;
use crate::js::{have, num, on, str, t};
use crate::out::{c, now, print, write, write_bytes, Done, Fail};
use crate::sys;
use serde_json::{json, Map, Value};
use xenonet_base::json::stringify;

pub const HELP: &str = include_str!("help/refusals.txt");

fn kind_words(kind: &str) -> String {
    match kind {
        "refusal" => "refused".into(),
        "refusal-recovered" => "refused, another model answered".into(),
        "reroute" => "moved to another model".into(),
        "possible-refusal" => "rejected the prompt".into(),
        k => k.into(),
    }
}

fn plural_s(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

// The lines a refusal might be in, of the text that might hold one.
fn may_hold(text: &str) -> bool {
    text.contains("refusal") || text.contains("codex_error_info") || text.contains("model_reroute")
}

// A transcript's session: its app and the uuid its file is named by
// (/([0-9a-f-]{36})\.jsonl$/i), or else the file's name.
fn session_of(app: &str, file: &str) -> String {
    let stem = file.strip_suffix(".jsonl").unwrap_or(file);
    let tail: Vec<char> = stem.chars().rev().take(36).collect();
    let id = (file.ends_with(".jsonl") && tail.len() == 36 && tail.iter().all(|c| c.is_ascii_hexdigit() || *c == '-')).then(|| tail.iter().rev().collect::<String>());
    let name = sys::base_name(file);
    format!("{app}:{}", id.unwrap_or_else(|| name.strip_suffix(".jsonl").unwrap_or(&name).to_string()))
}

fn scan(dir: &str, flags: &Flags) -> Done {
    let from = if flags.on("since") { since_of(flags.get("since"), now())? } else { now() - 90.0 * 86400e3 };
    let roots = [("claude", config::claude_projects_dir()), ("codex", sys::join(&config::codex_home(), "sessions"))];
    let (mut files, mut found) = (0usize, 0usize);
    for (app, root) in &roots {
        for file in xenonet_base::paths::list_files(root, &mut |n: &str| n.ends_with(".jsonl"), 6) {
            let Ok(st) = std::fs::metadata(&file) else { continue };
            if xenonet_base::fs::mtime_ms_of(&st).unwrap_or(0.0) < from {
                continue;
            }
            files += 1;
            let Ok(bytes) = std::fs::read(&file) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            // Most lines can't be one; only these are read.
            if !may_hold(&text) {
                continue;
            }
            for line in text.split('\n').filter(|l| may_hold(l)) {
                let Ok(raw) = xenonet_base::json::parse(line) else { continue };
                let Some(sig) = cores::refusal_signal(app, &raw)?.filter(|s| !s.is_null()) else { continue };
                if on(&sig, "at") && num(&sig, "at") < from {
                    continue;
                }
                let agent = if file.contains("subagents") { json!(sys::base_name(&file).trim_end_matches(".jsonl")) } else { Value::Null };
                let cwd = raw.get("cwd").filter(|c| c.is_string()).cloned().unwrap_or(Value::Null);
                let at = json!({ "sessionId": session_of(app, &file), "app": app, "agentId": agent, "file": file, "cwd": cwd });
                if cores::refusals_record(dir, &sig, &at)?.is_some_and(|r| !r.is_null()) {
                    found += 1;
                }
            }
        }
    }
    let without = if found > 0 && !cores::refusals_context_on(dir)? { c::dim(" (without context: xenonet refusals context on, then capture --all)") } else { String::new() };
    print(format!("{} Looked through {}; kept {} XenoNet didn't have{without}.", c::green("✓"), plural_s(files, "transcript"), plural_s(found, "refusal")));
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let dir = sys::join(&sys::state_dir(flags.on("dev")), "refusals");
    let sub = args.first().map_or("list", String::as_str);
    let id = args.get(1).filter(|x| !x.is_empty());
    let since = if flags.on("since") { since_of(flags.get("since"), now())? } else { 0.0 };
    let provider = flags.text("provider").map(|p| match p.as_str() {
        "claude" => "anthropic".to_string(),
        "codex" => "openai".to_string(),
        _ => p,
    });
    let on_off = |on: bool| if on { "on" } else { "off" };

    match sub {
        "context" => {
            let Some(want) = id.filter(|x| *x == "on" || *x == "off") else {
                let now_on = cores::refusals_context_on(&dir)?;
                print(format!("Keeping context is {}. xenonet refusals context on|off changes it.", if now_on { c::green("on") } else { "off".into() }));
                return Ok(0);
            };
            cores::refusals_set_context(&dir, want == "on")?;
            print(if want == "on" {
                format!("{} XenoNet keeps each new refusal's context: the transcript up to it, in its state folder. Transcripts can hold secrets; export --scrub takes home folders and keys out.", c::green("✓"))
            } else {
                format!("{} XenoNet keeps only the refusal itself from now on.", c::green("✓"))
            });
            Ok(0)
        }
        "list" => {
            let all = cores::refusals_list(&dir, since, provider.as_deref())?;
            if flags.on("json") {
                let lines: Vec<String> = all
                    .iter()
                    .map(|r| {
                        let mut m: Map<String, Value> = r.as_object().cloned().unwrap_or_default();
                        let source = m.shift_remove("source");
                        m.shift_remove("key");
                        m.insert(
                            "source".into(),
                            source.filter(|s| xenonet_base::text::truthy(Some(s))).map_or(Value::Null, |s| json!({ "file": s.get("file").cloned().unwrap_or(Value::Null) })),
                        );
                        stringify(&Value::Object(m), 0)
                    })
                    .collect();
                print(lines.join("\n"));
                return Ok(0);
            }
            if all.is_empty() {
                print(c::dim(format!("No refusals kept{}. Keeping context is {}.", if since != 0.0 { " in that time" } else { "" }, on_off(cores::refusals_context_on(&dir)?))));
                return Ok(0);
            }
            for r in &all {
                let signal = r.get("signal").cloned().unwrap_or(Value::Null);
                let session = r.get("session").cloned().unwrap_or(Value::Null);
                let category = if on(&signal, "category") { format!(" · {}", t(&signal, "category")) } else { String::new() };
                let model = if on(&session, "model") { c::dim(format!(" · {}", t(&session, "model"))) } else { String::new() };
                let context = if on(r, "context") { c::cyan(" · context") } else { String::new() };
                let said = have(&signal, "explanation")
                    .map(|e| crate::js::text(Some(e)))
                    .or_else(|| str(&signal, "text").map(|x| x.split('\n').next().unwrap_or("").to_string()))
                    .unwrap_or_default();
                let who = have(r, "app").map_or_else(|| t(r, "provider"), |a| crate::js::text(Some(a)));
                print(format!("{}  {}  {who} {}{category}{model}{context}\n  {}", c::dim(when(num(r, "at"))), c::bold(t(r, "id")), kind_words(&t(r, "kind")), c::dim(said)));
            }
            print(c::dim(format!("\n{} kept. Keeping context is {}.", all.len(), on_off(cores::refusals_context_on(&dir)?))));
            Ok(0)
        }
        "show" => {
            let r = match id {
                Some(id) => cores::refusals_get(&dir, id)?.filter(|r| xenonet_base::text::truthy(Some(r))),
                None => None,
            };
            let Some(r) = r else { return Err(Fail::error(format!("no refusal {}; xenonet refusals lists them", id.map_or("", String::as_str)))) };
            if flags.on("context") {
                if !on(&r, "context") {
                    return Err(Fail::error("it was kept without context; xenonet refusals capture adds it while its transcript is still there"));
                }
                write_bytes(&cores::refusals_context(&dir, &t(&r, "id"))?);
                return Ok(0);
            }
            let mut rest = r.as_object().cloned().unwrap_or_default();
            rest.shift_remove("key");
            print(stringify(&Value::Object(rest), 2));
            Ok(0)
        }
        "export" => {
            let lines =
                cores::refusals_export_lines(&dir, &json!({ "since": since, "provider": provider, "withContext": flags.on("withContext"), "scrubbed": flags.on("scrub") }))?;
            let text = lines.iter().map(|l| stringify(l, 0)).collect::<Vec<_>>().join("\n") + if lines.is_empty() { "" } else { "\n" };
            let Some(out) = flags.text("out") else {
                write(&text);
                return Ok(0);
            };
            std::fs::write(&out, &text).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &out).message))?;
            print(format!("{} Wrote {} to {out}{}.", c::green("✓"), plural_s(lines.len(), "refusal"), if flags.on("scrub") { ", scrubbed" } else { "" }));
            Ok(0)
        }
        "capture" => {
            let ids: Vec<String> = if flags.on("all") {
                cores::refusals_list(&dir, 0.0, None)?.iter().filter(|r| !on(r, "context")).map(|r| t(r, "id")).collect()
            } else {
                id.map(|x| vec![x.clone()]).unwrap_or_default()
            };
            if ids.is_empty() {
                return Err(Fail::error("say which: xenonet refusals capture <id>, or --all"));
            }
            let mut added = 0;
            for x in &ids {
                let r = cores::refusals_capture(&dir, x)?;
                if on(&r, "ok") && !on(&r, "already") {
                    added += 1;
                } else if !on(&r, "ok") && !flags.on("all") {
                    return Err(Fail::error(format!("{x}: {}", t(&r, "why"))));
                }
            }
            print(format!("{} Added context to {added} of {}.", c::green("✓"), ids.len()));
            Ok(0)
        }
        "scan" => scan(&dir, flags),
        _ => Err(Fail::error(format!("no such: xenonet refusals {sub}. xenonet refusals --help lists them."))),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sessions_by_their_files() {
        assert_eq!(super::session_of("claude", "/p/-w-atlas/0190d3e0-0000-7000-8000-00000000aa01.jsonl"), "claude:0190d3e0-0000-7000-8000-00000000aa01");
        assert_eq!(super::session_of("codex", "/s/2026/10/rollout-x.jsonl"), "codex:rollout-x");
    }
}
