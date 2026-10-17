// XenoNet's version is package.json's, which the release keeps in step with
// the notes and the lock (docs/guide/releases.md): the binary says the same.
// The Tools package's package.json also says it's the Tools'
// (xenonetComponent), whose xenonet console drives installed XenoNet.
fn main() {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../package.json");
    println!("cargo:rerun-if-changed={file}");
    let text = std::fs::read_to_string(file).unwrap_or_default();
    // A top-level "key": "value" line of package.json.
    let field = |key: &str| {
        let prefix = format!("\"{key}\":");
        text.lines()
            .find_map(|l| l.strip_prefix("  ").filter(|l| !l.starts_with(' ')).and_then(|l| l.strip_prefix(prefix.as_str())))
            .and_then(|v| v.trim().trim_end_matches(',').trim().strip_prefix('"').and_then(|v| v.strip_suffix('"')).map(str::to_string))
    };
    let version = field("version").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=XENONET_VERSION={version}");
    println!("cargo:rustc-env=XENONET_COMPONENT={}", field("xenonetComponent").unwrap_or_default());
}
