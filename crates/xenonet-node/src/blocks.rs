//! xenonet-blocks for the twins of core/blocks/: a block's lifecycle, the
//! list of blocks, and the normalizer. Each Rust-backed block class carries
//! the lifecycle API (`lifecycle_api!`), so the twin of Block can stand on
//! any of them.

use crate::js::Json;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use xenonet_base::json as js_json;
use xenonet_blocks::lifecycle::{Counters, Lifecycle, Registry, State};
use xenonet_blocks::normalizer::{Hooks, Normalizer};

pub trait HasLife {
    fn life(&self) -> &Lifecycle;
    fn life_mut(&mut self) -> &mut Lifecycle;
}

#[napi(object)]
pub struct Change {
    pub from: String,
    pub to: String,
    pub at: f64,
}

#[napi(object)]
pub struct LastError {
    pub at: f64,
    pub message: String,
}

pub fn health_of(l: &Lifecycle) -> Value {
    let counters: Map<String, Value> = l.counters.iter().map(|(k, v)| (k.to_string(), js_json::number(v))).collect();
    json!({
        "state": l.state.as_str(),
        "since": js_json::number(l.since),
        "lastOk": l.last_ok.map(js_json::number),
        "lastError": l.last_error.as_ref().map(|e| json!({ "at": js_json::number(e.at), "message": e.message })),
        "counters": counters,
        "budget": null,
    })
}

fn state_of(s: &str) -> Result<State> {
    State::parse(s).ok_or_else(|| Error::new(Status::InvalidArg, format!("no such state: {s}")))
}

/// The lifecycle API on a napi class that has a lifecycle.
macro_rules! lifecycle_api {
    ($t:ty) => {
        #[napi]
        impl $t {
            #[napi(getter)]
            pub fn state(&self) -> &'static str {
                self.life().state.as_str()
            }
            #[napi(getter)]
            pub fn since(&self) -> f64 {
                self.life().since
            }
            #[napi(getter, js_name = "lastOk")]
            pub fn last_ok(&self) -> Option<f64> {
                self.life().last_ok
            }
            #[napi(setter, js_name = "lastOk")]
            pub fn set_last_ok(&mut self, at: Option<f64>) {
                self.life_mut().last_ok = at;
            }
            #[napi(getter, js_name = "lastError")]
            pub fn last_error(&self) -> Option<crate::blocks::LastError> {
                self.life().last_error.as_ref().map(|e| crate::blocks::LastError { at: e.at, message: e.message.clone() })
            }
            #[napi(js_name = "setLastError")]
            pub fn set_last_error(&mut self, e: Option<crate::blocks::LastError>) {
                self.life_mut().last_error = e.map(|e| xenonet_blocks::lifecycle::ErrorAt { at: e.at, message: e.message });
            }
            #[napi(getter)]
            pub fn started(&self) -> bool {
                self.life().started
            }
            #[napi(setter)]
            pub fn set_started(&mut self, v: bool) {
                self.life_mut().started = v;
            }
            #[napi(js_name = "setState")]
            pub fn set_state(&mut self, state: String, at: f64) -> napi::Result<()> {
                let s = crate::blocks::state_of_str(&state)?;
                self.life_mut().set_state(s, at);
                Ok(())
            }
            #[napi(js_name = "beginStart")]
            pub fn begin_start(&mut self, at: f64) -> Option<f64> {
                self.life_mut().begin_start(at).map(|r| r as f64)
            }
            #[napi(js_name = "startThrew")]
            pub fn start_threw(&mut self, run: f64, message: String, at: f64) {
                self.life_mut().start_threw(run as u64, &message, at);
            }
            #[napi(js_name = "startDone")]
            pub fn start_done(&mut self, run: f64, at: f64) {
                self.life_mut().start_done(run as u64, at);
            }
            #[napi]
            pub fn current(&self, run: f64) -> bool {
                self.life().current(run as u64)
            }
            #[napi(js_name = "beginStop")]
            pub fn begin_stop(&mut self) {
                self.life_mut().begin_stop();
            }
            #[napi]
            pub fn ok(&mut self, at: f64) {
                self.life_mut().ok(at);
            }
            #[napi]
            pub fn problem(&mut self, message: String, at: f64) {
                self.life_mut().problem(&message, at);
            }
            #[napi]
            pub fn fail(&mut self, message: String, at: f64) {
                self.life_mut().fail(&message, at);
            }
            #[napi]
            pub fn count(&mut self, name: String, n: f64) {
                self.life_mut().count(&name, n);
            }
            #[napi]
            pub fn counter(&self, name: String) -> Option<f64> {
                self.life().counters.get(&name)
            }
            #[napi(js_name = "setCounter")]
            pub fn set_counter(&mut self, name: String, v: f64) {
                self.life_mut().counters.set(&name, v);
            }
            #[napi(js_name = "deleteCounter")]
            pub fn delete_counter(&mut self, name: String) -> bool {
                self.life_mut().counters.remove(&name)
            }
            #[napi(js_name = "counterNames")]
            pub fn counter_names(&self) -> Vec<String> {
                self.life().counters.iter().map(|(k, _)| k.to_string()).collect()
            }
            /// Replaces the counters with an object's numbers, in its order.
            #[napi(js_name = "setCounters")]
            pub fn set_counters(&mut self, counters: crate::js::Json) {
                self.life_mut().counters = crate::blocks::counters_of(&counters.0);
            }
            #[napi(js_name = "takeChanges")]
            pub fn take_changes(&mut self) -> Vec<crate::blocks::Change> {
                self.life_mut().take_changes().into_iter().map(|c| crate::blocks::Change { from: c.from.as_str().into(), to: c.to.as_str().into(), at: c.at }).collect()
            }
            #[napi]
            pub fn health(&self) -> crate::js::Json {
                crate::js::Json(crate::blocks::health_of(self.life()))
            }
        }
    };
}
pub(crate) use lifecycle_api;

