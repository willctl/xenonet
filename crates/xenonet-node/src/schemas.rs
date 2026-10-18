//! xenonet-schema for the twins of core/schema/'s registry, check and
//! files: the ids defined and the documents written (`SchemaRegistry`),
//! data checked against a schema's JSON Schema (`SchemaChecker`), and the
//! files in schemas/. A value to check is read as check() sees it, with
//! what JSON leaves out (undefined, NaN, a Date) kept.

use crate::js::{type_of, Json};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::ptr;
use xenonet_schema::check::{self, Checked, Checker, Data, Object, PathKey};
use xenonet_schema::files;
use xenonet_schema::registry::Registry;

const MAX_DEPTH: usize = 512;

fn ok(status: sys::napi_status, what: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Error::new(Status::GenericFailure, format!("couldn't {what} ({status})")))
    }
}

/// A JS value to check.
pub struct CheckData(Data);

// This realm's Object.prototype, which a plain object has for its own.
unsafe fn object_prototype(env: sys::napi_env) -> Result<sys::napi_value> {
    let mut global = ptr::null_mut();
    ok(unsafe { sys::napi_get_global(env, &raw mut global) }, "find the globals")?;
    let object = unsafe { named(env, global, c"Object")? };
    unsafe { named(env, object, c"prototype") }
}

unsafe fn named(env: sys::napi_env, v: sys::napi_value, name: &std::ffi::CStr) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    ok(unsafe { sys::napi_get_named_property(env, v, name.as_ptr(), &raw mut out) }, "read a property")?;
    Ok(out)
}

unsafe fn truthy(env: sys::napi_env, v: sys::napi_value) -> Result<bool> {
    let mut b = ptr::null_mut();
    ok(unsafe { sys::napi_coerce_to_bool(env, v, &raw mut b) }, "tell truthiness")?;
    unsafe { bool::from_napi_value(env, b) }
}

// An object to zod's isObject: typeof "object", not null, not an array.
unsafe fn is_object(env: sys::napi_env, v: sys::napi_value) -> Result<bool> {
    if unsafe { type_of(env, v)? } != sys::ValueType::napi_object {
        return Ok(false);
    }
    let mut is = false;
    ok(unsafe { sys::napi_is_array(env, v, &raw mut is) }, "tell an array")?;
    Ok(!is)
}

// For an object whose prototype isn't Object.prototype: the name zod's
// parsedType gives it (its constructor's), and whether zod's isPlainObject
// takes it (no constructor, or one whose prototype has its own
// isPrototypeOf, as Object's has).
unsafe fn class_of(env: sys::napi_env, v: sys::napi_value) -> Result<(Option<String>, bool)> {
    let mut has = false;
    ok(unsafe { sys::napi_has_named_property(env, v, c"constructor".as_ptr(), &raw mut has) }, "look for a constructor")?;
    let ctor = unsafe { named(env, v, c"constructor")? };
    let mut class = None;
    if has && unsafe { truthy(env, ctor)? } {
        let name = unsafe { named(env, ctor, c"name")? };
        let mut text = name;
        if unsafe { type_of(env, name)? } != sys::ValueType::napi_string {
            ok(unsafe { sys::napi_coerce_to_string(env, name, &raw mut text) }, "name a constructor")?;
        }
        class = Some(unsafe { String::from_napi_value(env, text)? });
    }
    let plain = if unsafe { type_of(env, ctor)? } != sys::ValueType::napi_function {
        true
    } else {
        let proto = unsafe { named(env, ctor, c"prototype")? };
        if !unsafe { is_object(env, proto)? } {
            false
        } else {
            let key = unsafe { String::to_napi_value(env, "isPrototypeOf".to_string())? };
            let mut own = false;
            ok(unsafe { sys::napi_has_own_property(env, proto, key, &raw mut own) }, "look at a prototype")?;
            own
        }
    };
    Ok((class, plain))
}

// What a read needs: this realm's Object.prototype, and the objects it's
// inside, so a value that holds itself is read once.
struct Reader {
    object_proto: sys::napi_value,
    inside: Vec<sys::napi_value>,
}

