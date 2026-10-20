//! The places around the map (core/environments.js), which the page imports
//! too, so the JS stays for it and the service has this copy.

use serde_json::{json, Value};

pub const DEFAULT_ENVIRONMENT: &str = "nebula";

pub fn environments() -> Value {
    json!([
        { "id": "nebula", "name": "Nebula", "about": "The original drifting clouds and star field.", "shape": "clouds", "index": 0 },
        { "id": "aurora", "name": "Aurora", "about": "Slow curtains of light across a quiet atmospheric horizon.", "shape": "curtains", "index": 1 },
        { "id": "trench", "name": "Trench", "about": "A deep-sea trench with rippling caustics and distant light shafts.", "shape": "water", "index": 2 },
        { "id": "vault", "name": "Vault", "about": "A monumental hall of repeating arches and illuminated ribs.", "shape": "arches", "index": 3 }
    ])
}

/// `environmentOf(id).id`: the one named, or the first.
pub fn environment_of(id: &Value) -> String {
    let all = environments();
    let list = all.as_array().expect("environments() is an array literal");
    list.iter().find(|e| &e["id"] == id).unwrap_or(&list[0])["id"].as_str().unwrap_or(DEFAULT_ENVIRONMENT).to_string()
}

pub fn is_environment(id: &Value) -> bool {
    environments().as_array().is_some_and(|l| l.iter().any(|e| &e["id"] == id))
}
