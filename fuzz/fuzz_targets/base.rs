#![no_main]

// xenonet-base's helpers over text another program wrote: JS's string
// functions, apply_patch texts, times, paths (XenoNet's and Node's, for both
// platforms), ids and git remotes, model names and prices, the log's
// scrubbing, and the numbers the page shows. The first byte picks a group;
// NULs split the rest into arguments.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_base::nodepath::Style;
use xenonet_base::{ids, jsstr, log, patch, paths, pricing, text, time, units};

fn num(s: &str) -> f64 {
    text::js_number(Some(&Value::String(s.to_string())))
}

// A count from text, kept to what XenoNet asks for (a clip of a few
// hundred, a page of a few dozen), so a huge one isn't a slow run.
fn small(s: &str) -> usize {
    s.bytes().take(3).fold(0usize, |n, b| n * 7 + usize::from(b)) % 400
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let input = xenonet_fuzz::text(rest);
    let mut args = input.split('\0');
    let a = args.next().unwrap_or_default();
    let b = args.next().unwrap_or_default();
    let c = args.next().unwrap_or_default();
    match pick % 8 {
        0 => {
            let n = small(b);
            let (start, end) = (num(b) as i64, num(c) as i64);
            let _ = (jsstr::len(a), jsstr::head(a, n), jsstr::last(a, n), jsstr::slice(a, start, Some(end)), jsstr::slice(a, start, None));
            let _ = (jsstr::substring(a, start, Some(end)), jsstr::pad_start(a, n, c), jsstr::cmp_units(a, b), jsstr::collapse_space(a));
            let _ = (jsstr::is_array_index(a), jsstr::locale_compare(a, b), jsstr::locale_compare_num(a, b), jsstr::hex(a.as_bytes()));
            let _ = (text::js_trim(a), text::js_trim_start(a), text::is_blank(a), num(a), patch::headers(a));
            if let Some(Value::Object(m)) = xenonet_fuzz::json(a.as_bytes()) {
                let _ = jsstr::js_key_order(m);
            }
            if let Some(v) = xenonet_fuzz::json(a.as_bytes()) {
                let _ = (text::js_string(&v), text::truthy(Some(&v)), text::type_of(Some(&v)), text::js_number(Some(&v)));
                if let Value::Array(items) = &v {
                    let _ = text::join(items.iter().map(Some), b);
                }
            }
        }
        1 => {
            if let Some(ms) = time::parse_iso(a) {
                let _ = time::iso(ms);
                let _ = (jsstr::local_day(ms), jsstr::local_day_bounds(ms));
                let days = (ms / 86_400_000.0).floor() as i64;
                let (y, m, d) = time::civil_from_days(days);
                assert_eq!(time::days_from_civil(y, m, d), days);
            }
            let _ = time::iso(num(a));
            let _ = (jsstr::local_day(num(a)), jsstr::local_offset_ms(num(a)));
        }
        2 => {
            let _ = (paths::path_key(a), paths::is_under(a, b), paths::tilde(a, b), paths::base_name(a), paths::path_style(a));
            let _ = (paths::join_all(&[a, b, c]), paths::join(a, b), paths::normalize_posix(a), paths::scratch_zone(a));
            if let Some(key) = paths::path_key(a) {
                let _ = paths::in_scratch_zone(small(b), &key);
            }
            let _ = paths::on_path(a, b, Some(c), pick & 8 != 0, &mut |f| f.len() % 3 == 0);
        }
        3 => {
            let style = Style::of(if pick & 8 != 0 { "win32" } else { "linux" });
            let _ = (style.join(&[a, b, c]), style.resolve_from(&[a, b], c), style.normalize(a), style.dirname(a));
            let _ = (style.basename(a, Some(b)), style.basename(a, None), style.extname(a), style.is_absolute(a));
            let _ = style.relative(a, b);
        }
        4 => {
            let _ = (ids::normalize_remote(a), ids::repo_id(a), ids::remote_parts(a), ids::parse_repo_id(a));
            let _ = (ids::is_schema_path(a), ids::short_id(a, num(b)), ids::parse_schema_id(a), ids::schema_uri(a));
            let _ = (ids::is_host_id(a), ids::parse_qualified(a), ids::qualify(a, Some(b), Some(c)), ids::qualify(a, None, None));
            let _ = (ids::is_session_id(a), ids::app_of_session(a), ids::is_actor_slug(a), ids::parse_actor_id(a));
        }
        5 => {
            let at = Some(1_791_230_000_000.0);
            let u = pricing::Tokens { input: Some(num(b)), output: Some(num(c)), cache_read: Some(1000.0), cache_write5m: None, cache_write1h: Some(5.0) };
            let _ = (pricing::plain_model(a), pricing::price_of(a, at), pricing::priority_tier(b));
            let _ = (pricing::cost_parts(a, at, &u, pick & 8 != 0, pick & 16 != 0), pricing::cost_of(a, at, &u, false, false));
        }
        6 => {
            let _ = (log::scrub(a, b), log::unsafe_text(a, b), log::js_slice(a, small(b)));
            if let Some(Value::Array(entries)) = xenonet_fuzz::json(a.as_bytes()) {
                let _ = log::summarize(&entries);
            }
            let mut l = log::Log::new(None, "fuzz", None, None, "/home/nova", 1 << 20);
            let ctx = xenonet_fuzz::json(c.as_bytes());
            let _ = l.write("error", "fuzz.code", Some(a), None, Some(b), ctx.as_ref(), 1_791_230_000_000.0);
        }
        _ => {
            let x = num(a);
            let n = small(b) % 21;
            let _ = (units::js_round(x), units::to_fixed(x, n), units::locale(x, 0, n), units::money(Some(x)), units::tokens(Some(x)));
            let _ = (units::duration(Some(x)), units::plural(x, b, Some(c)), units::share(x, num(c)), units::share_of(x), units::excerpt(a, small(c)));
        }
    }
});