unsafe fn read(env: sys::napi_env, v: sys::napi_value, r: &mut Reader) -> Result<Data> {
    use sys::ValueType::{napi_boolean, napi_function, napi_null, napi_number, napi_object, napi_string, napi_symbol, napi_undefined};
    if r.inside.len() > MAX_DEPTH {
        return Err(Error::new(Status::InvalidArg, "a value nested too deeply".to_string()));
    }
    let t = unsafe { type_of(env, v)? };
    Ok(match t {
        _ if t == napi_undefined => Data::Undefined,
        _ if t == napi_null => Data::Null,
        _ if t == napi_boolean => Data::Bool(unsafe { bool::from_napi_value(env, v)? }),
        _ if t == napi_number => Data::Num(unsafe { f64::from_napi_value(env, v)? }),
        _ if t == napi_string => Data::Str(unsafe { String::from_napi_value(env, v)? }),
        _ if t == napi_symbol => Data::Other("symbol"),
        _ if t == napi_function => Data::Fn(unsafe { f64::from_napi_value(env, named(env, v, c"length")?)? }),
        // napi_bigint
        9 => Data::Other("bigint"),
        _ if t == napi_object => {
            let mut is = false;
            ok(unsafe { sys::napi_is_array(env, v, &raw mut is) }, "tell an array")?;
            let mut again = false;
            for outer in &r.inside {
                ok(unsafe { sys::napi_strict_equals(env, *outer, v, &raw mut again) }, "compare objects")?;
                if again {
                    break;
                }
            }
            r.inside.push(v);
            let read = if is { unsafe { read_array(env, v, r, again)? } } else { unsafe { read_object(env, v, r, again)? } };
            r.inside.pop();
            read
        }
        _ => Data::Obj(Object::new()),
    })
}

// An array; one inside itself is read as empty.
unsafe fn read_array(env: sys::napi_env, v: sys::napi_value, r: &mut Reader, again: bool) -> Result<Data> {
    if again {
        return Ok(Data::Arr(Vec::new()));
    }
    let mut n = 0u32;
    ok(unsafe { sys::napi_get_array_length(env, v, &raw mut n) }, "read an array's length")?;
    let mut a = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut item = ptr::null_mut();
        ok(unsafe { sys::napi_get_element(env, v, i, &raw mut item) }, "read an array")?;
        a.push(unsafe { read(env, item, r)? });
    }
    Ok(Data::Arr(a))
}

// An object: its own enumerable keys, and what kind of object it is. One
// inside itself is read as the same kind of object without its keys (zod
// goes into it only as far as the schema does, which XenoNet's never need).
unsafe fn read_object(env: sys::napi_env, v: sys::napi_value, r: &mut Reader, again: bool) -> Result<Data> {
    let mut o = Object::new();
    let mut proto = ptr::null_mut();
    ok(unsafe { sys::napi_get_prototype(env, v, &raw mut proto) }, "find a prototype")?;
    let mut same = false;
    ok(unsafe { sys::napi_strict_equals(env, proto, r.object_proto, &raw mut same) }, "compare prototypes")?;
    if !same {
        (o.class, o.plain) = unsafe { class_of(env, v)? };
    }
    if again {
        return Ok(Data::Obj(o));
    }
    let mut keys = ptr::null_mut();
    ok(
        unsafe {
            sys::napi_get_all_property_names(
                env,
                v,
                sys::KeyCollectionMode::own_only,
                sys::KeyFilter::enumerable | sys::KeyFilter::skip_symbols,
                sys::KeyConversion::numbers_to_strings,
                &raw mut keys,
            )
        },
        "list an object's keys",
    )?;
    let mut n = 0u32;
    ok(unsafe { sys::napi_get_array_length(env, keys, &raw mut n) }, "count an object's keys")?;
    for i in 0..n {
        let mut key = ptr::null_mut();
        ok(unsafe { sys::napi_get_element(env, keys, i, &raw mut key) }, "read a key")?;
        let mut item = ptr::null_mut();
        ok(unsafe { sys::napi_get_property(env, v, key, &raw mut item) }, "read a property")?;
        o.insert(unsafe { String::from_napi_value(env, key)? }, unsafe { read(env, item, r)? });
    }
    Ok(Data::Obj(o))
}

impl FromNapiValue for CheckData {
    unsafe fn from_napi_value(env: sys::napi_env, v: sys::napi_value) -> Result<Self> {
        let proto = unsafe { object_prototype(env)? };
        Ok(CheckData(unsafe { read(env, v, &mut Reader { object_proto: proto, inside: Vec::new() })? }))
    }
}

impl TypeName for CheckData {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl ValidateNapiValue for CheckData {}

fn checked_json(r: Checked) -> Json {
    let ok = r.ok();
    let issues: Vec<Value> = r.issues.into_iter().map(|i| json!({ "path": i.path, "code": i.code, "message": i.message })).collect();
    Json(json!({ "ok": ok, "issues": issues, "undeclared": r.undeclared }))
}

fn thrown(message: String) -> Error {
    Error::new(Status::InvalidArg, message)
}

// ---------- the registry ----------

#[napi(js_name = "SchemaRegistry")]
pub struct JsRegistry {
    inner: Registry,
}

#[napi]
impl JsRegistry {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsRegistry { inner: Registry::new() }
    }

    /// Why `id` can't be defined, or null.
    #[napi]
    pub fn refusal(&self, id: String, has_title: bool, has_description: bool) -> Option<String> {
        self.inner.refusal(&id, has_title, has_description)
    }

    #[napi]
    pub fn add(&mut self, id: String) {
        self.inner.add(&id);
    }

