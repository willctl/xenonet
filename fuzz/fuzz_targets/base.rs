#![no_main]

// The text, OS, model and adapter helpers over text another program
// wrote: JS's string functions, apply_patch texts, times, paths (XenoNet's
// and Node's, for both platforms, and the adapters' scratch zones), ids
// and git remotes, model names and prices, the log's scrubbing, and the
// numbers the page shows. The first byte picks a group; NULs split the
// rest into arguments.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_adapter::{apps, formats::patch, platform};
use xenonet_model::usage::{Speed, TokenUsage};
use xenonet_model::{ids, usage::pricing};
use xenonet_os::nodepath::Style;
use xenonet_os::{log, paths};
use xenonet_text::{text, time, units, utf16};

fn num(s: &str) -> f64 {
    text::to_number(Some(&Value::String(s.to_string())))
}

// A count from text, kept to what XenoNet asks for (a clip of a few
// hundred, a page of a few dozen), so a huge one isn't a slow run.
fn small(s: &str) -> usize {
    s.bytes().take(3).fold(0usize, |n, b| n * 7 + usize::from(b)) % 400
}

fuzz_target!(|data: &[u8]| {
    xenonet_fuzz::adapters();
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
            let _ = (utf16::len(a), utf16::head(a, n), utf16::last(a, n), utf16::slice(a, start, Some(end)), utf16::slice(a, start, None));
            let _ = (utf16::substring(a, start, Some(end)), utf16::pad_start(a, n, c), utf16::cmp_units(a, b), utf16::collapse_space(a));
            let _ = (utf16::is_array_index(a), utf16::locale_compare(a, b), utf16::locale_compare_num(a, b), utf16::hex(a.as_bytes()));
            let _ = (text::trim(a), text::trim_start(a), text::is_blank(a), num(a), patch::headers(a));
            if let Some(Value::Object(m)) = xenonet_fuzz::json(a.as_bytes()) {
                let _ = utf16::key_order(m);
            }
            if let Some(v) = xenonet_fuzz::json(a.as_bytes()) {
                let _ = (text::display(&v), text::truthy(Some(&v)), text::type_of(Some(&v)), text::to_number(Some(&v)));
                if let Value::Array(items) = &v {
                    let _ = text::join(items.iter().map(Some), b);
                }
            }
        }
        1 => {
            if let Some(ms) = time::parse_iso(a) {
                let _ = time::iso(ms);
                let _ = (utf16::local_day(ms), utf16::local_day_bounds(ms));
                let days = (ms / 86_400_000.0).floor() as i64;
                let (y, m, d) = time::civil_from_days(days);
                assert_eq!(time::days_from_civil(y, m, d), days);
            }
            let _ = time::iso(num(a));
            let _ = (utf16::local_day(num(a)), utf16::local_offset_ms(num(a)));
        }
        2 => {
            let _ = (paths::path_key(a), paths::is_under(a, b), paths::tilde(a, b), paths::base_name(a), paths::path_style(a));
            let _ = (paths::join_all(&[a, b, c]), paths::join(a, b), paths::normalize_posix(a));
            // The throwaway folders' zones: each adapter's, then the temp
            // folders'.
            for key in [Some(a.to_string()), paths::path_key(a)].into_iter().flatten() {
                let _ = (platform::scratch_zone(&key), paths::temp_zone(&key));
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
            let _ = (ids::normalize_remote(a), ids::RepoId::of_remote(a), ids::remote_parts(a), ids::RepoId::parse(a));
            let _ = (ids::is_schema_path(a), ids::SchemaId::parse(a).map(|s| (s.short(), s.uri())));
            let _ = ids::is_host_id(a);
            let _ = (apps::is_session_id(a), apps::app_of_session(a), ids::is_actor_slug(a), ids::parse_actor_id(a));
        }
        5 => {
            let at = Some(1_791_230_000_000.0);
            let speed = Speed { fast: pick & 8 != 0, priority: pick & 16 != 0 };
            let u = TokenUsage { input: Some(num(b)), output: Some(num(c)), cache_read: Some(1000.0), cache_write_5m: None, cache_write_1h: Some(5.0), speed };
            let _ = (pricing::plain_model(a), pricing::price_of(a, at), pricing::priority_tier(b));
            let _ = (pricing::cost_parts(a, at, &u), pricing::cost_of(a, at, &TokenUsage { speed: Speed::STANDARD, ..u }));
        }
        6 => {
            let _ = (log::scrub(a, b), log::unsafe_text(a, b), utf16::whole_head(a, small(b)));
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
            let _ = (units::round(x), units::to_fixed(x, n), units::locale(x, 0, n), units::money(Some(x)), units::tokens(Some(x)));
            let _ = (units::duration(Some(x)), units::plural(x, b, Some(c)), units::share(x, num(c)), units::share_of(x), units::excerpt(a, small(c)));
        }
    }
});
