//! xenonet-base for the twins of core/pricing.js and core/paths.js.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use xenonet_base::{paths, pricing, time};

// ----- pricing -----

#[napi(js_name = "pricesChecked")]
pub fn prices_checked() -> &'static str {
    pricing::PRICES_CHECKED
}

#[napi(js_name = "priorityTier")]
pub fn priority_tier(tier: String) -> bool {
    pricing::priority_tier(&tier)
}

#[napi(js_name = "plainModel")]
pub fn plain_model(model: String) -> String {
    pricing::plain_model(&model)
}

// A price row as core/pricing.js builds it: its rates, and longContext and
// fast only when the row has them.
fn row_value(p: &pricing::Price) -> Value {
    let mut o = Map::new();
    o.insert("provider".into(), json!(p.provider.as_str()));
    o.insert("ids".into(), json!(p.ids));
    o.insert("from".into(), json!(p.from));
    o.insert("input".into(), json!(p.input));
    o.insert("output".into(), json!(p.output));
    o.insert("write5m".into(), json!(p.write5m));
    o.insert("write1h".into(), json!(p.write1h));
    o.insert("read".into(), json!(p.read));
    if let Some(l) = &p.long_context {
        let mut lc = Map::new();
        lc.insert("over".into(), json!(l.over));
        if let Some(f) = l.from {
            lc.insert("from".into(), json!(f));
        }
        if let Some(u) = l.until {
            lc.insert("until".into(), json!(u));
        }
        lc.insert("input".into(), json!(l.input));
        lc.insert("output".into(), json!(l.output));
        if let Some(s) = l.scale_cache {
            lc.insert("scaleCache".into(), json!(s));
        }
        o.insert("longContext".into(), Value::Object(lc));
    }
    if let Some(f) = p.fast {
        o.insert("fast".into(), json!({ "input": f.input, "output": f.output }));
    }
    Value::Object(o)
}

#[napi(js_name = "priceOf")]
pub fn price_of(model: String, at: Option<f64>) -> Option<Value> {
    pricing::price_of(&model, at).map(row_value)
}

#[napi(object)]
pub struct Usage {
    pub input: Option<f64>,
    #[napi(js_name = "cacheRead")]
    pub cache_read: Option<f64>,
    #[napi(js_name = "cacheWrite5m")]
    pub cache_write5m: Option<f64>,
    #[napi(js_name = "cacheWrite1h")]
    pub cache_write1h: Option<f64>,
    pub output: Option<f64>,
}

#[napi(object)]
pub struct CostParts {
    pub input: f64,
    #[napi(js_name = "cacheRead")]
    pub cache_read: f64,
    #[napi(js_name = "cacheWrite")]
    pub cache_write: f64,
    pub output: f64,
    pub total: f64,
}

#[napi(js_name = "costParts")]
pub fn cost_parts(model: String, at: Option<f64>, u: Usage, fast: bool, priority: bool) -> Option<CostParts> {
    let tokens = pricing::Tokens { input: u.input, cache_read: u.cache_read, cache_write5m: u.cache_write5m, cache_write1h: u.cache_write1h, output: u.output };
    pricing::cost_parts(&model, at, &tokens, fast, priority).map(|c| CostParts {
        input: c.input,
        cache_read: c.cache_read,
        cache_write: c.cache_write,
        output: c.output,
        total: c.total,
    })
}

// ----- time -----

#[napi(js_name = "parseIso")]
pub fn parse_iso(text: String) -> f64 {
    time::parse_iso(&text).unwrap_or(f64::NAN)
}

// ----- paths -----

#[napi(js_name = "pathKey")]
pub fn path_key(p: String) -> Option<String> {
    paths::path_key(&p)
}

#[napi(js_name = "isUnder")]
pub fn is_under(child: String, parent: String) -> bool {
    paths::is_under(&child, &parent)
}

#[napi]
pub fn tilde(text: String, home: String) -> String {
    paths::tilde(&text, &home)
}

#[napi(js_name = "baseName")]
pub fn base_name(p: String) -> String {
    paths::base_name(&p)
}

/// "win32", "posix" or "native": which of Node's path modules a path keeps
/// its syntax in.
#[napi(js_name = "pathStyle")]
pub fn path_style(p: String) -> &'static str {
    match paths::path_style(&p) {
        paths::PathStyle::Windows => "win32",
        paths::PathStyle::Posix => "posix",
        paths::PathStyle::Native => "native",
    }
}

#[napi(object)]
pub struct ZoneInfo {
    pub id: String,
    pub name: String,
    pub app: Option<String>,
}

#[napi(js_name = "scratchZones")]
pub fn scratch_zones() -> Vec<ZoneInfo> {
    paths::SCRATCH_ZONES.iter().map(|z| ZoneInfo { id: z.id.into(), name: z.name.into(), app: z.app.map(Into::into) }).collect()
}

/// The index of the zone a key belongs to, or null.
#[napi(js_name = "scratchZone")]
pub fn scratch_zone(key: String) -> Option<u32> {
    let z = paths::scratch_zone(&key)?;
    paths::SCRATCH_ZONES.iter().position(|s| s == z).map(|i| i as u32)
}

/// Whether a key falls in zone `i`, whatever the zones before it say.
#[napi(js_name = "inScratchZone")]
pub fn in_scratch_zone(i: u32, key: String) -> bool {
    paths::in_scratch_zone(i as usize, &key)
}

/// The first `cmd` on `path_var` that `can_run` accepts (it throws for a
/// file it can't run), or that this process may run when there's none.
#[napi(js_name = "onPath")]
pub fn on_path(cmd: String, path_var: String, pathext: Option<String>, windows: bool, can_run: Option<Function<String, Unknown>>) -> Option<String> {
    let mut check = |file: &str| match &can_run {
        Some(f) => f.call(file.to_string()).is_ok(),
        None => paths::can_run(file),
    };
    paths::on_path(&cmd, &path_var, pathext.as_deref(), windows, &mut check)
}

#[napi(js_name = "listFiles")]
pub fn list_files(dir: String, pick: Function<String, bool>, depth: u32) -> Result<Vec<String>> {
    // Once `pick` throws, JS would read no further: it isn't called again,
    // and its error is thrown.
    let mut failed = None;
    let mut keep = |name: &str| {
        if failed.is_some() {
            return false;
        }
        match pick.call(name.to_string()) {
            Ok(v) => v,
            Err(e) => {
                failed = Some(e);
                false
            }
        }
    };
    let out = paths::list_files(&dir, &mut keep, depth);
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}
