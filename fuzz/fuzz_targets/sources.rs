#![no_main]

// The rest of what XenoNet reads from other programs: the Codex
// app-server's stdio and threads, Claude desktop's and the Agent SDK's
// session records, git's output, plan usage from Anthropic and Claude
// desktop, OpenRouter's usage, tool inputs the live map reads, the
// xenolith context's environment, refusals XenoNet kept, and runs actors
// report. The first byte picks a group; NULs split the rest.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use xenonet_base::json;
use xenonet_sources::codex::{self, RolloutEntries, Stderr, Stdout};
use xenonet_sources::refusals::query;
use xenonet_sources::reported_runs::{self, ReportedRuns};
use xenonet_sources::{claude, claude_usage, git, live, openrouter_usage, xenolith};

const NOW: f64 = 1_791_230_000_000.0;

fn values(text: &str) -> Vec<Value> {
    text.split('\n').filter_map(|l| json::parse(l).ok()).collect()
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let input = xenonet_fuzz::text(rest);
    let mut args = input.split('\0');
    let a = args.next().unwrap_or_default();
    let b = args.next().unwrap_or_default();
    let va = json::parse(a).ok();
    let vb = json::parse(b).ok();
    xenonet_fuzz::run(rest, || match pick % 9 {
        0 => {
            // The app-server's stdout, in the pieces the pipe hands over.
            let mut out = Stdout::default();
            let cut = usize::from(pick / 9) % (a.len() + 1);
            let cut = (0..=cut).rev().find(|&i| a.is_char_boundary(i)).unwrap_or(0);
            let _ = out.take(&a[..cut]);
            let _ = out.take(&a[cut..]);
            let _ = out.take("\n");
            let mut err = Stderr::default();
            err.push(b);
            let _ = err.exited(vb.as_ref());
            let _ = codex::says_unsupported(va.as_ref());
        }
        1 => {
            let _ = codex::sub_agent_of(va.as_ref());
            let _ = codex::thread_to_session(va.as_ref(), pick & 16 != 0);
            let _ = codex::normalize_item(va.as_ref());
            let _ = codex::normalize_items(va.as_ref());
            let _ = codex::native_projects(va.as_ref());
            let _ = codex::rollout_page(usize::from(pick), vb.as_ref(), 20.0);
            let _ = codex::other_state_dbs(&b.split('/').map(str::to_string).collect::<Vec<_>>());
        }
        2 => {
            let mut entries = RolloutEntries::default();
            for v in values(a) {
                let _ = entries.add(v);
            }
            let _ = entries.list();
        }
        3 => {
            let listed = values(a);
            let desktop = values(b);
            let _ = claude::sessions(&listed, &desktop);
            let _ = claude::entries(&listed);
            let records =
                desktop.iter().enumerate().map(|(i, v)| (format!("/home/nova/claude-desktop/local_{i}.json"), vec!["acct".to_string(), "org".to_string()], v.clone())).collect();
            let _ = claude::desktop_index(records);
            if let Some(v) = va.clone() {
                let _ = claude::renamed_record(v, vb.as_ref(), vb.as_ref(), Some(&json!("Fix it")));
            }
            let _ = claude::page(listed.len(), vb.as_ref(), 10.0);
        }
        4 => {
            let _ = git::parse_status(a);
            let _ = git::filter_overrides(a, pick & 16 != 0);
            let _ = git::after_drivers(Some(0.0), false, a, pick & 16 != 0);
            let _ = git::problem_of(&b.split('\n').map(str::to_string).collect::<Vec<_>>());
        }
        5 => {
            let _ = claude_usage::token_of(Some(a));
            let _ = claude_usage::parse_usage(va.as_ref(), NOW);
            let _ = claude_usage::reading(va.as_ref(), NOW);
            let _ = claude_usage::sign_in_problem(va.as_ref(), NOW);
            let _ = claude_usage::status_problem(va.as_ref(), pick & 16 != 0, vb.as_ref());
            let _ = claude_usage::next_in(va.as_ref());
            let _ = claude_usage::desktop_version(va.as_ref());
            let _ = claude_usage::desktop_samples(va.as_ref(), va.as_ref());
            let _ = claude_usage::unreachable(va.as_ref(), vb.as_ref());
            let _ = openrouter_usage::usage_of(va.as_ref());
            let _ = openrouter_usage::reading(true, vb.as_ref().map(|v| (v, pick & 16 != 0)), va.as_ref(), NOW);
        }
        6 => {
            let input = va.as_ref();
            let _ = (live::string_of(input), live::mcp_server_of(b), live::categorize(b), live::clip(a, 40), live::clip_value(input, 40));
            let _ = (live::tail(a, 40), live::day_key(NOW), live::file_key(a), live::looks_like_question(a));
            let _ = (live::tool_detail(input, Some("/home/nova")), live::command_text(input), live::files_of(input), live::patch_files(input));
            let _ = (live::diff_lines("+", input, 20), live::claude_edits(b, input), live::codex_edits(input), live::tool_key(b, input));
            let _ = live::wiki_pages_of(Some(b), input);
        }
        7 => {
            if let Some(v) = &va {
                let _ = xenolith::env_of(v);
            }
            let _ = (xenolith::is_context_server(a), xenolith::context_name(Some(a)));
            let records = values(a);
            let _ = query::query_refusals(&records, vb.as_ref().unwrap_or(&Value::Null));
            for r in &records {
                let _ = (query::public_record(r), query::model_of(r), query::category_of(r));
            }
        }
        _ => {
            let mut runs = ReportedRuns::new();
            let mut t = NOW;
            let mut now = || {
                t += 1000.0;
                t
            };
            for line in a.split('\n') {
                let mut f = line.split('\t');
                let (actor, run, app, title, presence) = (f.next().unwrap_or(""), f.next(), f.next().unwrap_or("claude"), f.next(), f.next());
                let _ = reported_runs::refusal(run, app, presence);
                let _ = runs.report(actor, run, app, title, presence, &mut now);
            }
            let _ = runs.sessions(NOW + 3_600_000.0);
            let ids: Vec<Option<Value>> = values(b).into_iter().map(Some).collect();
            let _ = reported_runs::firsts(&ids);
        }
    });
});