pub fn state_of_str(s: &str) -> Result<State> {
    state_of(s)
}

pub fn counters_of(v: &Value) -> Counters {
    let mut c = Counters::default();
    if let Value::Object(m) = v {
        for (k, x) in m {
            c.set(k, x.as_f64().unwrap_or(f64::NAN));
        }
    }
    c
}

#[napi(js_name = "blockStates")]
pub fn block_states() -> Vec<&'static str> {
    State::ALL.iter().map(|s| s.as_str()).collect()
}

/// A plain block's lifecycle, for a JS block (a subclass of Block that
/// isn't ported) or one that has nothing else in Rust yet.
#[napi(js_name = "Lifecycle")]
pub struct JsLifecycle {
    life: Lifecycle,
}

#[napi]
impl JsLifecycle {
    #[napi(constructor)]
    pub fn new(now: f64) -> Self {
        JsLifecycle { life: Lifecycle::new(now) }
    }
}

impl HasLife for JsLifecycle {
    fn life(&self) -> &Lifecycle {
        &self.life
    }
    fn life_mut(&mut self) -> &mut Lifecycle {
        &mut self.life
    }
}

lifecycle_api!(JsLifecycle);

/// The names of a list of blocks (core/blocks/lifecycle.js's Blocks).
#[napi(js_name = "Registry")]
#[derive(Default)]
pub struct JsRegistry {
    r: Registry,
}

#[napi]
impl JsRegistry {
    #[napi(constructor)]
    pub fn new() -> Self {
        JsRegistry { r: Registry::default() }
    }
    #[napi]
    pub fn add(&mut self, name: String, same: bool, source: Option<String>) -> Result<()> {
        self.r.add(&name, same, source.as_deref()).map_err(|e| Error::new(Status::InvalidArg, e))
    }
    #[napi]
    pub fn remove(&mut self, name: String) {
        self.r.remove(&name);
    }
    #[napi]
    pub fn names(&self) -> Vec<String> {
        self.r.names().to_vec()
    }
    #[napi]
    pub fn source(&self, name: String) -> Option<String> {
        self.r.source(&name).map(Into::into)
    }
}

// ----- the normalizer -----

/// The hooks a twin passes: JS functions wrapped so they don't throw. A
/// wrapper that caught an error returns `failed`, and `failure()` says what
/// it was. Without `parse`, lines are parsed here, as JSON.parse would.
#[allow(clippy::type_complexity)]
struct JsHooks<'a> {
    env: &'a Env,
    parse: Option<Function<'a, String, Unknown<'a>>>,
    inspect: Option<Function<'a, FnArgs<(Unknown<'a>, Unknown<'a>)>, Unknown<'a>>>,
    unparsable: Option<Function<'a, FnArgs<(String, Unknown<'a>, String)>, Unknown<'a>>>,
    normalize: Function<'a, FnArgs<(Unknown<'a>, Unknown<'a>)>, Unknown<'a>>,
    failed: Unknown<'a>,
    failure: Function<'a, (), String>,
}

