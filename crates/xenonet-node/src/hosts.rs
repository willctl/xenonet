//! xenonet-hosts for the twins of core/hosts/.

// On the module, not the items: napi-derive copies an item's attributes
// onto the two-argument wrapper it generates for the export, and an
// expectation there goes unfulfilled.
#![expect(clippy::too_many_arguments, reason = "qualifyGraph takes the twin's arguments one by one, as the JS function did")]

use crate::app::{answer, fs_error, s};
use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_hosts::{aliases, cache, config, declared, doctor, frames, hub, link, merge, relay};

fn arr(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

fn refused(r: std::result::Result<Value, config::Refused>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
        Err(e) => Jv::Obj(vec![("error", Jv::Obj(vec![("message", s(e.message)), ("status", Jv::Num(f64::from(e.status)))]))]),
    }
}

// ----- frames -----

#[napi(js_name = "hostsFrameConstants")]
pub fn hosts_frame_constants() -> Json {
    Json(
        json!({ "protocol": frames::PROTOCOL, "maxFrame": frames::MAX_FRAME, "maxRequest": frames::MAX_REQUEST, "fromRelay": frames::FROM_RELAY, "fromConsole": frames::FROM_CONSOLE, "maxDepth": frames::MAX_DEPTH }),
    )
}

/// A stream of bytes split into lines, for the twin's lineSplitter.
#[napi(js_name = "HostsLines")]
pub struct HostsLines {
    inner: frames::LineSplitter,
}

#[napi]
impl HostsLines {
    #[napi(constructor)]
    pub fn new(max: f64) -> Self {
        HostsLines { inner: frames::LineSplitter::new(if max.is_finite() && max >= 0.0 { max as usize } else { frames::MAX_FRAME }) }
    }

    /// The lines and drops a chunk held, in order: a string for a line,
    /// null for a drop.
    #[napi]
    pub fn push(&mut self, chunk: Buffer) -> Jv {
        let split = self.inner.push(&chunk);
        let mut lines = split.lines.into_iter();
        Jv::Arr(split.order.into_iter().map(|line| if line { s(lines.next().unwrap_or_default()) } else { Jv::Json(Value::Null) }).collect())
    }
}

#[napi(js_name = "hostsParseFrame")]
pub fn hosts_parse_frame(line: String, known: Option<Vec<String>>) -> Jv {
    let k: Option<Vec<&str>> = known.as_ref().map(|k| k.iter().map(String::as_str).collect());
    match frames::parse_frame(&line, k.as_deref()) {
        Ok(f) => Jv::Obj(vec![("frame", Jv::Json(f))]),
        Err(e) => Jv::Obj(vec![("error", s(e))]),
    }
}

#[napi(js_name = "hostsEncode")]
pub fn hosts_encode(frame: Json) -> String {
    frames::encode(&frame.0)
}

/// shallow() of a JS value, walked where it is: converting one nested
/// deep enough would overflow the stack, which is what this guards.
#[napi(js_name = "hostsShallow")]
pub fn hosts_shallow(value: Unknown<'_>, max: f64) -> Result<bool> {
    let max = if max.is_finite() && max >= 0.0 { max as usize } else { 0 };
    let mut stack = vec![(value, 0usize)];
    while let Some((v, depth)) = stack.pop() {
        if depth > max {
            return Ok(false);
        }
        if v.get_type()? != ValueType::Object {
            continue;
        }
        let o: Object<'_> = unsafe { v.cast()? };
        let children: Vec<Unknown<'_>> = if o.is_array()? {
            (0..o.get_array_length()?).map(|i| o.get_element::<Unknown<'_>>(i)).collect::<Result<_>>()?
        } else {
            Object::keys(&o)?.iter().map(|k| o.get_named_property::<Unknown<'_>>(k)).collect::<Result<_>>()?
        };
        for x in children {
            if x.get_type()? == ValueType::Object {
                stack.push((x, depth + 1));
            }
        }
    }
    Ok(true)
}

// ----- aliases -----

/// When to read ~/.ssh/config again, for the twin's aliasReader.
#[napi(js_name = "HostsAliases")]
pub struct HostsAliases {
    inner: aliases::AliasReader,
}

#[napi]
impl HostsAliases {
    #[napi(constructor)]
    pub fn new(home: Option<String>) -> Self {
        HostsAliases { inner: aliases::AliasReader::new(home.as_deref()) }
    }

    /// "same", "none", or the file to read again.
    #[napi]
    pub fn look(&mut self) -> String {
        match self.inner.look() {
            aliases::Look::Same => "same".into(),
            aliases::Look::None => "none".into(),
            aliases::Look::Read(f) => f,
        }
    }
}

// ----- cache -----

#[napi(js_name = "hostsReadCache")]
pub fn hosts_read_cache(dir: Option<String>, key: String) -> Json {
    Json(cache::read_cache(dir.as_deref(), &key).unwrap_or(Value::Null))
}

#[napi(js_name = "hostsWriteCache")]
pub fn hosts_write_cache(dir: Option<String>, key: String, snap: Json) -> bool {
    let v = snap.0;
    let get = |k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    cache::write_cache(dir.as_deref(), &key, &get("id"), &get("savedAt"), &get("hello"), v.get("graph"), v.get("live"), &get("engine"))
}

#[napi(js_name = "hostsRemoveCache")]
pub fn hosts_remove_cache(dir: Option<String>, key: String) -> Jv {
    match cache::remove_cache(dir.as_deref(), &key) {
        Ok(()) => Jv::Obj(vec![("ok", Jv::Undef)]),
        Err(e) => Jv::Obj(vec![("error", fs_error(&e))]),
    }
}

// ----- doctor -----

#[napi(js_name = "hostsKeyCheck")]
pub fn hosts_key_check(hosts: MaybeJson) -> Json {
    Json(doctor::key_check(&arr(hosts.0)))
}

#[napi(js_name = "hostsKeyed")]
pub fn hosts_keyed(hosts: MaybeJson) -> Json {
    Json(Value::Array(doctor::keyed(&arr(hosts.0))))
}

// ----- declared -----

/// One schema's shape table, for the twin's declared().
#[napi(js_name = "HostsShapes")]
pub struct HostsShapes {
    table: Vec<declared::Shape>,
}

#[napi]
impl HostsShapes {
    #[napi(constructor)]
    pub fn new(table: Json) -> Self {
        HostsShapes { table: arr(Some(table.0)).iter().map(declared::shape_of).collect() }
    }

    /// `parses(i, value)` says whether shape i's schema takes the value.
    #[napi]
    pub fn declared(&self, at: u32, value: MaybeJson, parses: Function<'_, FnArgs<(u32, Json)>, bool>) -> Result<Jv> {
        let Some(v) = value.0 else { return Ok(Jv::Undef) };
        let mut failed = None;
        let out = declared::declared(&self.table, at as usize, &v, &mut |i, x| match parses.call((i as u32, Json(x.clone())).into()) {
            Ok(b) => b,
            Err(e) => {
                failed.get_or_insert(e);
                false
            }
        });
        failed.map_or(Ok(Jv::Json(out)), Err)
    }
}

// ----- config -----

fn checker<'a>(check: &'a Function<'a, Json, Option<String>>, failed: &'a mut Option<Error>) -> impl FnMut(&Value) -> std::result::Result<(), String> + 'a {
    move |v: &Value| match check.call(Json(v.clone())) {
        Ok(None) => Ok(()),
        Ok(Some(why)) => Err(why),
        Err(e) => {
            failed.get_or_insert(e);
            Err("the schema couldn't be checked".into())
        }
    }
}

