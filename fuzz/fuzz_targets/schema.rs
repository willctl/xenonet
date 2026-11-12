#![no_main]

// Records checked against XenoNet's own schemas (schemas/), as the service
// checks what other programs write: leniently against each vendor schema
// (what drifts is recorded, the record stripped or dropped), and as
// XenoNet's own documents, read as their types (what other hosts and
// actors send). The first byte picks the schema; the second, the version
// a record says it was written by.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;
use xenonet_model::{actor, graph, system};
use xenonet_schema::drift::DriftLog;
use xenonet_schema::lenient;
use xenonet_schema::typed;
use xenonet_schema::vendor::{self, VendorReader};

struct Schemas {
    dir: PathBuf,
    vendor: Vec<(String, Vec<lenient::Major>)>,
}

fn schemas() -> &'static Schemas {
    static S: OnceLock<Schemas> = OnceLock::new();
    S.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../schemas");
        let vendor = lenient::vendor_sources(&dir).into_iter().map(|s| (s.clone(), lenient::load_vendor(&s, &dir))).collect();
        Schemas { dir, vendor }
    })
}

/// XenoNet's own documents a record is read as: each gives the first
/// problem, worded, or the record cleaned to what its type declares.
const OWN: [fn(&Value) -> Option<String>; 5] = [
    |v| typed::clean::<graph::SessionNode>(v).map(|c| c.to_string()),
    |v| typed::clean::<graph::ProjectNode>(v).map(|c| c.to_string()),
    |v| typed::clean::<system::System>(v).map(|c| c.to_string()),
    |v| typed::read::<actor::Report>(v).err().map(|i| i.worded("the body")),
    |v| typed::read::<actor::Registration>(v).err().map(|i| i.worded("the body")),
];

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
            let _ = OWN[(n / 2) % OWN.len()](&record);
        }
    });
});
