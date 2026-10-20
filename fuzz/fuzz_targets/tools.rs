#![no_main]

// What other tools print or keep, as XenoNet reads it: gh's hosts.yml, an
// ssh config's aliases, git's remotes and config, ps, lsof and netstat on
// the processes at the other end of a connection, tmux's panes, Codex's
// picks for the projects folder, a delivery selector an agent wrote, and
// the system's GPU and memory stats. The first byte picks one.

use libfuzzer_sys::fuzz_target;
use serde_json::json;
use xenonet_engine::delivery::{opencode, selector, tmux};
use xenonet_engine::sql::Arg;
use xenonet_engine::{engine, procs};
use xenonet_state::{classify, identities};

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let text = xenonet_fuzz::text(rest);
    xenonet_fuzz::run(rest, || match pick % 6 {
        0 => {
            let _ = identities::parse_gh_hosts(&text);
            let aliases = identities::parse_ssh_aliases(&text);
            for line in text.lines() {
                let _ = identities::parse_remote(line, &aliases);
            }
            let _ = identities::parse_git_config(&text);
        }
        1 => {
            let _ = (procs::parse_texts(&text), procs::parse_cwd(&text), procs::parse_cwds(&text), procs::parse_commands(&text), procs::parse_ps(&text));
            let _ = procs::parse_netstat(&text, "47840", "51234");
            let _ = engine::parse_repo(&text);
        }
        2 => {
            let _ = tmux::parse_panes(&text);
            let _ = opencode::parse_ps(&text);
        }
        3 => {
            let outline = [json!({ "path": "labs/snippets", "xenonet": "folder", "markers": [] }), json!({ "path": "atlas-api", "xenonet": "repo", "markers": [] }), json!({ "path": "labs", "xenonet": "group" })];
            let _ = classify::parse_picks(&text, &outline);
        }
        4 => {
            let _ = selector::parse_selector(&Arg::text(text.clone()));
        }
        _ => {
            for line in text.lines() {
                let _ = xenonet_app::system_stats::parse_nvidia(line);
                let _ = xenonet_app::system_stats::parse_windows_gpu(line, &[("luid_0x0000_0x1234".into(), 8e9)]);
            }
            let _ = xenonet_app::system_stats::parse_ioreg(&text, Some(16e9));
        }
    });
});