impl<'a> JsHooks<'a> {
    fn checked(&self, r: Result<Unknown<'a>>) -> std::result::Result<Unknown<'a>, String> {
        let v = r.map_err(|e| e.reason.clone())?;
        let mut same = false;
        unsafe { sys::napi_strict_equals(self.env.raw(), v.raw(), self.failed.raw(), &mut same) };
        if same {
            Err(self.failure.call(()).unwrap_or_default())
        } else {
            Ok(v)
        }
    }

    fn nullish(v: &Unknown) -> bool {
        matches!(v.get_type(), Ok(ValueType::Null | ValueType::Undefined))
    }
}

impl<'a> Hooks<Unknown<'a>, Unknown<'a>, Unknown<'a>> for JsHooks<'a> {
    fn parse(&mut self, text: &str) -> std::result::Result<Unknown<'a>, String> {
        match &self.parse {
            Some(f) => self.checked(f.call(text.to_string())),
            None => {
                let v = js_json::parse(text).map_err(|e| e.to_string())?;
                let raw = unsafe { <Json as ToNapiValue>::to_napi_value(self.env.raw(), Json(v)) }.map_err(|e| e.reason.clone())?;
                Ok(unsafe { Unknown::from_raw_unchecked(self.env.raw(), raw) })
            }
        }
    }

    fn inspect(&mut self, raw: Unknown<'a>, where_: &Unknown<'a>) -> std::result::Result<Option<Unknown<'a>>, String> {
        let Some(f) = &self.inspect else { return Ok(Some(raw)) };
        let v = self.checked(f.call(FnArgs::from((raw, *where_))))?;
        Ok(if Self::nullish(&v) { None } else { Some(v) })
    }

    fn unparsable(&mut self, text: &str, where_: &Unknown<'a>, why: &str) {
        if let Some(f) = &self.unparsable {
            let _ = f.call(FnArgs::from((text.to_string(), *where_, why.to_string())));
        }
    }

    fn normalize(&mut self, raw: Unknown<'a>, where_: &Unknown<'a>) -> std::result::Result<Unknown<'a>, String> {
        self.checked(self.normalize.call(FnArgs::from((raw, *where_))))
    }

    fn inspects(&self) -> bool {
        self.inspect.is_some()
    }
}

#[napi(js_name = "Normalizer")]
pub struct JsNormalizer {
    n: Normalizer,
}

impl HasLife for JsNormalizer {
    fn life(&self) -> &Lifecycle {
        &self.n.life
    }
    fn life_mut(&mut self) -> &mut Lifecycle {
        &mut self.n.life
    }
}

lifecycle_api!(JsNormalizer);

#[napi]
impl JsNormalizer {
    #[napi(constructor)]
    pub fn new(name: String, source: Option<String>, now: f64) -> Self {
        JsNormalizer { n: Normalizer::new(&name, source.as_deref(), now) }
    }

    #[napi(getter)]
    pub fn source(&self) -> Option<String> {
        self.n.source.clone()
    }

    /// A line of text through the hooks; null when it isn't a record.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    #[napi]
    pub fn line(
        &mut self,
        env: &Env,
        text: String,
        where_: Unknown,
        parse: Option<Function<String, Unknown>>,
        inspect: Option<Function<FnArgs<(Unknown, Unknown)>, Unknown>>,
        unparsable: Option<Function<FnArgs<(String, Unknown, String)>, Unknown>>,
        normalize: Function<FnArgs<(Unknown, Unknown)>, Unknown>,
        failed: Unknown,
        failure: Function<(), String>,
        clock: Function<(), f64>,
    ) -> Option<Raw> {
        let mut hooks = JsHooks { env, parse, inspect, unparsable, normalize, failed, failure };
        self.n.line(&text, &where_, &mut hooks, || clock.call(()).unwrap_or(f64::NAN)).map(|v| Raw(v.raw()))
    }

