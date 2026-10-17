//! core/components/protocol.js: component releases change on their own;
//! these wire contracts don't.

use serde_json::Value;

/// A range of interface versions, as each side says it.
pub type Api = (i64, i64);

pub const SERVICE_API: Api = (1, 1);
pub const CONSOLE_API: Api = (1, 1);
/// "assistant-1": remote actors' read-only endpoint, at assistant API 1.
pub const CAPABILITIES: [&str; 4] = ["console", "terminals", "terminal-reattach", "assistant-1"];

fn int(v: Option<&Value>) -> Option<i64> {
    let n = v?.as_f64()?;
    (n.fract() == 0.0 && n.is_finite()).then_some(n as i64)
}

/// The highest version both sides speak.
pub fn negotiate(local: Option<&Value>, remote: Option<&Value>) -> Result<i64, String> {
    let get = |v: Option<&Value>, k: &str| int(v.and_then(|v| v.get(k)));
    let (Some(lmin), Some(lmax), Some(rmin), Some(rmax)) = (get(local, "min"), get(local, "max"), get(remote, "min"), get(remote, "max")) else {
        return Err("XenoNet did not report a supported interface version. Update the service first.".into());
    };
    let version = lmax.min(rmax);
    if lmin > lmax || rmin > rmax || version < lmin.max(rmin) {
        return Err("These XenoNet components use incompatible interfaces. Open Updates or run xenonet update.".into());
    }
    Ok(version)
}

/// An Api as JSON, { min, max }.
pub fn api_value(a: Api) -> Value {
    serde_json::json!({ "min": a.0, "max": a.1 })
}
