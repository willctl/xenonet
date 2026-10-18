//! Themes (core/themefiles.js, crates/xenonet-app/src/themefiles.rs): your
//! own, JSON files in the state folder's themes/, read fresh each time; and
//! that folder's watch, as server/index.js keeps it. A change there reaches
//! every page at once: the theme in use, which a page draws again only if
//! it changed, and that the list changed.

use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};
use xenonet_service::cores::{CoreError, Themes, R};
use xenonet_service::wire::Emitter;

pub struct ThemeFiles;

impl Themes for ThemeFiles {
    fn read(&self, dir: &str) -> R<Value> {
        let (themes, problems) = xenonet_app::themefiles::read_themes(dir);
        Ok(json!({ "themes": themes, "problems": problems }))
    }

    fn copy(&self, dir: &str, from: &str, id: &str, name: &Value) -> R<Value> {
        let name = (!name.is_null()).then_some(name);
        xenonet_app::themefiles::copy_theme(dir, Some(from), id, name).map(Value::String).map_err(|e| CoreError::new(e.message))
    }
}

// What the folder holds: each file's name, size and time, in order.
fn signature(dir: &Path) -> Vec<(String, u64, u128)> {
    let mut seen: Vec<(String, u64, u128)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| {
                    let meta = e.metadata().ok();
                    let size = meta.as_ref().map_or(0, |m| m.len());
                    let at = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                    (e.file_name().to_string_lossy().into_owned(), size, at)
                })
                .collect()
        })
        .unwrap_or_default();
    seen.sort();
    seen
}

/// fs.watch on the themes folder, as a look every half second: no
/// dependency for it, and a theme's edit is a person's, not a stream. A
/// change goes out 150 ms after the last one seen, as the JS's debounce.
pub fn watch(dir: String, emitter: Emitter) {
    let folder = Path::new(&dir).to_path_buf();
    if std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut last = signature(&folder);
        let mut every = tokio::time::interval(Duration::from_millis(500));
        loop {
            every.tick().await;
            let now = signature(&folder);
            if now == last {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
            last = signature(&folder);
            let Some(ctx) = emitter.ctx() else { continue };
            if let Ok(theme) = xenonet_service::routes::current_theme(&ctx) {
                ctx.broadcast("theme", &theme);
            }
            ctx.broadcast("themes", &json!({}));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_copies_into_the_folder_given() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-themes-{}", std::process::id()));
        let d = dir.to_string_lossy().into_owned();
        let t = ThemeFiles;
        let read = t.read(&d).unwrap();
        assert!(read["themes"]["xenonet"]["colors"].is_object());
        let file = t.copy(&d, "rime", "frost", &json!("Frost")).unwrap();
        assert!(file.as_str().unwrap().ends_with("frost.json"));
        assert_eq!(t.read(&d).unwrap()["themes"]["frost"]["name"], "Frost");
        assert!(t.copy(&d, "rime", "frost", &Value::Null).is_err(), "once");
        std::fs::write(dir.join("Bad Name.json"), "{}").unwrap();
        assert_eq!(t.read(&d).unwrap()["problems"].as_array().unwrap().len(), 1);
        assert_ne!(signature(&dir), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
