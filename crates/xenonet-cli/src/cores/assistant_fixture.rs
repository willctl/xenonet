//! `xenonet mcp assistant --fixture`'s backend: the assistant's service in
//! this process, over made-up data (xenonet-assistant's fixtures), through the same checks a
//! real grant's reads go through: the request's shape, the rate and the
//! four-at-a-time limit, the arguments' schema, and the answer's (the MCP
//! server's `contract`). No grant, no XenoNet and nothing of this
//! computer's.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_api::limits::Buckets;
use xenonet_assistant::fixtures;
use xenonet_assistant::pages::Standalone;
use xenonet_assistant::service::{self, Config, Inflight, ReadHost, LIMITS};
use xenonet_mcp::fixture::contract as schema;
use xenonet_mcp::fixture::Answer;
use xenonet_os::error::Failure;

/// The made-up session's grant, actor and connection.
const GRANT: &str = "gr_000000000000000000000000";
const ACTOR: &str = "fixture-client";
// The service's rate for a grant: one a second, ten at once.
const RATE: (f64, f64) = (1.0, 10.0);

struct State {
    buckets: Buckets,
    inflight: Inflight,
}

pub struct Fixture {
    version: String,
    /// The page cursors: a grant's token minted in memory when the
    /// service starts, narrowed, and good nowhere else.
    pages: Standalone,
    policy: Value,
    state: Mutex<State>,
}

fn now() -> f64 {
    xenonet_text::time::now_ms()
}

impl Fixture {
    pub fn new(version: &str) -> Fixture {
        Fixture {
            version: version.into(),
            pages: Standalone::new(ACTOR),
            policy: fixtures::standalone_policy(true),
            state: Mutex::new(State { buckets: Buckets::new(1024), inflight: Inflight::default() }),
        }
    }

    fn config(&self) -> Config {
        Config {
            operations: schema::OPERATIONS.iter().map(|(t, o)| (t.to_string(), o.to_string())).collect(),
            attention_kinds: schema::ATTENTION_KINDS.iter().map(ToString::to_string).collect(),
            result_schema: schema::RESULT_SCHEMA.into(),
            error_schema: schema::ERROR_SCHEMA.into(),
            version: self.version.clone(),
            limits: LIMITS,
        }
    }

    fn no(request_id: &Value, code: &str, retry_after_ms: Option<f64>) -> Answer {
        Answer { ok: false, envelope: service::error_envelope(schema::ERROR_SCHEMA, Some(request_id), code, retry_after_ms) }
    }

    /// service.read(session, { request_id, operation, arguments }).
    fn read_one(&self, operation: &str, args: &Value, request_id: &str) -> Answer {
        let cfg = self.config();
        let b = service::begin(&cfg, &json!({ "request_id": request_id, "operation": operation, "arguments": args }));
        if let Some(no) = &b.no {
            return Self::no(&b.request_id, no.code, None);
        }
        let (Some(op), Some(tool)) = (b.op.clone(), b.tool.clone()) else { return Self::no(&b.request_id, "INVALID_ARGUMENT", None) };
        {
            let mut s = self.state.lock().expect("the service's state");
            let wait = s.buckets.take(GRANT, RATE.0, RATE.1, 1.0, now());
            if wait != 0.0 {
                return Self::no(&b.request_id, "RATE_LIMITED", Some(wait));
            }
            if !s.inflight.enter(GRANT, cfg.limits.concurrent) {
                return Self::no(&b.request_id, "RATE_LIMITED", Some(1000.0));
            }
        }
        let out = self.answer(cfg, &op, &tool, &b.args, &b.request_id);
        self.state.lock().expect("the service's state").inflight.leave(GRANT);
        match out {
            Ok(envelope) => Answer { ok: true, envelope },
            Err(code) => Self::no(&b.request_id, &code, None),
        }
    }

    // The arguments checked again, then the answer, its reads answered
    // here: the time, the made-up view and stats, and the check of the
    // answer against its schema.
    fn answer(&self, cfg: Config, op: &str, tool: &str, args: &Value, request_id: &Value) -> Result<Value, String> {
        let parsed = schema::arguments(tool, args).ok_or_else(|| "INVALID_ARGUMENT".to_string())?;
        let done = ready(service::answer(&MadeUp, &cfg, &self.pages, op, tool, &parsed, &self.policy, request_id));
        match done {
            Ok(out) => Ok(out.get("envelope").cloned().unwrap_or(Value::Null)),
            Err(f) if f.fields.get("refused") == Some(&json!(true)) => Err(f.code().unwrap_or("INTERNAL_ERROR").to_string()),
            Err(_) => Err("INTERNAL_ERROR".into()),
        }
    }
}

impl xenonet_mcp::fixture::Fixture for Fixture {
    fn read(&self, operation: &str, args: &Value, request_id: &str) -> Answer {
        self.read_one(operation, args, request_id)
    }
}

/// The made-up world the fixture's reads see.
struct MadeUp;

#[async_trait(?Send)]
impl ReadHost for MadeUp {
    async fn now(&self) -> f64 {
        now()
    }
    async fn view(&self) -> Result<Value, Failure> {
        Ok(fixtures::fixture_view(now()))
    }
    async fn actor(&self, _id: &Value) -> Result<Value, Failure> {
        Ok(Value::Null)
    }
    async fn stats(&self, id: &str) -> Result<Value, Failure> {
        Ok(fixtures::fixture_stats(id))
    }
    async fn id_key(&self) -> Result<Vec<u8>, Failure> {
        Ok(b"fixture".to_vec())
    }
    async fn check(&self, tool: &str, envelope: &Value) -> Result<bool, Failure> {
        Ok(schema::answers(tool, envelope))
    }
}

/// Runs an answer to its end: nothing the made-up world does waits.
fn ready<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => unreachable!("the made-up world never waits"),
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
            let r = f.read_one(op, &args, "r_00000000-0000-4000-8000-000000000000");
            assert!(r.ok, "{tool}: {}", r.envelope);
            assert!(r.envelope["warnings"].as_array().unwrap().iter().any(|w| w["code"] == json!("FIXTURE_DATA")), "{tool}");
        }
        let bad = f.read_one("sessions", &json!({ "path": "/etc" }), "r_00000000-0000-4000-8000-000000000001");
        assert_eq!((bad.ok, bad.envelope["code"].clone()), (false, json!("INVALID_ARGUMENT")));
    }
}
