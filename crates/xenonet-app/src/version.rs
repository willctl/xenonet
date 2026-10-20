//! core/version.js: XenoNet's version, from its package.json, for what it
//! tells the programs and services it talks to.

use serde_json::Value;

/// The `version` field of the package.json at `file` as JSON.parse reads it
/// (None when the field isn't there, as JS reads it as undefined), or
/// "dev" when the file can't be read or isn't JSON.
pub fn version_of(file: &std::path::Path) -> Option<Value> {
    let Ok(text) = std::fs::read_to_string(file) else { return Some(Value::String("dev".into())) };
    match xenonet_base::json::parse(&text) {
        Ok(Value::Object(m)) => m.get("version").cloned(),
        // JSON.parse("null").version throws, which the JS catches.
        Ok(Value::Null) => Some(Value::String("dev".into())),
        Ok(_) => None,
        Err(_) => Some(Value::String("dev".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_or_says_dev() {
        let dir = std::env::temp_dir().join(format!("xenonet-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("package.json");
        std::fs::write(&f, r#"{"name":"x","version":"1.2.3"}"#).unwrap();
        assert_eq!(version_of(&f), Some(Value::String("1.2.3".into())));
        std::fs::write(&f, "{not json").unwrap();
        assert_eq!(version_of(&f), Some(Value::String("dev".into())));
        std::fs::write(&f, r#"{"name":"x"}"#).unwrap();
        assert_eq!(version_of(&f), None);
        assert_eq!(version_of(&dir.join("missing.json")), Some(Value::String("dev".into())));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
