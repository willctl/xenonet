//! xenonet-sources' lines for the twin of core/lines.js: the views of a
//! Claude Code and a Codex line, the drift hooks' sorting, and the splitting
//! of what a read gave into lines.

use crate::js::{Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use xenonet_base::usage::Figure;
use xenonet_sources::lines::{self, ClaudeLine, CodexLine, TypeError, Usage};

// What JS would throw, thrown as JS throws it: a TypeError.
fn thrown<T>(env: Env, e: TypeError) -> Result<T> {
    let _ = env.throw_type_error(&e.0, None);
    Err(Error::new(Status::PendingException, e.0))
}

fn maybe(v: Option<&Value>) -> Jv {
    v.cloned().into()
}

// A figure as the line wrote it, or the number worked out.
fn figure(f: Figure) -> Jv {
    match f {
        Figure::Written(v) => v.into(),
        Figure::Worked(n) => Jv::Num(n),
    }
}

fn usage(u: Usage) -> Jv {
    let t = u.tokens;
    Jv::Obj(vec![
        ("input", figure(t.input)),
        ("cacheRead", figure(t.cache_read)),
        ("cacheWrite", Jv::Json(u.cache_write)),
        ("cacheWrite5m", figure(t.cache_write_5m)),
        ("cacheWrite1h", figure(t.cache_write_1h)),
        ("output", figure(t.output)),
        ("fast", Jv::Bool(t.speed.fast)),
    ])
}

#[napi(js_name = "ClaudeLineCore")]
pub struct JsClaudeLine {
    l: ClaudeLine,
}

#[napi]
impl JsClaudeLine {
    #[napi(constructor)]
    pub fn new(raw: crate::js::Json) -> Self {
        JsClaudeLine { l: ClaudeLine { raw: raw.0 } }
    }
    #[napi(getter)]
    pub fn kind(&self, env: Env) -> Result<Jv> {
        self.l.kind().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn ts(&self, env: Env) -> Result<f64> {
        self.l.ts().or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn meta(&self, env: Env) -> Result<Jv> {
        self.l.meta().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn sidechain(&self, env: Env) -> Result<Jv> {
        self.l.sidechain().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "compactSummary")]
    pub fn compact_summary(&self, env: Env) -> Result<Jv> {
        self.l.compact_summary().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn effort(&self, env: Env) -> Result<Jv> {
        self.l.effort().map(|v| Jv::Json(v.cloned().unwrap_or(Value::Null))).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn content(&self, env: Env) -> Result<Jv> {
        self.l.content().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "hasResults")]
    pub fn has_results(&self, env: Env) -> Result<bool> {
        self.l.has_results().or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn results(&self, env: Env) -> Result<Jv> {
        match self.l.results() {
            Ok(None) => Ok(Jv::Json(Value::Null)),
            Ok(Some(rs)) => Ok(Jv::Arr(rs.into_iter().map(|r| Jv::Obj(vec![("id", r.id.into()), ("error", Jv::Bool(r.error))])).collect())),
            Err(e) => thrown(env, e),
        }
    }
    #[napi(getter)]
    pub fn text(&self, env: Env) -> Result<String> {
        self.l.text().or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn interruption(&self, env: Env) -> Result<bool> {
        self.l.interruption().or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn message(&self, env: Env) -> Result<Jv> {
        self.l.message().map(|m| Jv::Json(m.clone())).or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "messageId")]
    pub fn message_id(&self, env: Env) -> Result<Jv> {
        self.l.message_id().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn model(&self, env: Env) -> Result<Jv> {
        self.l.model().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn usage(&self, env: Env) -> Result<Jv> {
        match self.l.usage() {
            Ok(u) => Ok(u.map_or(Jv::Json(Value::Null), usage)),
            Err(e) => thrown(env, e),
        }
    }
    #[napi(getter)]
    pub fn blocks(&self, env: Env) -> Result<Jv> {
        self.l.blocks().map(|b| Jv::Json(b.clone())).or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "endTurn")]
    pub fn end_turn(&self, env: Env) -> Result<bool> {
        self.l.end_turn().or_else(|e| thrown(env, e))
    }
}

#[napi(js_name = "CodexLineCore")]
pub struct JsCodexLine {
    l: CodexLine,
}

#[napi]
impl JsCodexLine {
    #[napi(constructor)]
    pub fn new(raw: crate::js::Json) -> Self {
        JsCodexLine { l: CodexLine { raw: raw.0 } }
    }
    #[napi(getter)]
    pub fn kind(&self, env: Env) -> Result<Jv> {
        self.l.kind().map(maybe).or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn ts(&self, env: Env) -> Result<f64> {
        self.l.ts().or_else(|e| thrown(env, e))
    }
    #[napi(getter)]
    pub fn payload(&self, env: Env) -> Result<Jv> {
        self.l.payload().map(|p| Jv::Json(p.clone())).or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "userMessage")]
    pub fn user_message(&self, env: Env) -> Result<Option<String>> {
        self.l.user_message().or_else(|e| thrown(env, e))
    }
    #[napi(getter, js_name = "userEvent")]
    pub fn user_event(&self, env: Env) -> Result<Option<String>> {
        self.l.user_event().or_else(|e| thrown(env, e))
    }
}

#[napi(js_name = "isCodexPrompt")]
pub fn is_codex_prompt(text: MaybeJson) -> bool {
    lines::is_codex_prompt(text.0.as_ref())
}

#[napi(js_name = "claudeLineKindOf")]
pub fn claude_line_kind_of(kind: MaybeJson) -> String {
    lines::claude_kind_of(kind.0.as_ref())
}

#[napi(js_name = "claudeLineVersionOf")]
pub fn claude_line_version_of(version: MaybeJson, current: Option<String>) -> Option<String> {
    lines::claude_version_of(version.0.as_ref(), current)
}

#[napi(js_name = "codexLineKindOf")]
pub fn codex_line_kind_of(kind: MaybeJson, payload_type: MaybeJson, item_type: MaybeJson) -> String {
    lines::codex_kind_of(kind.0.as_ref(), payload_type.0.as_ref(), item_type.0.as_ref())
}

#[napi(js_name = "codexLineVersionOf")]
pub fn codex_line_version_of(kind: MaybeJson, cli_version: MaybeJson, current: Option<String>) -> Option<String> {
    lines::codex_version_of(kind.0.as_ref(), cli_version.0.as_ref(), current)
}

/// What a read gave, split: { lines, hasLast, last, rest }.
#[napi(js_name = "takeLines")]
pub fn take_lines(data: Buffer) -> Jv {
    let t = lines::take(&data);
    Jv::Obj(vec![
        ("lines", Jv::Arr(t.lines.into_iter().map(|l| Jv::Json(Value::String(l))).collect())),
        ("hasLast", Jv::Bool(t.last.is_some())),
        ("last", t.last.into()),
        ("rest", Jv::Bytes(t.rest)),
    ])
}

/// Where a read of a transcript starts: [restarted, offset].
#[napi(js_name = "readFrom")]
pub fn read_from(size: f64, offset: f64) -> (bool, f64) {
    let (restarted, from) = lines::read_from(size as u64, offset as u64);
    (restarted, from as f64)
}