#[napi(js_name = "hostsConfigConstants")]
pub fn hosts_config_constants() -> Json {
    Json(json!({ "hostsFile": config::HOSTS_FILE, "sshConfigFile": config::SSH_CONFIG_FILE, "defaultPort": config::DEFAULT_PORT, "defaultSince": config::DEFAULT_SINCE }))
}

#[napi(js_name = "hostsParseTarget")]
pub fn hosts_parse_target(target: MaybeJson) -> Jv {
    refused(config::parse_target(target.0.as_ref()).map(|(user, hostname)| json!({ "user": user, "hostname": hostname })))
}

/// `check(entry)`: what the schema says is wrong with it, or null.
#[napi(js_name = "hostsEntry")]
pub fn hosts_entry(input: MaybeJson, now: f64, check: Function<'_, Json, Option<String>>) -> Result<Jv> {
    let mut failed = None;
    let r = {
        let mut c = checker(&check, &mut failed);
        config::host_entry(&input.0.unwrap_or_else(|| json!({})), now, &mut c)
    };
    failed.map_or(Ok(refused(r)), Err)
}

#[napi(js_name = "hostsSshConfigText")]
pub fn hosts_ssh_config_text(hosts: Json, check: Function<'_, Json, Option<String>>) -> Result<String> {
    let mut failed = None;
    let text = {
        let mut c = checker(&check, &mut failed);
        config::ssh_config_text(&arr(Some(hosts.0)), &mut |h| c(h).is_ok())
    };
    failed.map_or(Ok(text), Err)
}

#[napi(js_name = "hostsVersionOf")]
pub fn hosts_version_of(doc: Json) -> Option<f64> {
    config::version_of(&doc.0)
}

/// hosts.json as XenoNet keeps it: { self, hosts, dropped }.
#[napi(js_name = "hostsPrepare")]
pub fn hosts_prepare(doc: Json, now: f64, check: Function<'_, Json, Option<String>>) -> Result<Json> {
    let mut failed = None;
    let (self_v, hosts, dropped) = {
        let mut c = checker(&check, &mut failed);
        config::prepare(&doc.0, now, &mut |h| c(h).is_ok())
    };
    failed.map_or(Ok(Json(json!({ "self": self_v, "hosts": hosts, "dropped": dropped }))), Err)
}

#[napi(js_name = "hostsCheckSelfId")]
pub fn hosts_check_self_id(id: MaybeJson) -> Jv {
    refused(config::check_self_id(id.0.as_ref()).map(|()| Value::Null))
}

#[napi(js_name = "hostsAdd")]
pub fn hosts_add(hosts: Json, entry: Json, replace: bool) -> Jv {
    refused(config::add(&arr(Some(hosts.0)), &entry.0, replace).map(Value::Array))
}

#[napi(js_name = "hostsLearn")]
pub fn hosts_learn(entry: Json, key: MaybeJson, name: MaybeJson) -> Json {
    Json(config::learn(&entry.0, key.0.as_ref(), name.0.as_ref()))
}

// ----- link -----

#[napi(js_name = "hostsLinkConstants")]
pub fn hosts_link_constants() -> Json {
    Json(
        json!({ "helloMs": link::HELLO_MS, "staleMs": link::STALE_MS, "backoff": { "min": link::BACKOFF_MIN, "max": link::BACKOFF_MAX }, "getMs": link::GET_MS, "sshOptions": link::SSH_OPTIONS }),
    )
}