    /// A record already parsed, through the hooks.
    #[allow(clippy::too_many_arguments)]
    #[napi]
    pub fn record(
        &mut self,
        env: &Env,
        raw: Unknown,
        where_: Unknown,
        inspect: Option<Function<FnArgs<(Unknown, Unknown)>, Unknown>>,
        normalize: Function<FnArgs<(Unknown, Unknown)>, Unknown>,
        failed: Unknown,
        failure: Function<(), String>,
        clock: Function<(), f64>,
    ) -> Option<Raw> {
        let mut hooks = JsHooks { env, parse: None, inspect, unparsable: None, normalize, failed, failure };
        self.n.record(raw, &where_, &mut hooks, || clock.call(()).unwrap_or(f64::NAN)).map(|v| Raw(v.raw()))
    }

    /// Records went through since the last look: lastOk is now.
    #[napi]
    pub fn look(&mut self, at: f64) {
        self.n.look(at);
    }
}

/// A JS value handed straight back to JS, within the call that made it.
pub struct Raw(pub sys::napi_value);

impl ToNapiValue for Raw {
    unsafe fn to_napi_value(_env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        Ok(val.0)
    }
}

impl TypeName for Raw {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

// ----- the scanner -----

use crate::js::{Field, Fields};
use xenonet_base::fs::{DirEntry, Fs, FsError, StdFs};
use xenonet_blocks::scanner::{Breadth, Halt, Last, Limits, Read, Rules, Scanner};

fn last_json(l: &Option<Last>) -> Value {
    match l {
        None => Value::Null,
        Some(l) => json!({ "files": l.files, "read": l.read, "seen": l.seen, "truncated": l.truncated, "ms": js_json::number(l.ms) }),
    }
}

/// A twin's rules: JS functions that answer true or false, and throw when
/// the walk has to stop (the twin keeps what they threw).
struct JsRules<'a> {
    pick: Function<'a, FnArgs<(String, String)>, bool>,
    enter: Function<'a, FnArgs<(String, String, u32)>, bool>,
}

impl Rules for JsRules<'_> {
    fn pick(&mut self, name: &str, dir: &str) -> std::result::Result<bool, Halt> {
        self.pick.call(FnArgs::from((name.to_string(), dir.to_string()))).map_err(|_| Halt)
    }
    fn enter(&mut self, name: &str, dir: &str, level: u32) -> std::result::Result<bool, Halt> {
        self.enter.call(FnArgs::from((name.to_string(), dir.to_string(), level))).map_err(|_| Halt)
    }
}

/// A twin's own fs (one a test passed): `stat(dir)` gives the mtime or
/// null, `readdir(dir)` the entries as [name, isDirectory, isFile], or
/// { code, message }.
struct JsFs<'a> {
    stat: Function<'a, String, Option<f64>>,
    readdir: Function<'a, String, Json>,
}

fn entries_of(v: &Value) -> std::result::Result<Vec<DirEntry>, FsError> {
    match v {
        Value::Array(a) => Ok(a
            .iter()
            .map(|e| DirEntry { name: e[0].as_str().unwrap_or_default().to_string(), is_dir: e[1].as_bool().unwrap_or(false), is_file: e[2].as_bool().unwrap_or(false) })
            .collect()),
        _ => Err(FsError { code: v["code"].as_str().map(Into::into), message: v["message"].as_str().unwrap_or_default().to_string() }),
    }
}

impl Fs for JsFs<'_> {
    fn mtime_ms(&mut self, path: &str) -> std::result::Result<f64, FsError> {
        match self.stat.call(path.to_string()) {
            Ok(Some(m)) => Ok(m),
            _ => Err(FsError { code: None, message: "couldn't stat".into() }),
        }
    }
    fn read_dir(&mut self, path: &str) -> std::result::Result<Vec<DirEntry>, FsError> {
        match self.readdir.call(path.to_string()) {
            Ok(Json(v)) => entries_of(&v),
            Err(e) => Err(FsError { code: None, message: e.reason.clone() }),
        }
    }
}

#[napi(js_name = "Scanner")]
pub struct JsScanner {
    s: Scanner,
}

impl HasLife for JsScanner {
    fn life(&self) -> &Lifecycle {
        &self.s.life
    }
    fn life_mut(&mut self) -> &mut Lifecycle {
        &mut self.s.life
    }
}

lifecycle_api!(JsScanner);

#[napi(js_name = "BreadthWalk")]
pub struct JsBreadth {
    w: Option<Breadth>,
}

impl JsBreadth {
    fn w(&mut self) -> Result<&mut Breadth> {
        self.w.as_mut().ok_or_else(|| Error::new(Status::GenericFailure, "the walk is over".to_string()))
    }
}

