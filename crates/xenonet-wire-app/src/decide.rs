//! The decision model (core/decide.js, crates/xenonet-app/src/decide.rs)
//! and what OpenRouter's key has spent (core/openrouter-usage.js,
//! crates/xenonet-sources/src/openrouter_usage.rs). One key serves both,
//! kept in your credential store, the demo's in memory; it goes only into
//! a request's header. The Rust builds each request and reads each answer;
//! the waiting is here.

use crate::fx::Locked;
use crate::fx::{now_ms, BoxFuture};
use crate::secrets::Secrets;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_app::decide;
use xenonet_service::cores::{CoreError, Decider, Kind, OpenRouter, R};
use xenonet_service::wire::Emitter;
use xenonet_sources::openrouter_usage as usage;

fn decision_error(e: decide::DecisionError) -> CoreError {
    CoreError::of(Kind::Decision, e.status, e.code, e.message)
}

pub struct Decisions {
    secrets: Arc<Secrets>,
    url: String,
    model: String,
    has_key: Mutex<Option<bool>>,
    emitter: Emitter,
}

impl Decisions {
    pub fn new(secrets: Arc<Secrets>, emitter: Emitter) -> Self {
        let (url, model) = decide::settings(std::env::var("XENONET_DECISIONS_URL").ok().as_deref(), std::env::var("XENONET_DECISION_MODEL").ok().as_deref());
        Decisions { secrets, url, model, has_key: Mutex::new(None), emitter }
    }

    fn say(&self, line: &decide::LogLine) {
        self.emitter.log(line.level, &line.code, &line.message);
    }

    async fn key(&self) -> Option<String> {
        let k = self.secrets.get(decide::KEY_TARGET).await.ok().flatten();
        *self.has_key.locked() = Some(k.is_some());
        k
    }