fn options_of(o: &Value) -> link::Options {
    let ssh_bin = match o.get("sshBin") {
        Some(Value::Array(a)) => a.iter().map(xenonet_base::text::js_string).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => vec!["ssh".into()],
    };
    link::Options {
        ssh_config: o.get("sshConfig").and_then(Value::as_str).filter(|c| !c.is_empty()).map(String::from),
        ssh_bin,
        exec: o.get("exec").map(xenonet_base::text::js_string).unwrap_or_default(),
        run_as_node: xenonet_app::jsv::truthy(o.get("runAsNode")),
        root: o.get("root").map(xenonet_base::text::js_string).unwrap_or_default(),
        env: o.get("env").and_then(Value::as_object).cloned().unwrap_or_default(),
    }
}

#[napi(js_name = "hostsTransport")]
pub fn hosts_transport(entry: MaybeJson, options: Json) -> Jv {
    answer(link::transport(&entry.0.unwrap_or(Value::Null), &options_of(&options.0)).map(|(file, args, env)| Jv::Json(json!({ "file": file, "args": args, "env": env }))))
}

#[napi(js_name = "hostsShellPath")]
pub fn hosts_shell_path(p: String) -> String {
    link::shell_path(&p)
}

#[napi(js_name = "hostsTrustCommand")]
pub fn hosts_trust_command(id: String, ssh_config: Option<String>, tilde: Option<Function<'_, String, String>>) -> String {
    let t = |p: &str| tilde.as_ref().and_then(|f| f.call(p.to_string()).ok()).unwrap_or_else(|| p.to_string());
    link::trust_command(&id, ssh_config.as_deref(), &t)
}

fn why_jv(w: link::Why) -> Json {
    let mut m = Map::new();
    m.insert("state".into(), json!(w.state));
    m.insert("code".into(), json!(w.code));
    m.insert("message".into(), json!(w.message));
    if let Some(f) = w.fix {
        m.insert("fix".into(), json!(f));
    }
    Json(Value::Object(m))
}

/// whyEnded({ entry, exitCode, signal, stderr, bye, spawnError, sshConfig }).
#[napi(js_name = "hostsWhyEnded")]
pub fn hosts_why_ended(args: Json, tilde: Option<Function<'_, String, String>>) -> Json {
    let a = args.0;
    let t = |p: &str| tilde.as_ref().and_then(|f| f.call(p.to_string()).ok()).unwrap_or_else(|| p.to_string());
    let stderr = a.get("stderr").map(xenonet_base::text::js_string).unwrap_or_default();
    let spawn_error = a.get("spawnError").filter(|v| !v.is_null()).map(xenonet_base::text::js_string);
    let e = link::Ended {
        exit_code: a.get("exitCode").and_then(Value::as_f64),
        signal: a.get("signal").and_then(Value::as_str).map(String::from),
        stderr: &stderr,
        bye: a.get("bye").filter(|b| !b.is_null()),
        spawn_error: spawn_error.as_deref(),
    };
    why_jv(link::why_ended(a.get("entry").unwrap_or(&Value::Null), &e, a.get("sshConfig").and_then(Value::as_str), &t))
}

struct LinkAsks<'a> {
    random: &'a Function<'a, (), f64>,
    check: &'a Function<'a, Json, Option<String>>,
    tilde: Option<&'a Function<'a, String, String>>,
    failed: Option<Error>,
}

impl link::Asks for LinkAsks<'_> {
    fn random(&mut self) -> f64 {
        match self.random.call(()) {
            Ok(r) => r,
            Err(e) => {
                self.failed.get_or_insert(e);
                0.5
            }
        }
    }
    fn check_hello(&mut self, frame: &Value) -> Option<String> {
        match self.check.call(Json(frame.clone())) {
            Ok(r) => r,
            Err(e) => {
                self.failed.get_or_insert(e);
                Some("the schema couldn't be checked".into())
            }
        }
    }
    fn tilde(&self, p: &str) -> String {
        self.tilde.and_then(|f| f.call(p.to_string()).ok()).unwrap_or_else(|| p.to_string())
    }
}

fn effects(fx: Vec<link::Effect>) -> Jv {
    Jv::Arr(
        fx.into_iter()
            .map(|e| match e {
                link::Effect::Spawn { run, file, args, env } => {
                    Jv::Obj(vec![("do", s("spawn")), ("run", Jv::Num(run as f64)), ("file", s(file)), ("args", Jv::Json(json!(args))), ("env", Jv::Json(Value::Object(env)))])
                }
                link::Effect::Write { run, text } => Jv::Obj(vec![("do", s("write")), ("run", Jv::Num(run as f64)), ("text", s(text))]),
                link::Effect::Fail { run } => Jv::Obj(vec![("do", s("fail")), ("run", Jv::Num(run as f64))]),
                link::Effect::Kill { run } => Jv::Obj(vec![("do", s("kill")), ("run", Jv::Num(run as f64))]),
                link::Effect::Emit { name, data } => Jv::Obj(vec![("do", s("emit")), ("name", s(name)), ("data", Jv::Json(data))]),
                link::Effect::Timer { name, ms } => Jv::Obj(vec![("do", s("timer")), ("name", s(name)), ("ms", Jv::Num(ms))]),
                link::Effect::Interval { name, ms } => Jv::Obj(vec![("do", s("interval")), ("name", s(name)), ("ms", Jv::Num(ms))]),
                link::Effect::Clear { name } => Jv::Obj(vec![("do", s("clear")), ("name", s(name))]),
                link::Effect::Resolve { id, value } => Jv::Obj(vec![("do", s("resolve")), ("id", Jv::Num(id as f64)), ("value", Jv::Json(value))]),
                link::Effect::Reject { id, message, status } => {
                    Jv::Obj(vec![("do", s("reject")), ("id", Jv::Num(id as f64)), ("message", s(message)), ("status", Jv::Num(f64::from(status)))])
                }
            })
            .collect(),
    )
}

/// One host's link, for the twin's HostLink.
#[napi(js_name = "HostsLink")]
pub struct HostsLink {
    inner: link::Link,
}

