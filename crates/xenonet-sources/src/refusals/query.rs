//! core/refusals/query.js: the refusals browser's query over every record:
//! the facets, the filters and words, the sort and the page.

use serde_json::{Map, Value};
use std::cmp::Ordering;
use xenonet_base::json::number;
use xenonet_base::jsstr::{collapse_space, locale_compare};
use xenonet_base::text::{js_number, js_string, js_trim, truthy};

pub const REFUSAL_SORTS: [&str; 6] = ["newest", "oldest", "provider", "model", "category", "kind"];

// String(v ?? "")
fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k).filter(|_| v.is_object()))
}

// a || b || ""
fn first_truthy(vs: [Option<&Value>; 2]) -> Value {
    vs.into_iter().find(|v| truthy(*v)).flatten().cloned().unwrap_or_else(|| Value::String(String::new()))
}

/// A record as the browser gets it: without its dedupe key or the
/// transcript's path, its source last.
pub fn public_record(r: &Value) -> Value {
    let Some(m) = r.as_object() else { return r.clone() };
    let mut out: Map<String, Value> = m.iter().filter(|(k, _)| *k != "source" && *k != "key").map(|(k, v)| (k.clone(), v.clone())).collect();
    let source = m.get("source");
    let shown = if truthy(source) {
        let mut s = Map::new();
        if let Some(f) = source.and_then(|s| s.get("file")) {
            s.insert("file".into(), f.clone());
        }
        Value::Object(s)
    } else {
        Value::Null
    };
    out.insert("source".into(), shown);
    Value::Object(out)
}

pub fn model_of(r: &Value) -> Value {
    first_truthy([at(r, &["signal", "model"]), at(r, &["session", "model"])])
}

pub fn category_of(r: &Value) -> Value {
    first_truthy([at(r, &["signal", "category"]), None])
}

fn facet(all: &[&Value], f: impl Fn(&Value) -> Value) -> Value {
    let mut seen: Vec<Value> = Vec::new();
    for r in all {
        let v = f(r);
        if truthy(Some(&v)) && !seen.contains(&v) {
            seen.push(v);
        }
    }
    seen.sort_by(|a, b| locale_compare(&js_string(a), &js_string(b)));
    Value::Array(seen)
}

