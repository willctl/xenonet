//! Why something XenoNet did, or asked of the machine, didn't give its
//! value: the error's message, and the other fields its callers read (a
//! code such as ENOENT, an HTTP status, a program's stderr, the name of
//! the kind of error). It travels as it was made, so whoever reports it
//! words it the same wherever it came from.

use crate::fs::FsError;
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub message: String,
    /// The error's other fields, in the order they were given.
    pub fields: Map<String, Value>,
}

impl Failure {
    pub fn msg(message: impl Into<String>) -> Self {
        Failure { message: message.into(), fields: Map::new() }
    }

    pub fn coded(message: impl Into<String>, code: &str) -> Self {
        let mut fields = Map::new();
        fields.insert("code".into(), Value::String(code.into()));
        Failure { message: message.into(), fields }
    }

    pub fn with(message: impl Into<String>, fields: Map<String, Value>) -> Self {
        Failure { message: message.into(), fields }
    }

    /// A value of the wrong kind, named TypeError.
    pub fn type_error(message: impl Into<String>) -> Self {
        let mut fields = Map::new();
        fields.insert("name".into(), Value::String("TypeError".into()));
        Failure { message: message.into(), fields }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// One of its other fields, null read as missing.
    pub fn field(&self, k: &str) -> Option<&Value> {
        self.fields.get(k).filter(|v| !v.is_null())
    }

    /// Its code, when it's a string.
    pub fn code(&self) -> Option<&str> {
        self.fields.get("code").and_then(Value::as_str)
    }

    /// The error as a JSON object: its message first, then its other fields.
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("message".into(), Value::String(self.message.clone()));
        for (k, v) in &self.fields {
            m.insert(k.clone(), v.clone());
        }
        Value::Object(m)
    }

    /// An error given as a JSON object: its message (or, without one, the
    /// value as text), and the rest as its fields.
    pub fn from_value(v: &Value) -> Self {
        let mut fields = v.as_object().cloned().unwrap_or_default();
        let message = match fields.shift_remove("message") {
            Some(Value::String(s)) => s,
            Some(m) => xenonet_text::text::display(&m),
            None if v.is_object() => xenonet_text::text::display(v),
            None => {
                fields.clear();
                xenonet_text::text::display(v)
            }
        };
        Failure { message, fields }
    }
}

impl From<FsError> for Failure {
    fn from(e: FsError) -> Self {
        match e.code {
            Some(c) => Failure::coded(e.message, &c),
            None => Failure::msg(e.message),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn its_message_first_then_its_fields() {
        let f = Failure::coded("gone", "ENOENT");
        assert_eq!(f.to_value(), json!({ "message": "gone", "code": "ENOENT" }));
        assert_eq!(Failure::from_value(&f.to_value()), f);
        assert_eq!(Failure::type_error("no").to_value(), json!({ "message": "no", "name": "TypeError" }));
        assert_eq!(Failure::from_value(&json!("plain")).message, "plain");
        assert_eq!(Failure::from(FsError { code: Some("EACCES".into()), message: "denied".into() }).code(), Some("EACCES"));
    }
}