macro_rules! with_asks {
    ($random:expr, $check:expr, $tilde:expr, |$a:ident| $body:expr) => {{
        let mut $a = LinkAsks { random: &$random, check: &$check, tilde: $tilde.as_ref(), failed: None };
        let fx = $body;
        match $a.failed {
            Some(e) => Err(e),
            None => Ok(effects(fx)),
        }
    }};
}

#[napi]
impl HostsLink {
    #[napi(constructor)]
    pub fn new(entry: Json, options: Json, settings: Json, now: f64) -> Self {
        let st = settings.0;
        let n = |k: &str, d: f64| st.get(k).and_then(Value::as_f64).unwrap_or(d);
        let backoff = (
            st.get("backoff").and_then(|b| b.get("min")).and_then(Value::as_f64).unwrap_or(link::BACKOFF_MIN),
            st.get("backoff").and_then(|b| b.get("max")).and_then(Value::as_f64).unwrap_or(link::BACKOFF_MAX),
        );
        let mut e = entry.0;
        if !e.is_object() {
            e = json!({});
        }
        HostsLink {
            inner: link::Link::new(
                e,
                options_of(&options.0),
                now,
                xenonet_app::jsv::truthy(st.get("demoOnly")),
                xenonet_app::jsv::truthy(st.get("once")),
                n("helloMs", link::HELLO_MS),
                n("staleMs", link::STALE_MS),
                backoff,
                n("getMs", link::GET_MS),
            ),
        }
    }

    #[napi(getter)]
    pub fn entry(&self) -> Json {
        Json(self.inner.entry.clone())
    }

    #[napi(getter)]
    pub fn state(&self) -> &'static str {
        self.inner.state
    }

    #[napi(getter)]
    pub fn hello(&self) -> Json {
        Json(self.inner.hello.clone())
    }

    #[napi(getter, js_name = "lastHeard")]
    pub fn last_heard(&self) -> Option<f64> {
        self.inner.last_heard
    }

    #[napi(setter, js_name = "lastHeard")]
    pub fn set_last_heard(&mut self, at: Option<f64>) {
        self.inner.last_heard = at;
    }

    #[napi(getter)]
    pub fn stopped(&self) -> bool {
        self.inner.stopped
    }

    #[napi(getter, js_name = "getMs")]
    pub fn get_ms(&self) -> f64 {
        self.inner.get_ms
    }

    #[napi]
    pub fn info(&self) -> Json {
        Json(self.inner.info())
    }

    #[napi]
    pub fn start(&mut self, now: f64, random: Function<'_, (), f64>, check: Function<'_, Json, Option<String>>, tilde: Option<Function<'_, String, String>>) -> Result<Jv> {
        with_asks!(random, check, tilde, |a| self.inner.start(now, &mut a))
    }

    #[napi]
    pub fn stop(&mut self, now: f64) -> Jv {
        effects(self.inner.stop(now))
    }

    #[napi]
    pub fn pause(&mut self, message: String, now: f64) -> Jv {
        effects(self.inner.pause(&message, now))
    }

    #[napi]
    pub fn reconnect(&mut self, now: f64, random: Function<'_, (), f64>, check: Function<'_, Json, Option<String>>, tilde: Option<Function<'_, String, String>>) -> Result<Jv> {
        with_asks!(random, check, tilde, |a| self.inner.reconnect(now, &mut a))
    }

    #[napi]
    pub fn retry(&mut self, now: f64, random: Function<'_, (), f64>, check: Function<'_, Json, Option<String>>, tilde: Option<Function<'_, String, String>>) -> Result<Jv> {
        with_asks!(random, check, tilde, |a| self.inner.retry(now, &mut a))
    }

    /// { id, effects }, or { error: { message, status } }.
    #[napi]
    pub fn get(&mut self, route: MaybeJson, timeout_ms: f64) -> Jv {
        match self.inner.get(&route.0.unwrap_or(Value::Null), timeout_ms) {
            Ok((id, fx)) => Jv::Obj(vec![("id", Jv::Num(id as f64)), ("effects", effects(fx))]),
            Err((message, status)) => Jv::Obj(vec![("error", Jv::Obj(vec![("message", s(message)), ("status", Jv::Num(f64::from(status)))]))]),
        }
    }

    #[napi(js_name = "getTimeout")]
    pub fn get_timeout(&mut self, id: f64, timeout_ms: f64) -> Jv {
        effects(self.inner.get_timeout(id as u64, timeout_ms))
    }

    #[napi(js_name = "helloTimeout")]
    pub fn hello_timeout(
        &mut self,
        run: f64,
        now: f64,
        random: Function<'_, (), f64>,
        check: Function<'_, Json, Option<String>>,
        tilde: Option<Function<'_, String, String>>,
    ) -> Result<Jv> {
        with_asks!(random, check, tilde, |a| self.inner.hello_timeout(run as u64, now, &mut a))
    }

    #[napi(js_name = "timeTick")]
    pub fn time_tick(&mut self, run: f64, now: f64) -> Jv {
        effects(self.inner.time_tick(run as u64, now))
    }

    #[napi(js_name = "checkStale")]
    pub fn check_stale(&mut self, now: f64) -> Jv {
        effects(self.inner.check_stale(now))
    }

    #[napi]
    pub fn stderr(&mut self, run: f64, chunk: String) {
        self.inner.stderr(run as u64, &chunk);
    }

    #[napi(js_name = "tooBig")]
    pub fn too_big(&mut self, run: f64, now: f64) {
        self.inner.too_big(run as u64, now);
    }

    #[napi]
    pub fn line(
        &mut self,
        run: f64,
        line: String,
        now: f64,
        random: Function<'_, (), f64>,
        check: Function<'_, Json, Option<String>>,
        tilde: Option<Function<'_, String, String>>,
    ) -> Result<Jv> {
        with_asks!(random, check, tilde, |a| self.inner.line(run as u64, &line, now, &mut a))
    }

    /// The run's program ended: { exitCode, signal, spawnError }.
    #[napi]
    pub fn end(
        &mut self,
        run: f64,
        how: Json,
        now: f64,
        random: Function<'_, (), f64>,
        check: Function<'_, Json, Option<String>>,
        tilde: Option<Function<'_, String, String>>,
    ) -> Result<Jv> {
        let h = how.0;
        let spawn_error = h.get("spawnError").filter(|v| !v.is_null()).map(xenonet_base::text::js_string);
        let e = link::Ended {
            exit_code: h.get("exitCode").and_then(Value::as_f64),
            signal: h.get("signal").and_then(Value::as_str).map(String::from),
            stderr: "",
            bye: None,
            spawn_error: spawn_error.as_deref(),
        };
        with_asks!(random, check, tilde, |a| self.inner.end(run as u64, &e, now, &mut a))
    }
}