#[napi]
impl JsBreadth {
    #[napi]
    pub fn more(&self) -> bool {
        self.w.as_ref().is_some_and(Breadth::more)
    }
    #[napi]
    pub fn level(&self) -> Vec<String> {
        self.w.as_ref().map(|w| w.level().to_vec()).unwrap_or_default()
    }
    #[napi(getter)]
    pub fn n(&self) -> u32 {
        self.w.as_ref().map_or(0, |w| w.n)
    }
}

fn read_of(v: &Value) -> Read {
    Read { slot: v["slot"].as_u64().unwrap_or(0) as usize, dir: v["dir"].as_str().unwrap_or_default().to_string(), mtime_ms: v["mtimeMs"].as_f64() }
}

#[napi]
impl JsScanner {
    #[napi(constructor)]
    pub fn new(name: String, depth: f64, folders: f64, ms: f64, now: f64) -> Self {
        JsScanner { s: Scanner::new(&name, "", Limits { depth, folders, ms }, true, now) }
    }

    #[napi]
    pub fn forget(&mut self) {
        self.s.forget();
    }

    /// The folders known, as [dir, { mtimeMs, subdirs, racy }].
    #[napi]
    pub fn folders(&self) -> Json {
        Json(Value::Array(self.s.folders.iter().map(|(d, f)| json!([d, { "mtimeMs": f.mtime_ms.map(js_json::number), "subdirs": f.subdirs, "racy": f.racy }])).collect()))
    }

    #[napi(getter)]
    pub fn last(&self) -> Json {
        Json(last_json(&self.s.last))
    }

    #[napi(getter)]
    pub fn budget(&self) -> Fields {
        let b = self.s.budget;
        Fields(vec![("depth", Field::Num(b.depth)), ("folders", Field::Num(b.folders)), ("ms", Field::Num(b.ms)), ("last", Field::Json(last_json(&self.s.last)))])
    }

    /// A depth-first walk, with the twin's root, limits and rules. Without
    /// `stat` and `readdir`, it reads the disk itself.
    #[allow(clippy::too_many_arguments)]
    #[napi]
    pub fn scan(
        &mut self,
        root: String,
        depth: f64,
        folders: f64,
        ms: f64,
        use_cache: bool,
        pick: Function<FnArgs<(String, String)>, bool>,
        enter: Function<FnArgs<(String, String, u32)>, bool>,
        clock: Function<(), f64>,
        stat: Option<Function<String, Option<f64>>>,
        readdir: Option<Function<String, Json>>,
    ) -> Result<Vec<String>> {
        self.s.root = root;
        self.s.limits = Limits { depth, folders, ms };
        self.s.use_cache = use_cache;
        let mut rules = JsRules { pick, enter };
        let mut now = || clock.call(()).unwrap_or(f64::NAN);
        let found = match (stat, readdir) {
            (Some(stat), Some(readdir)) => self.s.scan(&mut JsFs { stat, readdir }, &mut rules, &mut now),
            _ => self.s.scan(&mut StdFs, &mut rules, &mut now),
        };
        found.map_err(|_| Error::new(Status::GenericFailure, "a rule threw".to_string()))
    }

    #[napi(js_name = "beginBreadth")]
    pub fn begin_breadth(&mut self, root: String, depth: f64, folders: f64, ms: f64, use_cache: bool, clock: Function<(), f64>) -> JsBreadth {
        self.s.use_cache = use_cache;
        let mut now = || clock.call(()).unwrap_or(f64::NAN);
        JsBreadth { w: Some(self.s.begin_breadth(&root, Limits { depth, folders, ms }, &mut now)) }
    }

    /// The level's folders to read, as { slot, dir, mtimeMs }.
    #[napi]
    pub fn plan(&mut self, w: &mut JsBreadth, mtimes: Option<Vec<Option<f64>>>) -> Result<Json> {
        let reads = self.s.plan(w.w()?, mtimes.as_deref());
        Ok(Json(Value::Array(reads.iter().map(|r| json!({ "slot": r.slot, "dir": r.dir, "mtimeMs": r.mtime_ms.map(js_json::number) })).collect())))
    }

    #[napi(js_name = "beginRead")]
    pub fn begin_read(&mut self, w: &mut JsBreadth, aborted: bool, clock: Function<(), f64>) -> Result<Option<f64>> {
        let mut now = || clock.call(()).unwrap_or(f64::NAN);
        Ok(self.s.begin_read(w.w()?, aborted, &mut now))
    }

