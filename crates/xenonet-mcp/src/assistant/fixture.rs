//! `xenonet mcp assistant --fixture`'s backend (core/assistant/service.js's
//! createFixtureService): the assistant's service in this process, over
//! made-up data (core/assistant/fixtures.js), through the same checks a
//! real grant's reads go through: the request's shape, the rate and the
//! four-at-a-time limit, the arguments' schema, and the answer's. No grant,
//! no XenoNet and nothing of this computer's.

use super::schema;
use serde_json::{json, Value};
use std::rc::Rc;
use std::sync::Mutex;
use xenonet_actors::limits::Buckets;
use xenonet_app::task::{run_with, Failure, Task};
use xenonet_assistant::cursors::Cursors;
use xenonet_assistant::fixtures;
use xenonet_assistant::service::{self, Config, Inflight, LIMITS};

/// The made-up session's grant, actor and connection.
const GRANT: &str = "gr_000000000000000000000000";
// The service's rate for a grant: one a second, ten at once.
const RATE: (f64, f64) = (1.0, 10.0);

/// What a read came to: its result envelope, or an error one.
pub struct Answer {
    pub ok: bool,
    pub envelope: Value,
}

struct State {
    buckets: Buckets,
    inflight: Inflight,
}

pub struct Fixture {
    version: String,
    /// The page tokens' key, made when the service starts and never
    /// written down.
    key: Vec<u8>,
    policy: Value,
    state: Mutex<State>,
}

fn now() -> f64 {
    super::now()
}

impl Fixture {
    pub fn new(version: &str) -> Fixture {
        let mut key = vec![0u8; 32];
        let _ = getrandom::fill(&mut key);
        Fixture { version: version.into(), key, policy: fixtures::standalone_policy(true), state: Mutex::new(State { buckets: Buckets::new(1024), inflight: Inflight::default() }) }
    }

    fn config(&self) -> Config {
        Config {
            operations: schema::OPERATIONS.iter().map(|(t, o)| (t.to_string(), o.to_string())).collect(),
            attention_kinds: schema::ATTENTION_KINDS.iter().map(|k| k.to_string()).collect(),
            result_schema: schema::RESULT_SCHEMA.into(),
            error_schema: schema::ERROR_SCHEMA.into(),
            version: self.version.clone(),
            limits: LIMITS,
        }
    }

    fn no(&self, request_id: &Value, code: &str, retry_after_ms: Option<f64>) -> Answer {
        Answer { ok: false, envelope: service::error_envelope(schema::ERROR_SCHEMA, Some(request_id), code, retry_after_ms) }
    }

    /// service.read(session, { request_id, operation, arguments }).
    pub fn read(&self, operation: &str, args: &Value, request_id: &str) -> Answer {
        let cfg = self.config();
        let b = service::begin(&cfg, &json!({ "request_id": request_id, "operation": operation, "arguments": args }));
        if let Some(no) = &b.no {
            return self.no(&b.request_id, no.code, None);
        }
        let (Some(op), Some(tool)) = (b.op.clone(), b.tool.clone()) else { return self.no(&b.request_id, "INVALID_ARGUMENT", None) };
        {
            let mut s = self.state.lock().expect("the service's state");
            let wait = s.buckets.take(GRANT, RATE.0, RATE.1, 1.0, now());
            if wait != 0.0 {
                return self.no(&b.request_id, "RATE_LIMITED", Some(wait));
            }
            if !s.inflight.enter(GRANT, cfg.limits.concurrent) {
                return self.no(&b.request_id, "RATE_LIMITED", Some(1000.0));
            }
        }
        let out = self.answer(cfg, &op, &tool, &b.args, &b.request_id);
        self.state.lock().expect("the service's state").inflight.leave(GRANT);
        match out {
            Ok(envelope) => Answer { ok: true, envelope },
            Err(code) => self.no(&b.request_id, &code, None),
        }
    }

    // The arguments checked again, then the answer as a task, its
    // requests answered here: the time, the made-up view and stats, the
    // page tokens' clock and the check of the answer against its schema.
    fn answer(&self, cfg: Config, op: &str, tool: &str, args: &Value, request_id: &Value) -> Result<Value, String> {
        let parsed = schema::input(tool).parse(args).map_err(|_| "INVALID_ARGUMENT".to_string())?;
        let cursors = Rc::new(Cursors::new(Some(self.key.clone())));
        let (cfg, op, tool, policy, rid) = (Rc::new(cfg), op.to_string(), tool.to_string(), self.policy.clone(), request_id.clone());
        let task = Task::new(move |io| async move { service::answer(io, cfg, cursors, op, tool, parsed, policy, rid).await });
        let done = run_with(task, |q| match q.get("op").and_then(Value::as_str) {
            Some("now" | "cursorNow") => Ok(xenonet_base::json::number(now())),
            Some("view") => Ok(fixtures::fixture_view(now())),
            Some("actor") => Ok(Value::Null),
            Some("stats") => Ok(fixtures::fixture_stats(&q.get("id").map(xenonet_base::text::js_string).unwrap_or_default())),
            Some("idKey") => Ok(json!("fixture".as_bytes())),
            Some("check") => {
                let tool = q.get("tool").and_then(Value::as_str).unwrap_or("");
                Ok(json!(schema::result(tool).parse(q.get("envelope").unwrap_or(&Value::Null)).is_ok()))
            }
            other => Err(json!({ "message": format!("no such request: {}", other.unwrap_or("undefined")) })),
        });
        match done {
            Ok(out) => Ok(out.get("envelope").cloned().unwrap_or(Value::Null)),
            Err(Failure::Error { fields, .. }) if fields.get("refused") == Some(&json!(true)) => {
                Err(fields.get("code").and_then(Value::as_str).unwrap_or("INTERNAL_ERROR").to_string())
            }
            Err(_) => Err("INTERNAL_ERROR".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test/assistant-endpoint.test.js's --fixture case, its reads: each
    // tool answers made-up data that fits its schema.
    #[test]
    fn each_tool_answers_made_up_data() {
        let f = Fixture::new("0.0.0-test");
        for (tool, op) in schema::OPERATIONS {
            let args = if tool == "xenonet_session" { json!({ "session_id": "claude:0190fa00-0000-7000-8000-000000000001" }) } else { json!({}) };
            let r = f.read(op, &args, "r_00000000-0000-4000-8000-000000000000");
            assert!(r.ok, "{tool}: {}", r.envelope);
            assert!(r.envelope["warnings"].as_array().unwrap().iter().any(|w| w["code"] == json!("FIXTURE_DATA")), "{tool}");
        }
        let bad = f.read("sessions", &json!({ "path": "/etc" }), "r_00000000-0000-4000-8000-000000000001");
        assert_eq!((bad.ok, bad.envelope["code"].clone()), (false, json!("INVALID_ARGUMENT")));
    }
}