    /// `decider.decide({ kind, state, questions })`: the decision, or why
    /// there's none, as a DecisionError.
    pub async fn decide_on(&self, kind: &str, state: &Value, questions: &Value) -> R<Value> {
        let Some(k) = self.key().await else { return Err(decision_error(decide::no_key())) };
        let started = now_ms();
        let (headers, body) = decide::request(&self.model, &k, Some(state), Some(questions));
        let headers: Vec<(String, String)> = headers.into_iter().map(|(h, v)| (h.to_string(), v)).collect();
        let res = match xenonet_service::net::fetch("POST", &self.url, &headers, Some(body.into_bytes()), Duration::from_millis(decide::TIMEOUT_MS as u64)).await {
            Ok(r) => r,
            Err(m) => {
                let (line, e) = decide::unreachable(&self.url, kind, m.contains("aborted due to timeout"), &m);
                self.say(&line);
                return Err(decision_error(e));
            }
        };
        let body = xenonet_base::json::parse(&res.text()).ok();
        if !res.ok() {
            let (line, e) = decide::refused(res.status, body.as_ref(), kind);
            self.say(&line);
            return Err(decision_error(e));
        }
        let n = questions.as_object().map_or(0, |q| q.len());
        let (line, d) = decide::answered(body.as_ref(), &self.model, kind, n, now_ms() - started);
        self.say(&line);
        Ok(d)
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl Decider for Decisions {
    async fn status(&self) -> R<Value> {
        let known = *self.has_key.locked();
        let has = match known {
            Some(h) => h,
            None => {
                let h = self.secrets.has(decide::KEY_TARGET).await.unwrap_or(false);
                *self.has_key.locked() = Some(h);
                h
            }
        };
        Ok(json!({ "model": self.model, "key": if has { "stored" } else { "missing" }, "store": self.secrets.kind() }))
    }

    async fn store_key(&self, key: &Value) -> R<Value> {
        let given = match key {
            Value::Null => String::new(),
            v => xenonet_base::text::js_string(v),
        };
        let checked = decide::check_key(&given).map_err(decision_error)?;
        self.secrets.set(decide::KEY_TARGET, &checked, decide::KEY_COMMENT).await.map_err(CoreError::new)?;
        *self.has_key.locked() = Some(true);
        Ok(json!({ "key": "stored", "store": self.secrets.kind() }))
    }

    async fn forget_key(&self) -> R<Value> {
        let _ = self.secrets.remove(decide::KEY_TARGET).await;
        *self.has_key.locked() = Some(false);
        Ok(json!({ "key": "missing" }))
    }

    async fn decide(&self, request: Value) -> R<Value> {
        let kind = request.get("kind").map(xenonet_base::text::js_string).unwrap_or_default();
        self.decide_on(&kind, request.get("state").unwrap_or(&Value::Null), request.get("questions").unwrap_or(&Value::Null)).await
    }
}

/// What the key has spent, at most once a minute, one read at a time.
pub struct Spent {
    secrets: Arc<Secrets>,
    emitter: Emitter,
    last: Mutex<Option<(Value, f64)>>,
    pending: tokio::sync::Mutex<()>,
}

impl Spent {
    pub fn new(secrets: Arc<Secrets>, emitter: Emitter) -> Self {
        Spent { secrets, emitter, last: Mutex::new(None), pending: tokio::sync::Mutex::new(()) }
    }

    async fn read(&self) -> Value {
        let Some(key) = self.secrets.get(decide::KEY_TARGET).await.ok().flatten() else { return usage::reading(false, None, None, 0.0) };
        let headers = vec![("authorization".to_string(), format!("Bearer {key}")), ("accept".to_string(), "application/json".to_string())];
        let res = match xenonet_service::net::fetch("GET", usage::KEY_URL, &headers, None, Duration::from_secs(10)).await {
            Ok(r) => r,
            Err(m) => {
                self.emitter.log("warn", "openrouter.usage", &format!("Couldn't reach OpenRouter for the key's usage: {m}"));
                return usage::reading(true, None, None, 0.0);
            }
        };
        let refused = res.status == 401 || res.status == 403;
        let body = if refused || !res.ok() { None } else { xenonet_base::json::parse(&res.text()).ok() };
        let status = json!(res.status);
        usage::reading(true, Some((&status, res.ok())), body.as_ref(), if refused || !res.ok() { 0.0 } else { now_ms() })
    }

    fn fresh(&self) -> Option<Value> {
        self.last.locked().as_ref().filter(|(_, at)| usage::fresh(*at, now_ms())).map(|(v, _)| v.clone())
    }
}

#[async_trait]
impl OpenRouter for Spent {
    async fn status(&self) -> R<Value> {
        if let Some(v) = self.fresh() {
            return Ok(v);
        }
        // A read under way is the one every ask gets.
        let _one = self.pending.lock().await;
        if let Some(v) = self.fresh() {
            return Ok(v);
        }
        let r = self.read().await;
        *self.last.locked() = Some((r.clone(), now_ms()));
        Ok(r)
    }

    // A new key, or none: read again on the next ask.
    fn forget(&self) {
        *self.last.locked() = None;
    }
}

/// The demo's key, made up.
pub struct DemoSpent;

#[async_trait]
impl OpenRouter for DemoSpent {
    async fn status(&self) -> R<Value> {
        Ok(usage::demo(now_ms()))
    }
}

/// The decider as the integrations and the map's Codex pass ask it.
pub type Decide = Arc<dyn Fn(String, Value, Value) -> BoxFuture<R<Value>> + Send + Sync>;

pub fn decide_fn(d: Arc<Decisions>) -> Decide {
    Arc::new(move |kind, state, questions| {
        let d = d.clone();
        Box::pin(async move { d.decide_on(&kind, &state, &questions).await })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_key_kept_in_memory_and_forgotten() {
        let s = Arc::new(Secrets::memory());
        let d = Decisions::new(s.clone(), Emitter::new());
        assert_eq!(d.status().await.unwrap()["key"], "missing");
        let bad = d.store_key(&json!("not a key")).await.unwrap_err();
        assert!(bad.is(Kind::Decision));
        let made_up = format!("sk-or-{}", "x".repeat(24));
        assert_eq!(d.store_key(&json!(made_up)).await.unwrap()["key"], "stored");
        assert_eq!(d.status().await.unwrap()["key"], "stored");
        assert_eq!(d.forget_key().await.unwrap()["key"], "missing");
        assert!(d.decide_on("x", &json!({}), &json!({})).await.unwrap_err().is(Kind::Decision), "no key, no decision");
        assert!(Decider::decide(&d, json!({ "kind": "projects", "state": {}, "questions": {} })).await.unwrap_err().is(Kind::Decision));
        let spent = Spent::new(s, Emitter::new());
        assert_eq!(spent.status().await.unwrap(), json!({ "key": "missing" }));
        assert_eq!(DemoSpent.status().await.unwrap()["key"], "stored");
    }
}