    #[napi(js_name = "readFailed")]
    pub fn read_failed(&mut self, w: &mut JsBreadth, dir: String, code: Option<String>, message: String) -> Result<bool> {
        Ok(self.s.read_failed(w.w()?, &dir, &FsError { code, message }))
    }

    #[napi(js_name = "lookFailed")]
    pub fn look_failed(&mut self, w: &mut JsBreadth, dir: String, message: String) -> Result<()> {
        self.s.look_failed(w.w()?, &dir, &message);
        Ok(())
    }

    #[napi]
    pub fn skip(&mut self, w: &mut JsBreadth, read: Json) -> Result<()> {
        self.s.skip(w.w()?, read_of(&read.0).slot);
        Ok(())
    }

    /// A folder read: its entries as [name, isDirectory, isFile].
    #[napi(js_name = "finishRead")]
    pub fn finish_read(
        &mut self,
        w: &mut JsBreadth,
        read: Json,
        at: f64,
        entries: Json,
        pick: Function<FnArgs<(String, String)>, bool>,
        enter: Function<FnArgs<(String, String, u32)>, bool>,
    ) -> Result<()> {
        let ents = entries_of(&entries.0).unwrap_or_default();
        let mut rules = JsRules { pick, enter };
        self.s.finish_read(w.w()?, &read_of(&read.0), at, &ents, &mut rules).map_err(|_| Error::new(Status::GenericFailure, "a rule threw".to_string()))
    }

    #[napi(js_name = "endLevel")]
    pub fn end_level(&mut self, w: &mut JsBreadth) -> Result<()> {
        self.s.end_level(w.w()?);
        Ok(())
    }

    #[napi(js_name = "finishBreadth")]
    pub fn finish_breadth(&mut self, w: &mut JsBreadth, clock: Function<(), f64>) -> Result<Vec<String>> {
        let walk = w.w.take().ok_or_else(|| Error::new(Status::GenericFailure, "the walk is over".to_string()))?;
        let mut now = || clock.call(()).unwrap_or(f64::NAN);
        Ok(self.s.finish_breadth(walk, &mut now))
    }
}

// ----- the tailer -----

use xenonet_base::ordered::Cursor;
use xenonet_blocks::tailer::{Listed, Swept, Tailer};

fn listed(l: Listed) -> &'static str {
    match l {
        Listed::Skip => "skip",
        Listed::Dormant => "dormant",
        Listed::Track => "track",
    }
}

fn stat_of(mtime_ms: Option<f64>, size: Option<f64>) -> Option<(f64, u64)> {
    Some((mtime_ms?, size? as u64))
}

#[napi(js_name = "Tailer")]
pub struct JsTailer {
    t: Tailer,
}

impl HasLife for JsTailer {
    fn life(&self) -> &Lifecycle {
        &self.t.life
    }
    fn life_mut(&mut self) -> &mut Lifecycle {
        &mut self.t.life
    }
}

lifecycle_api!(JsTailer);

#[napi]
impl JsTailer {
    #[napi(constructor)]
    pub fn new(name: String, prime_window_ms: f64, now: f64) -> Self {
        JsTailer { t: Tailer::new(&name, prime_window_ms, now) }
    }

    #[napi(js_name = "listedAtStart")]
    pub fn listed_at_start(&self, file: String) -> bool {
        self.t.listed_at_start(&file)
    }

