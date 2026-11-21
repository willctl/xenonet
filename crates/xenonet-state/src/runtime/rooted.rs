//! An app read from several folders: one lister for each root, answering as
//! one. A session from a root the person named carries it (`root`), so
//! what must run in that folder (a resume, its sign-in) can find it. The
//! app's default root's sessions carry nothing, as they never have.

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_adapter::reader::{BlockHealth, Failed, Lister, Renamed, VendorCheck};

pub struct Rooted {
    /// The default root's lister first, then those the person named, each
    /// with the root it reads.
    parts: Vec<(Option<String>, Arc<dyn Lister>)>,
}

impl Rooted {
    pub fn new(parts: Vec<(Option<String>, Arc<dyn Lister>)>) -> Rooted {
        Rooted { parts }
    }
}

impl Lister for Rooted {
    fn sessions(&self, vendor: &dyn VendorCheck) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        let mut failed: Option<String> = None;
        let mut any = false;
        for (root, l) in &self.parts {
            match l.sessions(vendor) {
                Ok(list) => {
                    any = true;
                    for mut s in list {
                        if let Some(o) = s.as_object_mut() {
                            // The default folder's sessions carry none, whatever an
                            // adapter put there.
                            match root {
                                Some(root) => o.insert("root".into(), json!(root)),
                                None => o.remove("root"),
                            };
                        }
                        out.push(s);
                    }
                }
                Err(e) => {
                    failed.get_or_insert(e);
                }
            }
        }
        // A root that couldn't be read doesn't hide the others; with none
        // read, the first one's reason is the answer.
        match (any, failed) {
            (false, Some(e)) => Err(e),
            _ => Ok(out),
        }
    }

    fn transcript(&self, vendor: &dyn VendorCheck, raw_id: &str, cursor: Option<&str>, limit: f64, files: &dyn Fn() -> Vec<String>) -> Result<Value, String> {
        let mut first: Option<String> = None;
        for (_, l) in &self.parts {
            match l.transcript(vendor, raw_id, cursor, limit, files) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        Err(first.unwrap_or_else(|| "XenoNet reads no folder of this app's".into()))
    }

    fn rename(&self, vendor: &dyn VendorCheck, node: &Value, title: &str) -> Result<Renamed, String> {
        // The root the session is in, else the default's.
        let root = node.get("root").and_then(Value::as_str);
        match self.parts.iter().find(|(r, _)| r.as_deref() == root).or_else(|| self.parts.first()) {
            Some((_, l)) => l.rename(vendor, node, title),
            None => Err("XenoNet can't rename this app's sessions".into()),
        }
    }

    fn projects(&self, vendor: &dyn VendorCheck) -> Option<Vec<Value>> {
        let mut out: Vec<Value> = Vec::new();
        let mut any = false;
        for (_, l) in &self.parts {
            if let Some(list) = l.projects(vendor) {
                any = true;
                for p in list {
                    if !out.iter().any(|q| q.get("id") == p.get("id")) {
                        out.push(p);
                    }
                }
            }
        }
        any.then_some(out)
    }

    fn block(&self) -> Option<BlockHealth> {
        self.parts.first().and_then(|(_, l)| l.block())
    }

    fn request(&self, vendor: &dyn VendorCheck, method: &str, params: Value) -> Result<Value, Failed> {
        match self.parts.first() {
            Some((_, l)) => l.request(vendor, method, params),
            None => Err(Failed::said("XenoNet has no server of this app's to ask")),
        }
    }

    fn stop(&self) {
        for (_, l) in &self.parts {
            l.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::reader::Unchecked;

    struct Fixed(Result<Vec<Value>, String>);
    impl Lister for Fixed {
        fn sessions(&self, _: &dyn VendorCheck) -> Result<Vec<Value>, String> {
            self.0.clone()
        }
        fn transcript(&self, _: &dyn VendorCheck, raw: &str, _: Option<&str>, _: f64, _: &dyn Fn() -> Vec<String>) -> Result<Value, String> {
            self.0.clone().map(|l| json!({ "entries": l, "raw": raw }))
        }
    }

    fn two(a: Result<Vec<Value>, String>, b: Result<Vec<Value>, String>) -> Rooted {
        Rooted::new(vec![(None, Arc::new(Fixed(a))), (Some("/h/.claude-work".into()), Arc::new(Fixed(b)))])
    }

    #[test]
    fn a_named_roots_sessions_carry_it_and_the_default_s_do_not() {
        let r = two(Ok(vec![json!({ "id": "claude:a" })]), Ok(vec![json!({ "id": "claude:b" })]));
        let list = r.sessions(&Unchecked).unwrap();
        assert_eq!(list, [json!({ "id": "claude:a" }), json!({ "id": "claude:b", "root": "/h/.claude-work" })]);
    }

    #[test]
    fn a_root_an_adapter_put_on_a_default_session_is_taken_off() {
        let r = two(Ok(vec![json!({ "id": "claude:a", "root": "/h/.claude-work" })]), Ok(Vec::new()));
        assert_eq!(r.sessions(&Unchecked).unwrap(), [json!({ "id": "claude:a" })]);
    }

    #[test]
    fn a_root_that_fails_does_not_hide_the_others() {
        let r = two(Ok(vec![json!({ "id": "claude:a" })]), Err("gone".into()));
        assert_eq!(r.sessions(&Unchecked).unwrap().len(), 1);
        let r = two(Err("first".into()), Err("second".into()));
        assert_eq!(r.sessions(&Unchecked), Err("first".into()));
    }

    #[test]
    fn a_transcript_is_read_from_the_root_that_has_it() {
        let r = two(Err("not here".into()), Ok(vec![json!(1)]));
        assert_eq!(r.transcript(&Unchecked, "b", None, 5.0, &Vec::new).unwrap()["raw"], "b");
    }
}