// ----- merge -----

struct JsSchemas<'a> {
    clean: &'a Function<'a, FnArgs<(String, Json)>, Json>,
    activity: &'a Function<'a, FnArgs<(Json, f64, Json)>, Json>,
    failed: Option<Error>,
}

impl merge::Schemas for JsSchemas<'_> {
    fn clean(&mut self, kind: merge::Kind<'_>, value: &Value) -> Option<Value> {
        let name = match kind {
            merge::Kind::Node(k) => format!("node:{k}"),
            merge::Kind::Identity => "identity".into(),
            merge::Kind::LiveSession => "live-session".into(),
            merge::Kind::LiveActivity => "live-activity".into(),
            merge::Kind::EngineAgent => "engine-agent".into(),
            merge::Kind::EngineMessage => "engine-message".into(),
        };
        match self.clean.call((name, Json(value.clone())).into()) {
            Ok(Json(Value::Null)) => None,
            Ok(Json(v)) => Some(v),
            Err(e) => {
                self.failed.get_or_insert(e);
                None
            }
        }
    }
    fn activity(&mut self, updated_at: &Value, now: f64, windows: &Value) -> Value {
        match self.activity.call((Json(updated_at.clone()), now, Json(windows.clone())).into()) {
            Ok(Json(v)) => v,
            Err(e) => {
                self.failed.get_or_insert(e);
                Value::Null
            }
        }
    }
}

macro_rules! with_schemas {
    ($clean:expr, $activity:expr, |$sc:ident| $body:expr) => {{
        let mut $sc = JsSchemas { clean: &$clean, activity: &$activity, failed: None };
        let v = $body;
        match $sc.failed {
            Some(e) => Err(e),
            None => Ok(v),
        }
    }};
}

#[napi(js_name = "hostsMergeConstants")]
pub fn hosts_merge_constants() -> Json {
    Json(json!({ "maxNodes": merge::MAX_NODES }))
}

#[napi(js_name = "hostsRemoteId")]
pub fn hosts_remote_id(host: String, id: MaybeJson) -> Option<String> {
    merge::remote_id(&host, id.0.as_ref())
}

#[napi(js_name = "hostsIdentityId")]
pub fn hosts_identity_id(host: String, id: MaybeJson) -> Option<String> {
    merge::identity_id(&host, id.0.as_ref())
}

/// cleanNode's memo for one host, for the twin's cleanOnce.
#[napi(js_name = "HostsCleanOnce")]
pub struct HostsCleanOnce {
    inner: merge::CleanOnce,
}

#[napi]
impl HostsCleanOnce {
    #[napi(constructor)]
    pub fn new() -> Self {
        HostsCleanOnce { inner: Default::default() }
    }

    #[napi]
    pub fn clean(&mut self, node: Json, clean: Function<'_, FnArgs<(String, Json)>, Json>, activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>) -> Result<Json> {
        let inner = &mut self.inner;
        with_schemas!(clean, activity, |sc| Json(inner.clean(&mut sc, &node.0).unwrap_or(Value::Null)))
    }

    #[napi]
    pub fn done(&mut self) {
        self.inner.done();
    }
}

fn part_jv(p: merge::Part) -> Json {
    let host_node = p.nodes.first().cloned().unwrap_or(Value::Null);
    Json(
        json!({ "hostNode": host_node, "nodes": p.nodes, "links": p.links, "relations": p.relations, "identities": p.identities, "stats": p.stats, "truncated": p.truncated, "dropped": p.dropped }),
    )
}

/// qualifyGraph; `once` is the host's cleanOnce memo, or null.
#[napi(js_name = "hostsQualifyGraph")]
pub fn hosts_qualify_graph(
    graph: MaybeJson,
    host: Json,
    layout: Json,
    now: f64,
    windows: Json,
    once: Option<&mut HostsCleanOnce>,
    clean: Function<'_, FnArgs<(String, Json)>, Json>,
    activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>,
) -> Result<Json> {
    let g = graph.0.unwrap_or(Value::Null);
    let mut cleaner = match once {
        Some(o) => merge::Cleaner::Once(&mut o.inner),
        None => merge::Cleaner::Each,
    };
    with_schemas!(clean, activity, |sc| part_jv(merge::qualify_graph(&g, &host.0, &layout.0, now, &windows.0, &mut cleaner, &mut sc)))
}

#[napi(js_name = "hostsSameRepo")]
pub fn hosts_same_repo(nodes: Json, cap: Option<f64>) -> Json {
    Json(Value::Array(merge::same_repo(&arr(Some(nodes.0)), cap.map_or(8, |c| c.max(0.0) as usize))))
}