    #[napi(js_name = "atStart")]
    pub fn at_start(&mut self, file: String, mtime_ms: Option<f64>, size: Option<f64>, now: f64) -> &'static str {
        listed(self.t.at_start(&file, stat_of(mtime_ms, size), now))
    }

    #[napi]
    pub fn park(&mut self, file: String, size: f64) -> bool {
        self.t.park(&file, size as u64)
    }

    /// The next file a sweep looks at: [cursor, file], or null at the end.
    #[napi(js_name = "nextFollowed")]
    pub fn next_followed(&self, cursor: Option<f64>) -> Option<(f64, String)> {
        self.t.next_followed(Cursor::of(cursor.map(|c| c as u64))).map(|(c, f)| (c.at().unwrap_or(0) as f64, f))
    }

    #[napi(js_name = "inSweep")]
    pub fn in_sweep(&mut self, file: String, size: Option<f64>) -> &'static str {
        match self.t.in_sweep(&file, size.map(|s| s as u64)) {
            Swept::Drop => "drop",
            Swept::Read => "read",
            Swept::Nothing => "nothing",
        }
    }

    #[napi(js_name = "mayDiscover")]
    pub fn may_discover(&self, file: String) -> bool {
        self.t.may_discover(&file)
    }

    #[napi]
    pub fn discovered(&mut self, file: String, mtime_ms: Option<f64>, size: Option<f64>, clock: Function<(), f64>) -> &'static str {
        listed(self.t.discovered(&file, stat_of(mtime_ms, size), || clock.call(()).unwrap_or(f64::NAN)))
    }

    #[napi(js_name = "addFile")]
    pub fn add_file(&mut self, file: String) {
        self.t.add_file(&file);
    }

    #[napi(js_name = "takeHeld")]
    pub fn take_held(&mut self, file: String) -> Option<f64> {
        self.t.take_held(&file).map(|n| n as f64)
    }

    /// The bytes to read now, as [from, to], or null.
    #[napi(js_name = "planTake")]
    pub fn plan_take(&mut self, file: String, size: f64, until: Option<f64>) -> Option<Vec<f64>> {
        let until = until.filter(|u| u.is_finite()).map(|u| u as u64);
        self.t.plan_take(&file, size as u64, until).map(|(a, b)| vec![a as f64, b as f64])
    }

    #[napi]
    pub fn took(&mut self, file: String, data: Buffer, end: f64) -> Vec<String> {
        self.t.took(&file, &data, end as u64)
    }

    #[napi(js_name = "okAfterRead")]
    pub fn ok_after_read(&self, watchers: u32, roots: u32) -> bool {
        self.t.ok_after_read(watchers as usize, roots as usize)
    }

    #[napi]
    pub fn has(&self, file: String) -> bool {
        self.t.files.contains(&file)
    }

    #[napi]
    pub fn files(&self) -> Vec<String> {
        self.t.files.keys().map(str::to_string).collect()
    }

    #[napi(js_name = "fileCount")]
    pub fn file_count(&self) -> u32 {
        self.t.files.len() as u32
    }

    #[napi(js_name = "offsetOf")]
    pub fn offset_of(&self, file: String) -> Option<f64> {
        self.t.files.get(&file).map(|f| f.offset as f64)
    }

    #[napi(js_name = "restOf")]
    pub fn rest_of(&self, file: String) -> Option<Buffer> {
        self.t.files.get(&file).map(|f| Buffer::from(f.rest.clone()))
    }

    /// The dormant files, as [file, size].
    #[napi]
    pub fn dormant(&self) -> Vec<(String, f64)> {
        self.t.dormant.iter().map(|(k, v)| (k.to_string(), *v as f64)).collect()
    }

    #[napi(js_name = "dormantCount")]
    pub fn dormant_count(&self) -> u32 {
        self.t.dormant.len() as u32
    }
}

// ----- the JSON store -----

use crate::js::Jv;
use xenonet_blocks::store::{self, DocHooks, Report};

/// A twin's document hooks. `prepare` is guarded (it answers `failed` for a
/// document it can't use); `fresh`, `versionOf` and `prepareFresh` throw
/// when the caller's code does, which stops the read.
struct JsDocHooks<'a> {
    env: &'a Env,
    fresh: Function<'a, (), Unknown<'a>>,
    version_of: Function<'a, Unknown<'a>, Unknown<'a>>,
    migrate: Function<'a, FnArgs<(Unknown<'a>, f64, f64)>, Unknown<'a>>,
    prepare: Function<'a, Unknown<'a>, Unknown<'a>>,
    prepare_fresh: Function<'a, Unknown<'a>, Unknown<'a>>,
    failed: Unknown<'a>,
    failure: Function<'a, (), String>,
}

impl<'a> JsDocHooks<'a> {
    fn is_failed(&self, v: &Unknown<'a>) -> bool {
        let mut same = false;
        unsafe { sys::napi_strict_equals(self.env.raw(), v.raw(), self.failed.raw(), &mut same) };
        same
    }
}

