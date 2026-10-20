//! core/components/console-policy.js: an installed XenoNet offers the
//! supported driver to the console tools, never arbitrary page execution.

use serde_json::Value;

const UPDATES: [&str; 5] = ["status", "check", "automatic", "components", "component-update"];

fn method_ok(m: &str) -> bool {
    let mut cs = m.chars();
    cs.next().is_some_and(|c| c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_alphanumeric()) && !["constructor", "prototype"].contains(&m)
}

/// Why an action isn't allowed in production, if it isn't.
pub fn console_policy(op: Option<&Value>, method: Option<&Value>, args: Option<&Value>, production: bool) -> Result<(), String> {
    if !production {
        return Ok(());
    }
    let op = op.and_then(Value::as_str);
    if matches!(op, Some("eval" | "cdp")) {
        return Err("Arbitrary page execution is available only in development instances.".into());
    }
    if op == Some("driver") {
        // RegExp.test reads the method as a string.
        let m = method.map_or_else(|| "undefined".to_string(), xenonet_base::text::js_string);
        if !method_ok(&m) {
            return Err("Unknown console action.".into());
        }
        if method.and_then(Value::as_str) == Some("updates") {
            let first = args.and_then(|a| a.get(0));
            let fine = match first {
                None => true,
                Some(Value::String(s)) => UPDATES.contains(&s.as_str()),
                Some(_) => false,
            };
            if !fine {
                return Err("Install Desktop updates through XenoNet's Updates panel.".into());
            }
        }
    }
    Ok(())
}