#[napi(js_name = "hostsMergeIdentities")]
pub fn hosts_merge_identities(local: Json, remotes: Json, names: MaybeJson, hidden: MaybeJson) -> Json {
    let remotes: Vec<(String, Vec<Value>)> =
        arr(Some(remotes.0)).into_iter().map(|r| (r.get("host").map(xenonet_base::text::js_string).unwrap_or_default(), arr(r.get("identities").cloned()))).collect();
    Json(Value::Array(merge::merge_identities(&arr(Some(local.0)), &remotes, &names.0.unwrap_or_else(|| json!({})), &hidden.0.unwrap_or_else(|| json!({})))))
}

#[napi(js_name = "hostsHostSummary")]
pub fn hosts_host_summary(host: Json, part: MaybeJson) -> Json {
    let p = part.0.filter(|p| !p.is_null()).map(|p| merge::Part {
        nodes: arr(p.get("nodes").cloned()),
        links: Vec::new(),
        relations: Vec::new(),
        identities: Vec::new(),
        stats: p.get("stats").and_then(Value::as_object).cloned().unwrap_or_default(),
        truncated: xenonet_app::jsv::truthy(p.get("truncated")),
        dropped: 0,
    });
    Json(merge::host_summary(&host.0, p.as_ref()))
}

fn aliases_of(v: &Value) -> HashMap<String, String> {
    match v {
        Value::Array(pairs) => pairs.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string()))).collect(),
        Value::Object(m) => m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
        _ => HashMap::new(),
    }
}

/// mergeGraph; `hosts` is [{ info, snapshot }], `memos` each host's
/// cleanOnce in the same order (or null), and `opts` the rest.
#[napi(js_name = "hostsMergeGraph")]
pub fn hosts_merge_graph(
    local: Json,
    hosts: Json,
    mut memos: Vec<Option<&mut HostsCleanOnce>>,
    opts: Json,
    clean: Function<'_, FnArgs<(String, Json)>, Json>,
    activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>,
) -> Result<Json> {
    let o = opts.0;
    let get = |k: &str| o.get(k).cloned().unwrap_or_else(|| json!({}));
    let mut ins: Vec<merge::HostIn<'_>> = Vec::new();
    for (i, h) in arr(Some(hosts.0)).into_iter().enumerate() {
        let memo = memos.get_mut(i).and_then(Option::take).map(|m| &mut m.inner);
        ins.push(merge::HostIn { info: h.get("info").cloned().unwrap_or(Value::Null), snapshot: h.get("snapshot").cloned().unwrap_or(Value::Null), clean: memo });
    }
    let aliases = aliases_of(&o.get("aliases").cloned().unwrap_or(Value::Null));
    let windows = o.get("windows").cloned().unwrap_or_else(|| json!({ "activeWindowMs": 600000, "recentWindowMs": 86400000, "projectActiveMs": 3600000 }));
    with_schemas!(clean, activity, |sc| Json(merge::merge_graph(
        &local.0,
        &mut ins,
        &get("layout"),
        o.get("now").and_then(Value::as_f64).unwrap_or(0.0),
        &windows,
        &o.get("self").cloned().unwrap_or(Value::Null),
        &get("names"),
        &get("hidden"),
        &aliases,
        &mut sc
    )))
}

/// mergeLimits: { limits, replaced }, the apps another machine's reading
/// took the place of.
#[napi(js_name = "hostsMergeLimits")]
pub fn hosts_merge_limits(local: MaybeJson, readings: Json) -> Json {
    let (limits, replaced) = merge::merge_limits_with(&local.0.unwrap_or_else(|| json!({})), &arr(Some(readings.0)));
    Json(json!({ "limits": limits, "replaced": replaced }))
}

fn pairs(hosts: Value) -> Vec<(Value, Value)> {
    arr(Some(hosts)).into_iter().map(|h| (h.get("info").cloned().unwrap_or(Value::Null), h.get("snapshot").cloned().unwrap_or(Value::Null))).collect()
}

#[napi(js_name = "hostsMergeLive")]
pub fn hosts_merge_live(local: Json, hosts: Json, clean: Function<'_, FnArgs<(String, Json)>, Json>, activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>) -> Result<Json> {
    let hs = pairs(hosts.0);
    with_schemas!(clean, activity, |sc| {
        let (live, replaced) = merge::merge_live_with(&local.0, &hs, &mut sc);
        Json(json!({ "live": live, "replaced": replaced }))
    })
}

#[napi(js_name = "hostsMergeSummary")]
pub fn hosts_merge_summary(local: Json, hosts: Json, clean: Function<'_, FnArgs<(String, Json)>, Json>, activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>) -> Result<Json> {
    let hs = pairs(hosts.0);
    with_schemas!(clean, activity, |sc| Json(merge::merge_summary(&local.0, &hs, &mut sc)))
}

#[napi(js_name = "hostsQualifyLive")]
pub fn hosts_qualify_live(live: MaybeJson, host: Json, clean: Function<'_, FnArgs<(String, Json)>, Json>, activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>) -> Result<Json> {
    let l = live.0.unwrap_or(Value::Null);
    with_schemas!(clean, activity, |sc| {
        let q = merge::qualify_live(&l, &host.0, &mut sc);
        Json(
            json!({ "sessions": q.sessions, "collisions": q.collisions, "collab": q.collab, "series": q.series, "at": q.at, "tpm": q.tpm, "today": q.today, "total24h": q.total24h, "readings": q.readings }),
        )
    })
}