impl<'a> DocHooks<Unknown<'a>> for JsDocHooks<'a> {
    fn fresh(&mut self) -> std::result::Result<Unknown<'a>, Halt> {
        self.fresh.call(()).map_err(|_| Halt)
    }
    fn parsed(&mut self, v: Value) -> std::result::Result<Unknown<'a>, Halt> {
        let raw = unsafe { crate::js::to_js(self.env.raw(), &v, 0) }.map_err(|_| Halt)?;
        Ok(unsafe { Unknown::from_raw_unchecked(self.env.raw(), raw) })
    }
    fn version_of(&mut self, doc: &Unknown<'a>) -> std::result::Result<Option<f64>, Halt> {
        let v = self.version_of.call(*doc).map_err(|_| Halt)?;
        Ok(match v.get_type() {
            Ok(ValueType::Number) => Some(unsafe { v.cast::<f64>() }.map_err(|_| Halt)?),
            _ => None,
        })
    }
    fn migrate(&mut self, doc: Unknown<'a>, from: f64, to: f64) -> std::result::Result<std::result::Result<Unknown<'a>, String>, Halt> {
        let v = self.migrate.call(FnArgs::from((doc, from, to))).map_err(|_| Halt)?;
        Ok(if self.is_failed(&v) { Err(self.failure.call(()).unwrap_or_default()) } else { Ok(v) })
    }
    fn prepare(&mut self, doc: &mut Unknown<'a>) -> std::result::Result<std::result::Result<Vec<String>, ()>, Halt> {
        let v = self.prepare.call(*doc).map_err(|_| Halt)?;
        if self.is_failed(&v) {
            return Ok(Err(()));
        }
        let list = unsafe { crate::js::from_js(self.env.raw(), v.raw(), 0) }.map_err(|_| Halt)?;
        Ok(Ok(list.as_array().map(|a| a.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default()))
    }
    fn prepare_fresh(&mut self, doc: &mut Unknown<'a>) -> std::result::Result<(), Halt> {
        self.prepare_fresh.call(*doc).map(|_| ()).map_err(|_| Halt)
    }
}

fn report_jv(r: Report) -> Jv {
    let opt = |s: Option<String>| Jv::Json(s.map_or(Value::Null, Value::String));
    let mut fields = vec![
        ("file", Jv::Json(Value::String(r.file))),
        ("version", Jv::Json(r.version.map_or(Value::Null, js_json::number))),
        ("migrated", Jv::Json(r.migrated.map_or(Value::Null, |(f, t)| json!({ "from": js_json::number(f), "to": js_json::number(t) })))),
        ("backup", opt(r.backup)),
        ("broken", opt(r.broken)),
        ("readOnly", Jv::Bool(r.read_only)),
        ("problem", opt(r.problem)),
    ];
    if !r.repaired.is_empty() {
        fields.push(("repaired", Jv::Json(json!(r.repaired))));
    }
    Jv::Obj(fields)
}

/// Reads a versioned JSON file through a twin's hooks: { doc, report }.
#[allow(clippy::too_many_arguments)]
#[napi(js_name = "readVersioned")]
pub fn read_versioned(
    env: &Env,
    file: String,
    latest: f64,
    now: f64,
    fresh: Function<(), Unknown>,
    version_of: Function<Unknown, Unknown>,
    migrate: Function<FnArgs<(Unknown, f64, f64)>, Unknown>,
    prepare: Function<Unknown, Unknown>,
    prepare_fresh: Function<Unknown, Unknown>,
    failed: Unknown,
    failure: Function<(), String>,
) -> Result<Jv> {
    let mut hooks = JsDocHooks { env, fresh, version_of, migrate, prepare, prepare_fresh, failed, failure };
    let (doc, report) = store::read_versioned(&file, latest, now, &mut hooks).map_err(|_| Error::new(Status::GenericFailure, "a hook threw".to_string()))?;
    Ok(Jv::Obj(vec![("doc", Jv::Raw(doc.raw())), ("report", report_jv(report))]))
}

#[napi(js_name = "freeName")]
pub fn free_name(file: String, base: String, now: f64, ext: String) -> String {
    store::free_name(&file, &base, now, &ext)
}

/// Writes a document as JSON.stringify(doc, null, indent) would, through a
/// temporary file renamed into place: null, or the error, as Node words it.
#[napi(js_name = "writeJson")]
pub fn write_json(file: String, doc: Json, indent: u32) -> Option<Jv> {
    let text = js_json::stringify(&doc.0, indent as usize);
    store::write_atomic(&file, &text).err().map(|e| Jv::Obj(vec![("code", Jv::Json(e.code.map_or(Value::Null, Value::String))), ("message", Jv::Json(Value::String(e.message)))]))
}
