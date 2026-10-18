//! core/themefiles.js: your own themes, JSON files in the themes folder of
//! XenoNet's state folder, one a theme (<id>.json), with the fields of a
//! theme and "extends" naming the theme they start from. A file named for
//! a built-in takes its place. Read fresh each time, so an edit shows on
//! the next look; a file that isn't a JSON object is skipped and reported.

use crate::fsx::{self, FsError};
use crate::nodepath::NATIVE;
use crate::themes::{themes, DEFAULT_THEME};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

fn id_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,40}$").expect("a fixed pattern"))
}

/// The themes, yours folded on top of the built-ins, and what was wrong
/// with the files that couldn't be used.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
pub fn read_themes(dir: &str) -> (Map<String, Value>, Vec<String>) {
    let mut custom = Map::new();
    let mut problems = Vec::new();
    let files: Vec<String> = fsx::read_dir_names(dir).map(|n| n.into_iter().filter(|f| f.ends_with(".json")).collect()).unwrap_or_default();
    for f in files {
        let id = f[..f.len() - 5].to_lowercase();
        if !id_re().is_match(&id) {
            problems.push(format!("{f}: a theme's file name is its id: lowercase letters, digits and dashes"));
            continue;
        }
        let parsed = fsx::read_text(&NATIVE.join(&[dir, &f])).map_err(|e| e.message).and_then(|text| xenonet_base::json::parse(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(t @ Value::Object(_)) => {
                custom.insert(id, t);
            }
            Ok(_) => problems.push(format!("{f}: a theme is a JSON object of fields")),
            Err(why) => problems.push(format!("{f}: {why}")),
        }
    }
    (themes(&custom), problems)
}

/// A theme of your own to start from: `from` with nothing changed yet,
/// saved as <id>.json. Returns its file.
pub fn copy_theme(dir: &str, from: Option<&str>, id: &str, name: Option<&Value>) -> Result<String, FsError> {
    if !id_re().is_match(id) {
        return Err(fsx::said("a theme's id is lowercase letters, digits and dashes"));
    }
    let file = NATIVE.join(&[dir, &format!("{id}.json")]);
    if fsx::exists(&file) {
        return Err(fsx::said(format!("there's already a theme called {id}")));
    }
    let (all, _) = read_themes(dir);
    let base = from.and_then(|f| all.get(f)).or_else(|| all.get(DEFAULT_THEME)).cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    out.insert("name".into(), name.filter(|n| !n.is_null()).cloned().unwrap_or_else(|| Value::String(id.into())));
    out.insert("about".into(), Value::String(format!("Starts from {}", base.get("name").map_or_else(|| "undefined".into(), xenonet_base::text::js_string))));
    let built_in = crate::jsv::truthy(base.get("builtIn"));
    match (built_in, from) {
        (true, Some(f)) => {
            out.insert("extends".into(), Value::String(f.into()));
        }
        (true, None) => {}
        (false, _) => {
            out.insert("extends".into(), Value::String(DEFAULT_THEME.into()));
        }
    }
    if let Value::Object(fields) = &base {
        for (k, v) in fields {
            if !["id", "builtIn", "replacesBuiltIn", "extends", "name", "about"].contains(&k.as_str()) {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    fsx::mkdir_p(dir, None)?;
    fsx::write_file(&file, format!("{}\n", xenonet_base::json::stringify(&Value::Object(out), 2)).as_bytes(), None)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn your_own_themes_and_copies() {
        let dir = std::env::temp_dir().join(format!("xenonet-themefiles-{}", std::process::id()));
        let d = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
        std::fs::write(dir.join("Bad Name.json"), "{}").unwrap();
        std::fs::write(dir.join("ember.json"), r##"{"extends":"helios","colors":{"accent":"#ff0000"}}"##).unwrap();
        let (all, problems) = read_themes(&d);
        assert_eq!(all["ember"]["colors"]["accent"], "#ff0000");
        assert_eq!(problems.len(), 2);
        let file = copy_theme(&d, Some("phosphor"), "mine", None).unwrap();
        let copy: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(copy["extends"], "phosphor");
        assert!(copy_theme(&d, Some("xenonet"), "mine", None).unwrap_err().message.contains("already a theme called mine"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
