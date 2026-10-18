//! xenonet setup (cli/commands/setup.js): a downloaded component installs
//! itself, from its extracted folder (core/components/install.js).

use crate::args::Flags;
use crate::cores;
use crate::js::t;
use crate::out::{print, Done, Fail};
use crate::sys;
use serde_json::Value;

pub const HELP: &str = include_str!("help/setup.txt");

pub fn run(_args: &[String], _flags: &Flags) -> Done {
    // The component's folder: the one its component.json is in, above the
    // binary.
    let root = sys::xenonet_root();
    let file = sys::join(&root, "component.json");
    let text = std::fs::read_to_string(&file).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &file).message))?;
    let manifest = xenonet_base::json::parse(&text).map_err(|e| Fail::error(e.to_string()))?;
    let dir = sys::join(&cores::runtime_dir()?, "downloads");
    std::fs::create_dir_all(&dir).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "mkdir", &dir).message))?;
    let package = sys::join(&dir, &format!("setup-{}-{}-{}.tar.gz", t(&manifest, "component"), t(&manifest, "version"), std::process::id()));
    let entries: Vec<String> = xenonet_base::paths::read_dir_sorted(std::path::Path::new(&root))
        .map_err(|e| Fail::error(e.to_string()))?
        .iter()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    cores::tar_create(&package, &root, &entries)?;
    let bytes = std::fs::read(&package).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &package).message))?;
    let mut item = manifest.as_object().cloned().unwrap_or_default();
    item.insert("size".into(), Value::from(bytes.len()));
    item.insert("sha256".into(), Value::String(cores::digest(&bytes)?));
    let result = cores::activate_package(&package, &Value::Object(item))?;
    if let Some(bin) = cores::install_launcher()?.and_then(|l| l["bin"].as_str().map(str::to_string)) {
        cores::install_command(&serde_json::json!({ "bin": bin }))?;
    }
    print(xenonet_base::json::stringify(&result, 0));
    Ok(0)
}
