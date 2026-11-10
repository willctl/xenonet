//! What the runtime's modules share: the time, a node's fields, and the
//! cores' answers (absent fields, NaN, objects built a key at a time) as
//! the JSON the service sends, in the format xenonet-text writes.

use serde_json::{Map, Value};
use xenonet_text::json::number;

/// Date.now().
pub use xenonet_text::time::now_ms;

/// The clock the map's runtime reads: the system's, or one a test
/// gives it, so what a refresh makes of a fixed world comes out the same
/// every run.
#[derive(Clone)]
pub struct Clock(std::sync::Arc<dyn Fn() -> f64 + Send + Sync>);

impl Clock {
    /// Date.now().
    pub fn system() -> Clock {
        Clock(std::sync::Arc::new(now_ms))
    }

    /// A clock that reads `f`.
    pub fn new(f: impl Fn() -> f64 + Send + Sync + 'static) -> Clock {
        Clock(std::sync::Arc::new(f))
    }

    /// A clock stopped at `ms`.
    pub fn fixed(ms: f64) -> Clock {
        Clock::new(move || ms)
    }

    /// The time, in ms since 1970.
    pub fn now(&self) -> f64 {
        (self.0)()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::system()
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Clock")
    }
}

pub fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

pub fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    get(v, k).and_then(Value::as_str)
}

/// `v ?? null` for a field.
pub fn or_null(v: &Value, k: &str) -> Value {
    get(v, k).cloned().unwrap_or(Value::Null)
}

pub fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// A number JSON can hold, or null, as JSON.stringify writes NaN.
pub fn num(n: f64) -> Value {
    number(n)
}

/// An object from its fields, in order.
pub fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The live telemetry's fields (undefined as None) as JSON: undefined
/// left out.
pub fn fields(f: &[(String, Option<Value>)]) -> Value {
    let mut m = Map::new();
    for (k, v) in f {
        if let Some(v) = v {
            m.insert(k.clone(), v.clone());
        }
    }
    Value::Object(m)
}

/// The native readers' answers as JSON.
pub fn native_out(o: &xenonet_sources::native::Out) -> Option<Value> {
    use xenonet_sources::native::Out;
    match o {
        Out::Undef => None,
        Out::Num(n) => Some(number(*n)),
        Out::Json(v) => Some(v.clone()),
        Out::Arr(a) => Some(Value::Array(a.iter().map(|x| native_out(x).unwrap_or(Value::Null)).collect())),
        Out::Obj(fs) => {
            let mut m = Map::new();
            for (k, v) in fs {
                if let Some(v) = native_out(v) {
                    m.insert((*k).to_string(), v);
                }
            }
            Some(Value::Object(m))
        }
    }
}

/// wiki.rs's answers (which stats and the wiki give) as JSON.
pub fn wiki_out(o: &xenonet_sources::wiki::Out) -> Option<Value> {
    use xenonet_sources::wiki::Out;
    match o {
        Out::Undef => None,
        Out::Null => Some(Value::Null),
        Out::Bool(b) => Some(Value::Bool(*b)),
        Out::Num(n) => Some(number(*n)),
        Out::Str(s) => Some(Value::String(s.clone())),
        // A Buffer, as JSON.stringify writes one.
        Out::Bytes(b) => Some(serde_json::json!({ "type": "Buffer", "data": b })),
        Out::Json(v) => Some(v.clone()),
        Out::Arr(a) => Some(Value::Array(a.iter().map(|x| wiki_out(x).unwrap_or(Value::Null)).collect())),
        Out::Obj(fs) | Out::Assigned(fs) => {
            let mut m = Map::new();
            for (k, v) in fs {
                // `o.__proto__ = v` sets the prototype, not a key.
                if matches!(o, Out::Assigned(_)) && k == "__proto__" {
                    continue;
                }
                if let Some(v) = wiki_out(v) {
                    m.insert(k.clone(), v);
                }
            }
            Some(Value::Object(m))
        }
    }
}

/// The message of a reader's type error, as a core gives it.
pub fn type_error(e: &xenonet_sources::lines::TypeError) -> String {
    e.0.clone()
}

/// Runs `f` off the runtime's threads: it reads the disk or a database.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// The local date and time at `ms`: year, month, day, hour, minute, in
/// the system's time zone.
pub fn local(ms: f64) -> [i64; 5] {
    let t = ms + xenonet_text::utf16::local_offset_ms(ms);
    let days = (t / 86400e3).floor();
    let (y, m, d) = xenonet_text::time::civil_from_days(days as i64);
    let rest = t - days * 86400e3;
    let minutes = (rest / 60e3).floor() as i64;
    [y, i64::from(m), i64::from(d), minutes / 60, minutes % 60]
}

/// The local date as en-CA writes it: YYYY-MM-DD.
pub fn local_day(ms: f64) -> String {
    let [y, m, d, ..] = local(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A lock, whichever thread held it last: a panic elsewhere while holding
/// it leaves what it guards as it was, which the drivers can still use.
pub fn lock<T: ?Sized>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

static EMITTER: std::sync::OnceLock<xenonet_api::wire::Emitter> = std::sync::OnceLock::new();

/// Where the drivers' log lines go: the service's error log.
pub fn set_log(emitter: &xenonet_api::wire::Emitter) {
    let _ = EMITTER.set(emitter.clone());
}

/// A line in the service's log (level error, warn or info).
pub fn log(level: &str, code: &str, message: &str) {
    if let Some(e) = EMITTER.get() {
        e.log(level, code, message);
    }
}