/// queryRefusals(records, q): { total, matched, offset, limit, facets,
/// records }.
pub fn query_refusals(records: &[Value], q: &Value) -> Value {
    let qv = |k: &str| q.get(k).filter(|_| q.is_object());
    let at_of = |r: &Value| r.get("at").and_then(Value::as_f64);
    let all: Vec<&Value> = records.iter().filter(|r| r.is_object() && r.get("id").is_some_and(Value::is_string) && at_of(r).is_some_and(f64::is_finite)).collect();
    let field = |r: &Value, k: &str| r.get(k).cloned();
    let mut facets = Map::new();
    facets.insert("provider".into(), facet(&all, |r| field(r, "provider").unwrap_or(Value::Null)));
    facets.insert("model".into(), facet(&all, model_of));
    facets.insert("category".into(), facet(&all, category_of));
    facets.insert("kind".into(), facet(&all, |r| field(r, "kind").unwrap_or(Value::Null)));
    let words: Vec<String> = collapse_space(&js_trim(&text(qv("q"))).to_lowercase()).split(' ').filter(|w| !w.is_empty()).map(String::from).collect();
    let or0 = |n: f64| if n.is_nan() || n == 0.0 { 0.0 } else { n };
    let since = or0(js_number(qv("since")));
    let until = match js_number(qv("until")) {
        n if n.is_nan() || n == 0.0 => f64::INFINITY,
        n => n,
    };
    let differs = |a: Option<&Value>, b: &Value| a != Some(b);
    let mut found: Vec<&Value> = all
        .iter()
        .copied()
        .filter(|r| {
            let t = at_of(r).unwrap();
            if t < since || t > until {
                return false;
            }
            if let Some(p) = qv("provider").filter(|v| truthy(Some(v))) {
                if differs(r.get("provider"), p) && differs(r.get("app"), p) {
                    return false;
                }
            }
            if let Some(k) = qv("kind").filter(|v| truthy(Some(v))) {
                if differs(r.get("kind"), k) {
                    return false;
                }
            }
            if let Some(m) = qv("model").filter(|v| truthy(Some(v))) {
                if &model_of(r) != m {
                    return false;
                }
            }
            if let Some(c) = qv("category").filter(|v| truthy(Some(v))) {
                if &category_of(r) != c {
                    return false;
                }
            }
            let context = qv("context").and_then(Value::as_str);
            if (context == Some("with") && !truthy(r.get("context"))) || (context == Some("without") && truthy(r.get("context"))) {
                return false;
            }
            let hay = [
                r.get("id").cloned(),
                r.get("provider").cloned(),
                r.get("app").cloned(),
                r.get("kind").cloned(),
                Some(model_of(r)),
                Some(category_of(r)),
                at(r, &["signal", "text"]).cloned(),
                at(r, &["signal", "explanation"]).cloned(),
                at(r, &["signal", "requestId"]).cloned(),
                at(r, &["signal", "fallbackModel"]).cloned(),
                at(r, &["session", "id"]).cloned(),
                at(r, &["session", "cwd"]).cloned(),
                at(r, &["source", "file"]).cloned(),
            ]
            .iter()
            .map(|v| text(v.as_ref()))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect();
    let sort = qv("sort").and_then(Value::as_str).filter(|s| REFUSAL_SORTS.contains(s)).unwrap_or("newest");
    let id = |r: &Value| r["id"].as_str().unwrap_or("").to_string();
    found.sort_by(|a, b| {
        let by_field = if sort == "newest" || sort == "oldest" {
            Ordering::Equal
        } else {
            let f = |r: &Value| match sort {
                "model" => Some(model_of(r)),
                "category" => Some(category_of(r)),
                k => r.get(k).cloned(),
            };
            locale_compare(&text(f(a).as_ref()), &text(f(b).as_ref()))
        };
        let (ta, tb) = (at_of(a).unwrap(), at_of(b).unwrap());
        let by_time = if sort == "oldest" { ta.partial_cmp(&tb) } else { tb.partial_cmp(&ta) }.unwrap_or(Ordering::Equal);
        by_field.then(by_time).then_with(|| locale_compare(&id(a), &id(b)))
    });
    let limit_n = js_number(qv("limit"));
    let limit = if limit_n.is_finite() { 500f64.min(1f64.max((if limit_n == 0.0 { 500.0 } else { limit_n }).floor())) } else { 500.0 };
    let offset_n = js_number(qv("offset"));
    let offset = if offset_n.is_finite() { 0f64.max(offset_n.floor()) } else { 0.0 };
    let page: Vec<Value> = found.iter().skip(offset as usize).take(limit as usize).map(|r| public_record(r)).collect();
    let mut out = Map::new();
    out.insert("total".into(), number(all.len() as f64));
    out.insert("matched".into(), number(found.len() as f64));
    out.insert("offset".into(), number(offset));
    out.insert("limit".into(), number(limit));
    out.insert("facets".into(), Value::Object(facets));
    out.insert("records".into(), Value::Array(page));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn words_filters_and_pages() {
        let records = super::super::demo::demo_refusals(1e8);
        let all = query_refusals(&records, &json!({}));
        assert_eq!(all["total"], 4);
        assert_eq!(all["facets"]["provider"], json!(["anthropic", "openai"]));
        assert_eq!(query_refusals(&records, &json!({ "provider": "codex" }))["matched"], 2);
        assert_eq!(query_refusals(&records, &json!({ "context": "with" }))["matched"], 1);
        assert_eq!(query_refusals(&records, &json!({ "q": "  FALLBACK   model " }))["matched"], 2);
        assert_eq!(query_refusals(&records, &json!({ "sort": "oldest" }))["records"][0]["id"], records[3]["id"]);
        assert_eq!(query_refusals(&records, &json!({ "limit": "2", "offset": 3 }))["records"].as_array().unwrap().len(), 1);
        let shown = &all["records"][0];
        assert_eq!(shown.as_object().unwrap().keys().last().unwrap(), "source");
    }
}
