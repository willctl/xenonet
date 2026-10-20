#![no_main]

// A hook's payload as the engine reads it: Claude Code and Codex post one
// for each event, with the tool an agent is about to run and its input,
// and the engine works out which files an edit touches, warns about other
// agents' claims on them, and hands over mail. The engine's collaborators
// (its store, the clock) are stand-ins that have nothing to say, so the
// payload is all there is. The first byte picks the app; a claim pattern
// after a NUL is matched against each file the edit names.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_engine::engine::{self, Engine};
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;

struct Quiet;

impl Calls for Quiet {
    fn call(&mut self, path: &str, method: &str, _: Vec<Arg>) -> Result<Arg> {
        Ok(match (path, method) {
            (_, "now") => Arg::Num(1_791_230_000_000.0),
            _ => Arg::Undef,
        })
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("undefined".into())
    }
}

fn arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), arg(v))).collect()),
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let (payload, pattern) = xenonet_fuzz::split(rest, 0);
    let Some(body) = xenonet_fuzz::json(payload) else { return };
    let pattern = Arg::text(xenonet_fuzz::text(pattern));
    xenonet_fuzz::run(payload, || {
        let input = arg(&body);
        let app = Arg::text(if pick & 1 == 0 { "claude" } else { "codex" });
        let event = input.field("hook_event_name");
        let _ = Engine::default().hook(&mut Quiet, &app, &event, &input);
        for f in engine::edit_paths(&input.field("tool_name"), &input.field("tool_input"), &input.field("cwd")) {
            let _ = engine::matches(&pattern, &Arg::text(f));
        }
        let _ = engine::is_agent_id(&input.field("session_id"));
    });
});
