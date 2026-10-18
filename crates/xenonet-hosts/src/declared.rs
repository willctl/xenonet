//! core/hosts/declared.js: what a host sends, kept as XenoNet's schemas
//! declare it. Every object in the schemas is loose, so a value that
//! passes its check still carries keys the schema never names, which the
//! page might read as its own and write into its markup unescaped.
//! declared() copies a value that has passed its schema with the declared
//! fields alone, at every level.
//!
//! The schemas are zod's, in JS: the twin reads each one's shape off it
//! once, as a table of the shapes below, and a union's options are tried
//! by the function given (zod's safeParse).

use serde_json::{Map, Value};

/// One schema's shape, its parts by their index in the table.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// optional, nullable, default, prefault, nonoptional, readonly, catch:
    /// null and missing stay, anything else goes through `inner`.
    Wrap(usize),
    /// pipe (its input) and lazy (what its getter gives).
    Same(usize),
    Array(usize),
    Tuple {
        items: Vec<usize>,
        rest: Option<usize>,
    },
    /// A record keeps its keys, which its schema declares as a kind of key.
    Record(usize),
    Union(Vec<usize>),
    /// An object's declared fields (an intersection's from both sides).
    Object(Vec<(String, usize)>),
    /// unknown and any: an object where the schema takes anything keeps
    /// none of its own.
    Any,
    /// Everything else, kept as it is.
    Leaf,
}

/// A table's shape from its JSON: [kind, …]: ["wrap", i], ["same", i],
/// ["array", i], ["tuple", [i…], rest|null], ["record", i], ["union", [i…]],
/// ["object", [[name, i]…]], ["any"], ["leaf"].
pub fn shape_of(v: &Value) -> Shape {
    let a = v.as_array();
    let at = |i: usize| a.and_then(|a| a.get(i));
    let idx = |x: Option<&Value>| x.and_then(Value::as_u64).map_or(0, |n| n as usize);
    let list = |x: Option<&Value>| x.and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_u64).map(|n| n as usize).collect()).unwrap_or_default();
    match at(0).and_then(Value::as_str) {
        Some("wrap") => Shape::Wrap(idx(at(1))),
        Some("same") => Shape::Same(idx(at(1))),
        Some("array") => Shape::Array(idx(at(1))),
        Some("tuple") => Shape::Tuple { items: list(at(1)), rest: at(2).and_then(Value::as_u64).map(|n| n as usize) },
        Some("record") => Shape::Record(idx(at(1))),
        Some("union") => Shape::Union(list(at(1))),
        Some("object") => Shape::Object(
            at(1).and_then(Value::as_array).map(|fs| fs.iter().filter_map(|f| Some((f.get(0)?.as_str()?.to_string(), f.get(1)?.as_u64()? as usize))).collect()).unwrap_or_default(),
        ),
        Some("any") => Shape::Any,
        _ => Shape::Leaf,
    }
}

/// A copy of `value` with what shape `at` of `table` declares, and
/// nothing else. `parses(i, v)` says whether shape i's schema takes v.
pub fn declared(table: &[Shape], at: usize, value: &Value, parses: &mut dyn FnMut(usize, &Value) -> bool) -> Value {
    let Some(shape) = table.get(at) else { return value.clone() };
    match shape {
        Shape::Wrap(i) => {
            if value.is_null() {
                value.clone()
            } else {
                declared(table, *i, value, parses)
            }
        }
        Shape::Same(i) => declared(table, *i, value, parses),
        Shape::Array(el) => match value {
            Value::Array(a) => Value::Array(a.iter().map(|v| declared(table, *el, v, parses)).collect()),
            _ => value.clone(),
        },
        Shape::Tuple { items, rest } => match value {
            Value::Array(a) => {
                let n = if rest.is_some() { a.len() } else { items.len().min(a.len()) };
                Value::Array(
                    a[..n]
                        .iter()
                        .enumerate()
                        .map(|(i, v)| match items.get(i).copied().or(*rest) {
                            Some(s) => declared(table, s, v, parses),
                            None => v.clone(),
                        })
                        .collect(),
                )
            }
            _ => value.clone(),
        },
        Shape::Record(v_shape) => match value {
            Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), declared(table, *v_shape, v, parses))).collect()),
            _ => value.clone(),
        },
        Shape::Union(options) => match options.iter().find(|o| parses(**o, value)) {
            Some(o) => declared(table, *o, value, parses),
            None => value.clone(),
        },
        Shape::Object(fields) => match value {
            Value::Object(m) => {
                let mut out = Map::new();
                for (k, f) in fields {
                    if let Some(v) = m.get(k) {
                        out.insert(k.clone(), declared(table, *f, v, parses));
                    }
                }
                Value::Object(out)
            }
            _ => value.clone(),
        },
        Shape::Any => match value {
            Value::Array(a) => Value::Array(a.iter().map(|v| declared(table, at, v, parses)).collect()),
            Value::Object(_) => Value::Object(Map::new()),
            _ => value.clone(),
        },
        Shape::Leaf => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_what_it_declares() {
        // { a: string, list: any[], counts: record<{ n }>, either: union({ k } | string) }
        let table = vec![
            Shape::Object(vec![("a".into(), 1), ("list".into(), 2), ("counts".into(), 4), ("either".into(), 6)]),
            Shape::Leaf,
            Shape::Array(3),
            Shape::Any,
            Shape::Record(5),
            Shape::Object(vec![("n".into(), 1)]),
            Shape::Union(vec![7, 1]),
            Shape::Object(vec![("k".into(), 1)]),
        ];
        let v = json!({ "a": "a", "extra": 1, "list": ["s", { "x": 1 }, [{}]], "counts": { "any": { "n": 1, "m": 2 } }, "either": { "k": 1, "z": 2 } });
        let got = declared(&table, 0, &v, &mut |i, v| i == 7 && v.is_object());
        assert_eq!(got, json!({ "a": "a", "list": ["s", {}, [{}]], "counts": { "any": { "n": 1 } }, "either": { "k": 1 } }));
    }
}
