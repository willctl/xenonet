//! xenonet start (docs/guide/network.md, "Starting sessions"): a new
//! Claude Code, Codex or Grok Build session, here or on another machine of
//! the network, with a first prompt, through the local service's
//! /api/start. It's yours: the service refuses it from under an agent,
//! which asks with the xenonet MCP server's start_session tool instead.

use crate::args::Flags;
use crate::js::{str, t};
use crate::out::{c, json, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Map, Value};

pub const HELP: &str = include_str!("help/start.txt");

pub fn run(args: &[String], flags: &Flags) -> Done {
    let prompt = args.join(" ");
    if prompt.trim().is_empty() {
        return Err(Fail::new("say what the new session should do: xenonet start [--on <machine>] \"<prompt>\"", 2));
    }
    let mut body = Map::new();
    body.insert("prompt".into(), json!(prompt));
    for (flag, key) in [("on", "machine"), ("in", "folder"), ("app", "app"), ("mode", "mode")] {
        if let Some(v) = flags.text(flag) {
            body.insert(key.into(), json!(v));
        }
    }
    let s = need(flags)?;
    let v: Value = s.request("POST", "/api/start", Some(&Value::Object(body)), &[]).map_err(|r| Fail::new(r.message, 1))?;
    if flags.on("json") {
        json(&v);
        return Ok(0);
    }
    if str(&v, "state") == Some("sent") {
        print(format!("{} sent to {}: XenoNet says here when it starts, or why it didn't, and again when it ends", c::green("✓"), t(&v, "host")));
        return Ok(0);
    }
    let session = str(&v, "session").map_or_else(|| "a Codex session (its id comes when Codex names it)".to_string(), str::to_string);
    print(format!("{} started {session} in {}, pid {}", c::green("✓"), t(&v, "folder"), t(&v, "pid")));
    Ok(0)
}
