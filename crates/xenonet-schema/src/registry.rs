//! XenoNet's schemas by id (core/schema/registry.js): the ids defined, the
//! lookups, and the JSON Schema documents npm run schemas writes, in the
//! order the files have their keys. The schemas themselves are zod's, in
//! JS, and so is z.toJSONSchema, which writes what this puts in order.

use crate::check::{self, Checked, Checker, Data};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use xenonet_base::{ids, json, jsstr};

pub const COMMENT: &str = "Written by npm run schemas from core/schema. Change the definition there, not this file.";

/// The ids defined, in the order they were, and the documents last
/// written for them.
#[derive(Debug, Default)]
pub struct Registry {
    ids: Vec<String>,
    known: HashSet<String>,
    docs: Vec<(String, Value)>,
    compiled: RefCell<Option<(Checker, HashMap<String, usize>)>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Why `id` can't be defined, if it can't: it's defined already, or it
    /// has no title or no description.
    pub fn refusal(&self, id: &str, has_title: bool, has_description: bool) -> Option<String> {
        if self.known.contains(id) {
            return Some(format!("{id} is defined twice"));
        }
        if !has_title || !has_description {
            return Some(format!("{id} needs a title and a description"));
        }
        None
    }

    pub fn add(&mut self, id: &str) {
        if self.known.insert(id.to_string()) {
            self.ids.push(id.to_string());
        }
    }

    /// The short id of a defined schema, from either form of its id.
    pub fn find(&self, id: &str) -> Option<String> {
        let short = ids::parse_schema_id(id)?.id;
        self.known.contains(&short).then_some(short)
    }

    /// Every id, sorted as JS's sort() sorts.
    pub fn ids(&self) -> Vec<String> {
        let mut out = self.ids.clone();
        out.sort_by(|a, b| jsstr::cmp_units(a, b));
        out
    }

    /// Takes what z.toJSONSchema wrote for the registry (its `schemas`, by
    /// id), as the documents: sorted by id, each with its $schema, $id, the
    /// comment, its title and description first.
    pub fn set_documents(&mut self, schemas: &Map<String, Value>) {
        let mut keys: Vec<&String> = schemas.keys().collect();
        keys.sort_by(|a, b| jsstr::cmp_units(a, b));
        self.docs = keys
            .into_iter()
            .map(|id| {
                let mut doc = Map::new();
                let empty = Map::new();
                let s = schemas[id].as_object().unwrap_or(&empty);
                for k in ["$schema", "$id"] {
                    if let Some(v) = s.get(k) {
                        doc.insert(k.into(), v.clone());
                    }
                }
                doc.insert("$comment".into(), Value::String(COMMENT.into()));
                for k in ["title", "description"] {
                    if let Some(v) = s.get(k) {
                        doc.insert(k.into(), v.clone());
                    }
                }
                for (k, v) in s {
                    if !matches!(k.as_str(), "$schema" | "$id" | "$comment" | "title" | "description") {
                        doc.insert(k.clone(), v.clone());
                    }
                }
                (id.clone(), Value::Object(doc))
            })
            .collect();
        self.compiled.replace(None);
    }

    /// The documents, by id.
    pub fn documents(&self) -> Map<String, Value> {
        self.docs.iter().cloned().collect()
    }

    /// The files npm run schemas writes: a path under schemas/, and its text.
    pub fn files(&self) -> Vec<(String, String)> {
        self.docs.iter().map(|(id, doc)| (format!("{id}.json"), format!("{}\n", json::stringify(doc, 2)))).collect()
    }

    /// check() against the documents, by short id: None when there's no
    /// such document; an error when one can't be checked against.
    pub fn check(&self, id: &str, data: &Data, strict: bool) -> Result<Option<Checked>, String> {
        if self.compiled.borrow().is_none() {
            let docs: Vec<Value> = self.docs.iter().map(|(_, d)| d.clone()).collect();
            let checker = check::compile(&docs)?;
            let roots = self.docs.iter().enumerate().map(|(i, (id, _))| (id.clone(), checker.root(i))).collect();
            self.compiled.replace(Some((checker, roots)));
        }
        let compiled = self.compiled.borrow();
        let Some((checker, roots)) = compiled.as_ref() else { return Ok(None) };
        Ok(roots.get(id).map(|&root| checker.check(root, data, strict)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defines_finds_and_writes() {
        let mut r = Registry::new();
        assert_eq!(r.refusal("test/leaf/v1", true, true), None);
        r.add("test/leaf/v1");
        r.add("test/a/v1");
        assert_eq!(r.refusal("test/leaf/v1", true, true).as_deref(), Some("test/leaf/v1 is defined twice"));
        assert_eq!(r.refusal("test/bare/v1", true, false).as_deref(), Some("test/bare/v1 needs a title and a description"));
        assert_eq!(r.find("https://xenonet.dev/schemas/test/leaf/v1.json").as_deref(), Some("test/leaf/v1"));
        assert_eq!(r.find("test/leaf/v2"), None);
        assert_eq!(r.ids(), ["test/a/v1", "test/leaf/v1"]);
        let written = json!({
            "test/leaf/v1": { "type": "object", "properties": { "at": { "type": "number" } }, "required": ["at"], "description": "A leaf", "$schema": "https://json-schema.org/draft/2020-12/schema", "title": "Leaf", "$id": "https://xenonet.dev/schemas/test/leaf/v1.json", "additionalProperties": {} },
            "test/a/v1": { "$id": "https://xenonet.dev/schemas/test/a/v1.json", "title": "A", "description": "An a", "type": "object", "properties": { "leaf": { "$ref": "https://xenonet.dev/schemas/test/leaf/v1.json" } }, "required": ["leaf"], "additionalProperties": {} }
        });
        r.set_documents(written.as_object().unwrap());
        let docs = r.documents();
        assert_eq!(docs.keys().collect::<Vec<_>>(), ["test/a/v1", "test/leaf/v1"]);
        assert_eq!(docs["test/leaf/v1"].as_object().unwrap().keys().take(5).collect::<Vec<_>>(), ["$schema", "$id", "$comment", "title", "description"]);
        assert!(r.files()[1].1.starts_with("{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n"));
        let bad = r.check("test/a/v1", &Data::from(&json!({ "leaf": { "at": "x" } })), false).unwrap().unwrap();
        assert_eq!(bad.issues[0].path, "leaf.at");
        assert!(r.check("test/a/v1", &Data::from(&json!({ "leaf": { "at": 1 } })), false).unwrap().unwrap().ok());
    }
}
