// Shared by the crates' build scripts (include!): on Windows, the program's icon, its manifest (DPI
// awareness, long paths, themed controls) and version information. Windows shows FileDescription as
// the program's name in Task Manager and in Settings > Other system tray icons.
fn windows_resources(internal: &str) {
    println!("cargo:rerun-if-changed=../../res/xenolith.ico");
    println!("cargo:rerun-if-changed=../../res/xenolith.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let res = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("..").join("..").join("res");
    let esc = |p: std::path::PathBuf| p.to_string_lossy().replace('\\', "/");
    let v = std::env::var("CARGO_PKG_VERSION").unwrap();
    let n: Vec<&str> = v.split('.').chain(["0", "0", "0"]).take(4).collect();
    let nums = n.join(",");
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {nums}
PRODUCTVERSION {nums}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "willctl"
      VALUE "FileDescription", "Xenolith"
      VALUE "FileVersion", "{v}"
      VALUE "InternalName", "{internal}"
      VALUE "OriginalFilename", "{internal}.exe"
      VALUE "ProductName", "Xenolith"
      VALUE "ProductVersion", "{v}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = esc(res.join("xenolith.ico")),
        manifest = esc(res.join("xenolith.manifest")),
    );
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("xenolith.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_required().unwrap();
}
