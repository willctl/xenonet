#![no_main]

// Records checked against XenoNet's own schemas (schemas/), as the service
// checks what other programs write: leniently against each vendor schema
// (what drifts is recorded, the record stripped or dropped), and strictly,
// as zod does, against the documents the doctor checks state.json with.
// The first byte picks the schema; the second, the version a record says
// it was written by and whether the strict check is strict.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;
use xenonet_schema::check::{self, Checker, Data};
use xenonet_schema::drift::DriftLog;
use xenonet_schema::lenient;
use xenonet_schema::vendor::{self, VendorReader};

struct Schemas {
    dir: PathBuf,
    vendor: Vec<(String, Vec<lenient::Major>)>,
    checker: Checker,
    roots: Vec<usize>,
}

fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(ents) = xenonet_base::paths::read_dir_sorted(dir) else { return };
    for e in ents {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

fn schemas() -> &'static Schemas {
    static S: OnceLock<Schemas> = OnceLock::new();
    S.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas");
        let vendor = lenient::vendor_sources(&dir).into_iter().map(|s| (s.clone(), lenient::load_vendor(&s, &dir))).collect();
        // XenoNet's own documents refer to each other by $id, so they're
        // compiled together.
        let mut files = Vec::new();
        walk(&dir, &mut files);
        let docs: Vec<Value> =
            files.iter().filter(|p| !p.starts_with(dir.join("vendor"))).filter_map(|p| xenonet_base::json::parse(&std::fs::read_to_string(p).ok()?).ok()).collect();
        let checker = check::compile(&docs).expect("XenoNet's schemas compile");
        let roots = (0..docs.len()).map(|i| checker.root(i)).collect();
        Schemas { dir, vendor, checker, roots }
    })
}

const VERSIONS: [Option<&str>; 6] = [None, Some("2.1.280"), Some("0.40.0"), Some("99.0.0-beta.1"), Some("x"), Some("")];

fuzz_target!(|data: &[u8]| {
    let [pick, how, body @ ..] = data else { return };
    let Some(record) = xenonet_fuzz::json(body) else { return };
    let s = schemas();
    xenonet_fuzz::run(body, || {
        let n = usize::from(*pick);
        if n % 2 == 0 {
            let (source, majors) = &s.vendor[(n / 2) % s.vendor.len()];
            let mut log = DriftLog::default();
            let mut r = VendorReader::new(source, s.dir.clone(), Some(majors.clone()));
            let version = VERSIONS[usize::from(*how) % VERSIONS.len()];
            let kind = record.get("type").and_then(Value::as_str).map(str::to_string);
            for _ in 0..2 {
                // The second time, what drifted is known.
                let _ = r.inspect(&record, version, kind.clone(), &mut log, 1_791_230_000_000.0);
            }
            let _ = r.sample(&record, version, kind, 1, &mut log, 1_791_230_000_000.0);
            for m in majors {
                let found = m.schema.inspect(&record);
                let _ = lenient::strip(&record, &found.strip);
            }
            let _ = log.list();
            if let Some(v) = version {
                let _ = vendor::parse_version(v);
            }
        } else {
            let root = s.roots[(n / 2) % s.roots.len()];
            let checked = s.checker.check(root, &Data::from(&record), how & 1 != 0);
            let issues: Vec<(String, String)> = checked.issues.iter().map(|f| (f.path.clone(), f.message.clone())).collect();
            let _ = check::explain(&issues, &checked.undeclared);
        }
    });
});
