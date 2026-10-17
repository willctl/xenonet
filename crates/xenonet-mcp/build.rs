// XenoNet's version is package.json's, which the release keeps in step with
// the notes and the lock (docs/guide/releases.md): the MCP servers give it
// as theirs, as the JS servers do.
fn main() {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../package.json");
    println!("cargo:rerun-if-changed={file}");
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let version = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("\"version\":"))
        .and_then(|v| v.trim().trim_end_matches(',').trim().strip_prefix('"').and_then(|v| v.strip_suffix('"')).map(str::to_string))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=XENONET_VERSION={version}");
}
