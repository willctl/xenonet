//! Remote actors (docs/design/cloud-assistants.md): the connections you
//! pair and their grants (core/actors/connections.js, connections.rs), what
//! they read and how (core/assistant/source.js, projection.js, service.js,
//! fixtures.js, audit.js: reads.rs), and the tunnels XenoNet runs for them
//! (core/assistant/tunnels.js, tunnel-client.js: tunnels.rs).
//!
//! Their rows are the engine's, in engine.sqlite, and the actors they
//! belong to are the actors' registry's: both through the engine's driver
//! (EngineStore::native, Actors::registry). Until it gives them, these
//! cores stay pending.

pub mod connections;
pub mod reads;
pub mod tunnels;

use serde_json::{Map, Value};
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::Arg;
use xenonet_service::cores::{CoreError, Kind};

/// A JSON value as the cores' JS-shaped arguments.
pub fn arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::Text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(arg).collect()),
        Value::Object(m) => Arg::Obj(m.iter().map(|(k, v)| (k.clone(), arg(v))).collect()),
    }
}

pub fn thrown(t: Thrown) -> CoreError {
    let mut e = CoreError::new(t.message);
    e.code = t.code.as_ref().map(xenonet_base::text::js_string);
    e
}

pub fn thrown_of(e: &CoreError) -> Thrown {
    let mut t = Thrown::new(e.message.clone());
    t.code = e.code.clone().map(Value::String);
    t
}

/// A connection's refusal as the routes answer it (ConnectionError), a
/// link's (LinkError), or what the core threw.
pub fn connection_error(f: xenonet_actors::connections::Fail) -> CoreError {
    use xenonet_actors::connections::Fail;
    match f {
        Fail::Connection { status, code, message, extra } => CoreError::of(Kind::Connection, status as u16, code, message).with_extra(extra),
        Fail::Link { code, message } => {
            let mut e = CoreError::new(message);
            e.code = Some(code.to_string());
            e
        }
        Fail::Thrown(t) => thrown(t),
    }
}

/// os.hostname() and os.userInfo().username, which a device's fingerprint
/// is made from.
pub fn host_and_user() -> (String, String) {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most the buffer's length.
        let host = if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        };
        // SAFETY: getpwuid hands back the process's own entry, read at once.
        let user = unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() || (*pw).pw_name.is_null() {
                std::env::var("USER").unwrap_or_default()
            } else {
                std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned()
            }
        };
        (host, user)
    }
    #[cfg(not(unix))]
    {
        (std::env::var("COMPUTERNAME").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default())
    }
}

/// `{ ...a, ...b }` for objects.
pub fn spread(a: Value, b: &Map<String, Value>) -> Value {
    let mut out = a;
    if let Value::Object(o) = &mut out {
        for (k, v) in b {
            o.insert(k.clone(), v.clone());
        }
    }
    out
}
