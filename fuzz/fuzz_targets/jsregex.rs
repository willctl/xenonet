#![no_main]

// JS's regular expressions, as the strict check runs them: every pattern
// in XenoNet's schemas against text another machine or program sent (of
// any length), and, so the compiler itself is tried, patterns from the
// input against a short text. The first byte picks which; a NUL splits a
// pattern from its text.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use xenonet_schema::jsregex::{self, Regex};

fn walk(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x) {
                    ("pattern", Value::String(p)) => out.push(p.clone()),
                    _ => walk(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(ents) = std::fs::read_dir(dir) else { return };
    for e in ents.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

fn patterns() -> &'static Vec<Regex> {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        let mut found = Vec::new();
        files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas"), &mut found);
        found.sort();
        let mut sources = Vec::new();
        for f in found {
            if let Some(v) = std::fs::read_to_string(&f).ok().and_then(|t| xenonet_base::json::parse(&t).ok()) {
                walk(&v, &mut sources);
            }
        }
        sources.sort();
        sources.dedup();
        sources.iter().map(|s| jsregex::compile(s).expect("XenoNet's patterns compile")).collect()
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    xenonet_fuzz::run(rest, || {
        if pick & 1 == 0 {
            let text = xenonet_fuzz::text(rest);
            let all = patterns();
            let _ = all[usize::from(pick / 2) % all.len()].test(&text);
        } else {
            let (pattern, text) = xenonet_fuzz::split(rest, 0);
            if let Ok(re) = jsregex::compile(&xenonet_fuzz::text(pattern)) {
                let text: String = xenonet_fuzz::text(text).chars().take(12).collect();
                let _ = re.test(&text);
            }
        }
    });
});
