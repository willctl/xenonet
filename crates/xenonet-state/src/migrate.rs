//! Versioned documents XenoNet keeps, such as state.json
//! (core/migrate.js): a list of steps, each a function from one version's
//! shape to the next one's, run in order from the document's version up to
//! the one wanted. A document is never taken back to an older version.
//!
//! `plan` says which steps take a document from `from` to `to`, or why
//! none do, in the JS's words; whoever has the steps runs them, on a copy.

use serde_json::Value;
use xenonet_base::text::js_string;

/// A step's versions: from, to.
pub type Step = (f64, f64);

fn whole(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.fract() == 0.0)
}

fn shown(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), js_string)
}

/// The steps to run, by their place in `steps`, from `from` to `to`.
pub fn plan(steps: &[Step], from: Option<&Value>, to: Option<&Value>) -> Result<Vec<usize>, String> {
    let (Some(f), Some(t)) = (whole(from), whole(to)) else {
        return Err(format!("versions are whole numbers, not {} and {}", shown(from), shown(to)));
    };
    if f > t {
        return Err(format!("version {} can't go back to {}", shown(from), shown(to)));
    }
    let mut out = Vec::new();
    let mut v = f;
    while v < t {
        let i = steps.iter().position(|s| s.0 == v);
        match i {
            Some(i) if steps[i].1 > steps[i].0 && steps[i].1 <= t => {
                out.push(i);
                v = steps[i].1;
            }
            _ => return Err(format!("there's no way from version {} to {}", js_string(&xenonet_base::json::number(v)), shown(to))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plans_as_migrate_does() {
        let steps = [(1.0, 2.0), (2.0, 3.0)];
        assert_eq!(plan(&steps, Some(&json!(1)), Some(&json!(3))), Ok(vec![0, 1]));
        assert_eq!(plan(&steps, Some(&json!(2)), Some(&json!(2))), Ok(vec![]));
        assert_eq!(plan(&steps, Some(&json!(3)), Some(&json!(1))).unwrap_err(), "version 3 can't go back to 1");
        assert_eq!(plan(&steps, Some(&json!(1)), Some(&json!(4))).unwrap_err(), "there's no way from version 3 to 4");
        assert_eq!(plan(&[(1.0, 3.0)], Some(&json!(1)), Some(&json!(2))).unwrap_err(), "there's no way from version 1 to 2");
        assert_eq!(plan(&steps, Some(&json!("1")), Some(&json!(3))).unwrap_err(), "versions are whole numbers, not 1 and 3");
    }
}
