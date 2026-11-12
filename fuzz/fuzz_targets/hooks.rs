#![no_main]

// A hook's payload as the engine reads it: Claude Code, Codex and Grok
// Build post one for each event, with the tool an agent is about to run
// and its input, and the engine works out which files an edit touches, warns about other
// agents' claims on them, and hands over mail. The engine's collaborators
// (the map, the clock, the repos, the waits) are stand-ins that have
// nothing to say, over an empty store in memory, so the payload is all
// there is. The first byte picks the app; a claim pattern after a NUL is
// matched against each file the edit names.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_engine::engine::{self, Engine, Result};
use xenonet_engine::host::{Clock, Collaborators, Events, IndexEntry, Listening, MapReader, Processes, Repo, Repos, Waits};
use xenonet_service::compose::runtime::store::Store;
use xenonet_store::engine::{EngineStore, Shared};

const NOW: f64 = 1_791_230_000_000.0;

struct Quiet;

impl Clock for Quiet {
    fn now(&self) -> f64 {
        NOW
    }
    fn time_of(&self, _: f64) -> String {
        "9:04 AM".into()
    }
}

impl Processes for Quiet {
    fn alive(&self, _: f64) -> bool {
        false
    }
}

impl MapReader for Quiet {
    fn live_ids(&self) -> Result<Vec<(String, f64)>> {
        Ok(Vec::new())
    }
    fn index(&self) -> Result<Vec<IndexEntry>> {
        Ok(Vec::new())
    }
    fn node(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn live(&self, _: &str, _: f64) -> Result<Option<Value>> {
        Ok(None)
    }
    fn observed_live(&self, _: &str) -> Result<bool> {
        Ok(false)
    }
    fn home_of(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn identities(&self) -> Result<Vec<Value>> {
        Ok(Vec::new())
    }
}

impl Repos for Quiet {
    fn of(&self, _: &str) -> Result<Option<Repo>> {
        Ok(None)
    }
}

impl Listening for Quiet {
    fn set(&self, _: &str, _: f64) {}
    fn delete(&self, _: &str) {}
    fn has(&self, _: &str) -> bool {
        false
    }
}

// A listening Stop hook waits until the clock passes its deadline, and
// this clock stands still: the request is gone at once instead.
impl Waits for Quiet {
    fn wake_waiters(&self, _: &str) {}
    fn wait(&self, _: &str, _: f64) {}
    fn listen_wait(&self, _: &str, _: f64) {}
    fn aborted(&self) -> bool {
        true
    }
}

impl Events for Quiet {
    fn emit(&self, _: &str, _: Value) {}
}

fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let (payload, pattern) = xenonet_fuzz::split(rest, 0);
    let Some(input) = xenonet_fuzz::json(payload) else { return };
    let pattern = xenonet_fuzz::text(pattern);
    xenonet_fuzz::run(payload, || {
        let store = Store(Shared::new(EngineStore::memory(NOW).expect("a store in memory")));
        let q = Quiet;
        let c = Collaborators {
            store: &store,
            clock: &q,
            processes: &q,
            map: &q,
            actors: None,
            delivery: None,
            policy: None,
            repos: &q,
            listening: &q,
            waits: &q,
            events: &q,
            net: None,
            projects: None,
        };
        let app = ["claude", "codex", "grok"][usize::from(pick) % 3];
        let event = field(&input, "hook_event_name").and_then(Value::as_str);
        let _ = Engine::default().hook(&c, app, event, &input);
        let tool = field(&input, "tool_name").and_then(Value::as_str).unwrap_or("");
        let cwd = field(&input, "cwd").and_then(Value::as_str);
        for f in engine::edit_paths(tool, field(&input, "tool_input").unwrap_or(&Value::Null), cwd) {
            let _ = engine::matches(&pattern, &f);
        }
        if let Some(id) = field(&input, "session_id").and_then(Value::as_str) {
            let _ = engine::is_agent_id(id);
        }
    });
});