#[napi(js_name = "hostsQualifyEvent")]
pub fn hosts_qualify_event(
    name: String,
    data: MaybeJson,
    host: Json,
    clean: Function<'_, FnArgs<(String, Json)>, Json>,
    activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>,
) -> Result<Json> {
    let d = data.0.unwrap_or(Value::Null);
    with_schemas!(clean, activity, |sc| Json(merge::qualify_event(&name, &d, &host.0, &mut sc).unwrap_or(Value::Null)))
}

#[napi(js_name = "hostsQualifyEngine")]
pub fn hosts_qualify_engine(
    engine: MaybeJson,
    host: Json,
    clean: Function<'_, FnArgs<(String, Json)>, Json>,
    activity: Function<'_, FnArgs<(Json, f64, Json)>, Json>,
) -> Result<Json> {
    let e = engine.0.unwrap_or(Value::Null);
    with_schemas!(clean, activity, |sc| Json(merge::qualify_engine(&e, &host.0, &mut sc)))
}

// ----- hub -----

#[napi(js_name = "hostsHubConstants")]
pub fn hosts_hub_constants() -> Json {
    Json(json!({ "graphEveryMs": hub::GRAPH_EVERY_MS, "liveEveryMs": hub::LIVE_EVERY_MS, "cacheEveryMs": hub::CACHE_EVERY_MS, "testMs": hub::TEST_MS }))
}

#[napi(js_name = "hostsSoonWait")]
pub fn hosts_soon_wait(last: f64, every: f64, now: f64) -> f64 {
    hub::soon_wait(last, every, now)
}

#[napi(js_name = "hostsCacheWait")]
pub fn hosts_cache_wait(last_cache_at: Option<f64>, now: f64) -> f64 {
    hub::cache_wait(last_cache_at, now)
}

#[napi(js_name = "hostsInfoOf")]
pub fn hosts_info_of(info: Json, cached: bool, cached_hello: MaybeJson, skew_ms: f64) -> Json {
    Json(hub::info_of(&info.0, cached, &cached_hello.0.unwrap_or(Value::Null), skew_ms))
}

