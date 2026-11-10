//! Remote actors (docs/design/cloud-assistants.md): the connections you
//! pair and their grants (core/actors/connections.js, connections.rs), what
//! they read and how (core/assistant/source.js, projection.js, service.js,
//! fixtures.js, audit.js: reads.rs), and the tunnels XenoNet runs for them
//! (core/assistant/tunnels.js, tunnel-client.js: tunnels.rs).
//!
//! Their rows are the engine's, in engine.sqlite, and the actors they
//! belong to are the actors' registry's: both through the engine's driver
//! (EngineStore::native, and the Actors trait). Until it gives them, these
//! cores stay pending.

pub mod connections;
pub mod reads;
pub mod tunnels;

use serde_json::{Map, Value};
use xenonet_api::cores::{CoreError, Kind};
use xenonet_engine::sql::{Cell, Row};

/// A row of the engine's store as JSON: each column as node:sqlite read it,
/// an integer as a number.
pub fn row_json(r: &Row) -> Value {
    let cell = |c: &Cell| match c {
        Cell::Null => Value::Null,
        Cell::Int(i) => xenonet_base::json::number(*i as f64),
        Cell::Real(f) => xenonet_base::json::number(*f),
        Cell::Text(s) => Value::String(s.clone()),
        Cell::Blob(b) => Value::Array(b.iter().map(|x| Value::from(*x)).collect()),
    };
    Value::Object(r.0.iter().map(|(k, c)| (k.clone(), cell(c))).collect())
}

/// A connection's refusal as the routes answer it (ConnectionError), a
/// link's (LinkError), or what the store or a collaborator failed with.
pub fn connection_error(f: xenonet_actors::connections::Fail) -> CoreError {
    use xenonet_actors::connections::Fail;
    match f {
        Fail::Connection { status, code, message, extra } => CoreError::of(Kind::Connection, status as u16, code, message).with_extra(extra),
        Fail::Link { code, message } => {
            let mut e = CoreError::new(message);
            e.code = Some(code.to_string());
            e
        }
        Fail::Error(e) => {
            let mut c = CoreError::new(e.message);
            c.code = e.code;
            c
        }
    }
}

/// os.hostname() and os.userInfo().username, which a device's fingerprint
/// is made from.
#[cfg_attr(unix, expect(unsafe_code, reason = "gethostname(3) and getpwuid(3), which std doesn't wrap"))]
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