    /// The short id of a defined schema, from either form, or null.
    #[napi]
    pub fn find(&self, id: String) -> Option<String> {
        self.inner.find(&id)
    }

    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.inner.ids()
    }

    /// Takes what z.toJSONSchema wrote for the registry, by id.
    #[napi(js_name = "setDocuments")]
    pub fn set_documents(&mut self, schemas: Json) {
        self.inner.set_documents(schemas.0.as_object().unwrap_or(&Map::new()));
    }

    #[napi]
    pub fn documents(&self) -> Json {
        Json(Value::Object(self.inner.documents()))
    }

    /// [{ file, text }], as schemaFiles() gives them.
    #[napi]
    pub fn files(&self) -> Json {
        Json(Value::Array(self.inner.files().into_iter().map(|(file, text)| json!({ "file": file, "text": text })).collect()))
    }
}

// ---------- checking ----------

#[napi(js_name = "SchemaChecker")]
pub struct JsChecker {
    checker: Checker,
    roots: HashMap<String, usize>,
}

// The short id of a document, from its $id.
fn short_of(doc: &Value) -> Option<String> {
    Some(xenonet_base::ids::parse_schema_id(doc.get("$id")?.as_str()?)?.id)
}

#[napi]
impl JsChecker {
    /// check() against the document with this short id, or the only one
    /// compiled when it's null.
    #[napi]
    pub fn check(&self, id: Option<String>, data: CheckData, strict: bool) -> Result<Json> {
        let root = match &id {
            Some(id) => *self.roots.get(id).ok_or_else(|| thrown(format!("XenoNet has no schema {id}")))?,
            None => self.checker.root(0),
        };
        Ok(checked_json(self.checker.check(root, &data.0, strict)))
    }
}

/// A checker for one JSON Schema document.
#[napi(js_name = "schemaChecker")]
pub fn schema_checker(doc: Json) -> Result<JsChecker> {
    let checker = check::compile(std::slice::from_ref(&doc.0)).map_err(thrown)?;
    Ok(JsChecker { checker, roots: HashMap::new() })
}

/// A checker for documents that refer to each other by $id, each found by
/// its short id.
#[napi(js_name = "schemaCheckers")]
pub fn schema_checkers(docs: Json) -> Result<JsChecker> {
    let docs: Vec<Value> = docs.0.as_array().cloned().unwrap_or_default();
    let checker = check::compile(&docs).map_err(thrown)?;
    let roots = docs.iter().enumerate().filter_map(|(i, d)| Some((short_of(d)?, checker.root(i)))).collect();
    Ok(JsChecker { checker, roots })
}

/// pathText(): numbers as indexes, anything else as a key.
#[napi(js_name = "schemaPathText")]
pub fn schema_path_text(path: Vec<Either<f64, String>>) -> String {
    let keys: Vec<PathKey> = path
        .into_iter()
        .map(|k| match k {
            Either::A(n) => PathKey::Num(n),
            Either::B(s) => PathKey::Key(s),
        })
        .collect();
    check::path_text(&keys)
}

// A field of an issue as a template literal writes it.
fn field(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => xenonet_base::text::js_string(v),
    }
}

/// explain(): an issue's path, or "(the value)" when it has none, and its
/// message; then the keys not in the schema.
#[napi(js_name = "schemaExplain")]
pub fn schema_explain(issues: Json, undeclared: Json) -> String {
    let issues: Vec<(String, String)> = issues
        .0
        .as_array()
        .map(|a| {
            a.iter()
                .map(|i| {
                    let path = i.get("path").filter(|p| xenonet_base::text::truthy(Some(p))).map_or_else(String::new, |p| field(Some(p)));
                    (path, field(i.get("message")))
                })
                .collect()
        })
        .unwrap_or_default();
    let undeclared: Vec<String> = undeclared.0.as_array().map(|a| a.iter().map(|p| field(Some(p))).collect()).unwrap_or_default();
    check::explain(&issues, &undeclared)
}

// ---------- files ----------

fn wanted(want: &Json) -> Vec<(String, String)> {
    want.0.as_array().map(|a| a.iter().map(|f| (field(f.get("file")), field(f.get("text")))).collect()).unwrap_or_default()
}

/// schemaFilesState(): `dir` against the files wanted ([{ file, text }]).
#[napi(js_name = "schemaFilesState")]
pub fn schema_files_state(dir: String, want: Json) -> Json {
    let s = files::state(&dir, &wanted(&want));
    Json(json!({ "current": s.current, "missing": s.missing, "changed": s.changed, "extra": s.extra }))
}

/// writeSchemaFiles(): { written, removed }.
#[napi(js_name = "writeSchemaFiles")]
pub fn write_schema_files(dir: String, want: Json) -> Result<Json> {
    let (written, removed) = files::write(&dir, &wanted(&want)).map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
    Ok(Json(json!({ "written": written, "removed": removed })))
}