#[napi(js_name = "hostsView")]
pub fn hosts_view(info: Json, entry: Json, snapshot: MaybeJson, cached: bool, simulated: bool, doctor: MaybeJson) -> Json {
    Json(hub::view(&info.0, &entry.0, &snapshot.0.unwrap_or(Value::Null), cached, simulated, &doctor.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "hostsForPage")]
pub fn hosts_for_page(view: Json) -> Json {
    Json(hub::for_page(&view.0))
}

#[napi(js_name = "hostsDoctorOf")]
pub fn hosts_doctor_of(data: Json) -> Json {
    Json(hub::doctor_of(&data.0))
}

#[napi(js_name = "hostsSkewOf")]
pub fn hosts_skew_of(ms: MaybeJson, b: MaybeJson) -> f64 {
    hub::skew_of(&ms.0.unwrap_or(Value::Null), &b.0.unwrap_or(Value::Null))
}

#[napi(js_name = "hostsProbeSnapshot")]
pub fn hosts_probe_snapshot(graph: Json, live: MaybeJson) -> Json {
    Json(hub::probe_snapshot(&graph.0, &live.0.unwrap_or(Value::Null)))
}

// ----- relay -----

#[napi(js_name = "hostsRelayConstants")]
pub fn hosts_relay_constants() -> Json {
    Json(
        json!({ "pingMs": relay::PING_MS, "defaultSinceMs": relay::DEFAULT_SINCE_MS, "getTimeoutMs": relay::GET_TIMEOUT_MS, "maxGets": relay::MAX_GETS, "exit": { "done": relay::EXIT_DONE, "failed": relay::EXIT_FAILED, "noXenoNet": relay::EXIT_NO_XENONET, "xenonetGone": relay::EXIT_XENONET_GONE, "oldXenoNet": relay::EXIT_OLD_XENONET } }),
    )
}

#[napi(js_name = "hostsParseSince")]
pub fn hosts_parse_since(text: String) -> Option<f64> {
    relay::parse_since(&text)
}

#[napi(js_name = "hostsFromOriginalCommand")]
pub fn hosts_from_original_command(text: String) -> Option<f64> {
    relay::from_original_command(&text)
}

/// A copy of values with the home folder as ~, for the twin's
/// homeScrubber.
#[napi(js_name = "HostsScrubber")]
pub struct HostsScrubber {
    inner: relay::Scrubber,
}

#[napi]
impl HostsScrubber {
    #[napi(constructor)]
    pub fn new(home: Option<String>, also: Vec<String>) -> Self {
        HostsScrubber { inner: relay::Scrubber::new(home.as_deref(), &also) }
    }

    #[napi]
    pub fn walk(&self, value: MaybeJson) -> Jv {
        value.0.map_or(Jv::Undef, |v| Jv::Json(self.inner.walk(&v)))
    }
}

#[napi(js_name = "hostsUserFolderOf")]
pub fn hosts_user_folder_of(p: MaybeJson, home: Option<String>) -> Option<String> {
    relay::user_folder_of(p.0.as_ref(), home.as_deref())
}

#[napi(js_name = "hostsUserFolders")]
pub fn hosts_user_folders(graph: MaybeJson, home: Option<String>) -> Vec<String> {
    relay::user_folders(&graph.0.unwrap_or(Value::Null), home.as_deref())
}

#[napi(js_name = "hostsMetadataOnly")]
pub fn hosts_metadata_only(kind: String, value: Json) -> Json {
    Json(match kind.as_str() {
        "graph" => relay::metadata_graph(value.0),
        "live" => relay::metadata_live(value.0),
        "activity" => relay::metadata_activity(value.0),
        "engine" => relay::metadata_engine(value.0),
        _ => relay::metadata_message(value.0),
    })
}

/// How the shapers copy what goes: the relay's scrubber, a function of the
/// caller's, or a plain copy.
fn copier<'a>(scrub: Option<&'a HostsScrubber>, copy: Option<&'a Function<'a, Json, Json>>, failed: &'a mut Option<Error>) -> impl FnMut(&Value) -> Value + 'a {
    let plain = relay::Scrubber::new(None, &[]);
    move |v: &Value| match (scrub, copy) {
        (Some(sc), _) => sc.inner.walk(v),
        (None, Some(f)) => match f.call(Json(v.clone())) {
            Ok(Json(out)) => out,
            Err(e) => {
                failed.get_or_insert(e);
                Value::Null
            }
        },
        (None, None) => plain.walk(v),
    }
}

/// shapeGraph: { graph, dropped }.
#[napi(js_name = "hostsShapeGraph")]
pub fn hosts_shape_graph(
    input: Json,
    since_ms: Option<f64>,
    now: f64,
    aliases: MaybeJson,
    detail: bool,
    scrub: Option<&HostsScrubber>,
    copy: Option<Function<'_, Json, Json>>,
) -> Result<Json> {
    let mut failed = None;
    let (g, dropped) = {
        let mut c = copier(scrub, copy.as_ref(), &mut failed);
        relay::shape_graph(&input.0, since_ms, now, &aliases_of(&aliases.0.unwrap_or(Value::Null)), detail, &mut c)
    };
    let mut d: Vec<String> = dropped.into_iter().collect();
    d.sort();
    failed.map_or(Ok(Json(json!({ "graph": g, "dropped": d }))), Err)
}

#[napi(js_name = "hostsShapeLive")]
pub fn hosts_shape_live(input: Json, dropped: Vec<String>, detail: bool, scrub: Option<&HostsScrubber>, copy: Option<Function<'_, Json, Json>>) -> Result<Json> {
    let d: HashSet<String> = dropped.into_iter().collect();
    let mut failed = None;
    let out = {
        let mut c = copier(scrub, copy.as_ref(), &mut failed);
        relay::shape_live(&input.0, &d, detail, &mut c)
    };
    failed.map_or(Ok(Json(out)), Err)
}

#[napi(js_name = "hostsShapeEngine")]
pub fn hosts_shape_engine(input: MaybeJson, detail: bool, scrub: Option<&HostsScrubber>, copy: Option<Function<'_, Json, Json>>) -> Result<Json> {
    let mut failed = None;
    let out = {
        let mut c = copier(scrub, copy.as_ref(), &mut failed);
        relay::shape_engine(&input.0.unwrap_or(Value::Null), detail, &mut c)
    };
    failed.map_or(Ok(Json(out)), Err)
}

#[napi(js_name = "hostsShapeEvent")]
pub fn hosts_shape_event(name: String, data: MaybeJson, dropped: Vec<String>, detail: bool, scrub: Option<&HostsScrubber>, copy: Option<Function<'_, Json, Json>>) -> Result<Json> {
    let d: HashSet<String> = dropped.into_iter().collect();
    let mut failed = None;
    let out = {
        let mut c = copier(scrub, copy.as_ref(), &mut failed);
        relay::shape_event(&name, &data.0.unwrap_or(Value::Null), &d, detail, &mut c)
    };
    failed.map_or(Ok(Json(out.unwrap_or(Value::Null))), Err)
}

/// allowed: { shape, path, read } or { error }.
#[napi(js_name = "hostsAllowed")]
pub fn hosts_allowed(raw: MaybeJson) -> Json {
    Json(match relay::allowed(raw.0.as_ref()) {
        Ok((shape, path)) => {
            let name = match shape {
                relay::Shape::None => "none",
                relay::Shape::Graph => "graph",
                relay::Shape::Live => "live",
                relay::Shape::Engine => "engine",
            };
            json!({ "shape": name, "path": path, "read": relay::route_to_read(shape, &path) })
        }
        Err(e) => json!({ "error": e }),
    })
}

#[napi(js_name = "hostsSelfBye")]
pub fn hosts_self_bye(status: f64, body: MaybeJson, up_version: MaybeJson) -> Json {
    let (code, bye) = relay::self_bye(status, &body.0.unwrap_or(Value::Null), up_version.0.as_ref());
    Json(json!({ "exit": code, "bye": bye }))
}

/// The relay's way out, for the twin's runRelay.
#[napi(js_name = "HostsOutbox")]
pub struct HostsOutbox {
    inner: relay::Outbox,
}

#[napi]
impl HostsOutbox {
    #[napi(constructor)]
    pub fn new() -> Self {
        HostsOutbox { inner: Default::default() }
    }

    /// The text to write now, or null while the output is busy.
    #[napi]
    pub fn send(&mut self, text: String, coalesce: Option<String>, now: f64) -> Option<String> {
        self.inner.send(text, coalesce.as_deref(), now)
    }

    #[napi]
    pub fn full(&mut self) {
        self.inner.full();
    }

    #[napi]
    pub fn wrote(&mut self, now: f64) {
        self.inner.wrote(now);
    }

    #[napi(getter)]
    pub fn busy(&self) -> bool {
        self.inner.busy
    }

    #[napi]
    pub fn drain(&mut self) -> Vec<String> {
        self.inner.drain()
    }

    #[napi]
    pub fn left(&mut self, rest: Vec<String>) {
        self.inner.left(rest);
    }

    #[napi(js_name = "takeGap")]
    pub fn take_gap(&mut self) -> bool {
        self.inner.take_gap()
    }

    #[napi(js_name = "pingDue")]
    pub fn ping_due(&self, now: f64) -> bool {
        self.inner.ping_due(now)
    }
}
