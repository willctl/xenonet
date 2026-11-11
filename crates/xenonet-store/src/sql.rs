//! SQL over rusqlite, bound and read as the engine's tables (`engine`)
//! and the tests that look into them expect, and as the engine has always
//! written them. These are the store's own types, so rusqlite's stay
//! inside the crate:
//!
//! - A number (`Bind::Real`, what an f64 becomes) is bound as a REAL; a
//!   column with INTEGER affinity keeps a whole one as an integer, and one
//!   with TEXT affinity keeps "5.0". `Bind::Int` is an integer.
//! - A record's field that is a boolean, a list or an object can't be
//!   bound: an error naming the parameter.
//! - Named parameters come as a record, its keys without their prefix
//!   (:x, $x or @x); a key no parameter has is an error, and a parameter
//!   the record leaves out is NULL. Positional ones fill the anonymous
//!   parameters in order; too few leave the rest NULL, too many are an
//!   error.
//! - Rows are read as records: an integer as a number (one past 2^53 is
//!   an error), a real as a number, text, a blob as bytes; of two columns
//!   with one name, the last one's value.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, Statement};

/// A value bound to a statement's parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Bind {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Bind {
    /// A JSON scalar as a parameter: null, a number (a REAL, as every
    /// number binds) or a text. None for a boolean, a list or an object,
    /// which can't be bound.
    pub fn scalar(v: &serde_json::Value) -> Option<Bind> {
        use serde_json::Value as V;
        match v {
            V::Null => Some(Bind::Null),
            V::Number(n) => Some(Bind::Real(n.as_f64().unwrap_or(f64::NAN))),
            V::String(s) => Some(Bind::Text(s.clone())),
            V::Bool(_) | V::Array(_) | V::Object(_) => None,
        }
    }
}

impl From<&str> for Bind {
    fn from(s: &str) -> Self {
        Bind::Text(s.into())
    }
}

impl From<&String> for Bind {
    fn from(s: &String) -> Self {
        Bind::Text(s.clone())
    }
}

impl From<String> for Bind {
    fn from(s: String) -> Self {
        Bind::Text(s)
    }
}

/// A number binds as a REAL: see the module's doc.
impl From<f64> for Bind {
    fn from(n: f64) -> Self {
        Bind::Real(n)
    }
}

impl From<i64> for Bind {
    fn from(n: i64) -> Self {
        Bind::Int(n)
    }
}

impl<T: Into<Bind>> From<Option<T>> for Bind {
    fn from(v: Option<T>) -> Self {
        v.map_or(Bind::Null, Into::into)
    }
}

/// A JSON document, kept as its text; null stays NULL.
impl From<&serde_json::Value> for Bind {
    fn from(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Bind::Null,
            v => Bind::Text(xenonet_text::json::stringify(v, 0)),
        }
    }
}

/// A row as node:sqlite reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Cell {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(*i as f64),
            Cell::Real(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Cell::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Cell::Null)
    }
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Cell::Null => serde_json::Value::Null,
            Cell::Int(i) => serde_json::Value::from(*i),
            Cell::Real(f) => serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, serde_json::Value::Number),
            Cell::Text(s) => serde_json::Value::String(s.clone()),
            Cell::Blob(b) => serde_json::Value::Array(b.iter().map(|x| serde_json::Value::from(*x)).collect()),
        }
    }
    /// The cell as a value to bind again.
    pub fn bind(&self) -> Bind {
        match self {
            Cell::Null => Bind::Null,
            Cell::Int(i) => Bind::Int(*i),
            Cell::Real(f) => Bind::Real(*f),
            Cell::Text(s) => Bind::Text(s.clone()),
            Cell::Blob(b) => Bind::Blob(b.clone()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row(pub Vec<(String, Cell)>);

static NULL: Cell = Cell::Null;

impl Row {
    /// A column's value; NULL for one the row hasn't.
    pub fn get(&self, k: &str) -> &Cell {
        self.0.iter().find(|(n, _)| n == k).map_or(&NULL, |(_, v)| v)
    }
    pub fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(n, _)| n == k)
    }
    pub fn text(&self, k: &str) -> Option<&str> {
        self.get(k).as_str()
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        self.get(k).as_f64()
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(self.0.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
    }
}

/// How a call failed, as node:sqlite throws it.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlError {
    /// SQLite's own: its message, extended code and that code's words
    /// (ERR_SQLITE_ERROR).
    Sqlite { message: String, errcode: i32, errstr: String },
    /// A value that can't be bound (ERR_INVALID_ARG_TYPE, a TypeError).
    Bind(String),
    /// A number too large for JS (ERR_OUT_OF_RANGE).
    Range(String),
    /// The database isn't open (ERR_INVALID_STATE).
    State(String),
    /// Any other error, with its words.
    Other(String),
}

impl SqlError {
    pub fn message(&self) -> &str {
        match self {
            SqlError::Sqlite { message, .. } | SqlError::Bind(message) | SqlError::Range(message) | SqlError::State(message) | SqlError::Other(message) => message,
        }
    }
    pub fn code(&self) -> Option<&'static str> {
        match self {
            SqlError::Sqlite { .. } => Some("ERR_SQLITE_ERROR"),
            SqlError::Bind(_) => Some("ERR_INVALID_ARG_TYPE"),
            SqlError::Range(_) => Some("ERR_OUT_OF_RANGE"),
            SqlError::State(_) => Some("ERR_INVALID_STATE"),
            SqlError::Other(_) => None,
        }
    }
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<rusqlite::Error> for SqlError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::SqliteFailure(f, msg) => {
                let errstr = rusqlite::ffi::code_to_str(f.extended_code).to_string();
                SqlError::Sqlite { message: msg.unwrap_or_else(|| errstr.clone()), errcode: f.extended_code, errstr }
            }
            rusqlite::Error::SqlInputError { error, msg, .. } => {
                let errstr = rusqlite::ffi::code_to_str(error.extended_code).to_string();
                SqlError::Sqlite { message: msg, errcode: error.extended_code, errstr }
            }
            other => SqlError::Other(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, SqlError>;

/// The error for a value that can't be bound to parameter `index` (from
/// 1), as node:sqlite words it.
pub fn unbindable(index: usize) -> SqlError {
    SqlError::Bind(format!("Provided value cannot be bound to SQLite parameter {index}."))
}

fn bind_one(stmt: &mut Statement<'_>, index: usize, v: &Bind) -> Result<()> {
    let value = match v {
        Bind::Null => SqlValue::Null,
        Bind::Int(n) => SqlValue::Integer(*n),
        Bind::Real(n) => SqlValue::Real(*n),
        Bind::Text(s) => SqlValue::Text(s.clone()),
        Bind::Blob(b) => SqlValue::Blob(b.clone()),
    };
    if index == 0 || index > stmt.parameter_count() {
        return Err(SqlError::Sqlite { message: "column index out of range".into(), errcode: 25, errstr: "column index out of range".into() });
    }
    stmt.raw_bind_parameter(index, value).map_err(SqlError::from)
}

fn clear(stmt: &mut Statement<'_>) -> Result<()> {
    for i in 1..=stmt.parameter_count() {
        stmt.raw_bind_parameter(i, SqlValue::Null)?;
    }
    Ok(())
}

/// Binds the anonymous parameters in order; too few leave the rest NULL,
/// too many are an error.
pub(crate) fn bind(stmt: &mut Statement<'_>, args: &[Bind]) -> Result<()> {
    clear(stmt)?;
    let mut anon = 1;
    for v in args {
        while anon <= stmt.parameter_count() && stmt.parameter_name(anon).is_some() {
            anon += 1;
        }
        bind_one(stmt, anon, v)?;
        anon += 1;
    }
    Ok(())
}

/// Binds a record's fields to the named parameters (:x, $x or @x) of the
/// same names, each as `Bind::scalar` takes it. A field no parameter has is
/// an error, and a parameter the record leaves out is NULL.
pub(crate) fn bind_record(stmt: &mut Statement<'_>, record: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    clear(stmt)?;
    for (k, v) in record {
        let mut index = None;
        for prefix in [":", "$", "@"] {
            if let Some(i) = stmt.parameter_index(&format!("{prefix}{k}"))? {
                index = Some(i);
                break;
            }
        }
        let Some(i) = index else { return Err(SqlError::Other(format!("Unknown named parameter '{k}'"))) };
        bind_one(stmt, i, &Bind::scalar(v).ok_or_else(|| unbindable(i))?)?;
    }
    Ok(())
}

const MAX_SAFE: i64 = 9_007_199_254_740_991;

fn read_rows(stmt: &mut Statement<'_>, first: bool) -> Result<Vec<Row>> {
    let names: Vec<String> = stmt.column_names().map(str::to_string).collect();
    let mut rows = stmt.raw_query();
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut row: Vec<(String, Cell)> = Vec::with_capacity(names.len());
        for (i, n) in names.iter().enumerate() {
            let cell = match r.get_ref(i)? {
                ValueRef::Null => Cell::Null,
                ValueRef::Integer(v) => {
                    if !(-MAX_SAFE..=MAX_SAFE).contains(&v) {
                        return Err(SqlError::Range(format!("Value is too large to be represented as a JavaScript number: {v}")));
                    }
                    Cell::Int(v)
                }
                ValueRef::Real(f) => Cell::Real(f),
                ValueRef::Text(t) => Cell::Text(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
            };
            match row.iter_mut().find(|(k, _)| k == n) {
                Some(slot) => slot.1 = cell,
                None => row.push((n.clone(), cell)),
            }
        }
        out.push(Row(row));
        if first {
            break;
        }
    }
    Ok(out)
}

/// A statement prepared (and kept, as the store keeps the ones it runs
/// often): its rows.
pub(crate) fn all(db: &Connection, sql: &str, args: &[Bind]) -> Result<Vec<Row>> {
    let mut st = db.prepare_cached(sql)?;
    bind(&mut st, args)?;
    read_rows(&mut st, false)
}

/// Its first row, or None.
pub(crate) fn get(db: &Connection, sql: &str, args: &[Bind]) -> Result<Option<Row>> {
    let mut st = db.prepare_cached(sql)?;
    bind(&mut st, args)?;
    Ok(read_rows(&mut st, true)?.into_iter().next())
}

/// What a run changed: rows, and the last rowid inserted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Changes {
    pub changes: i64,
    pub last_insert_rowid: i64,
}

pub(crate) fn run(db: &Connection, sql: &str, args: &[Bind]) -> Result<Changes> {
    let mut st = db.prepare_cached(sql)?;
    bind(&mut st, args)?;
    st.raw_execute()?;
    Ok(Changes { changes: db.changes() as i64, last_insert_rowid: db.last_insert_rowid() })
}

/// A run with a record's fields as its named parameters (`bind_record`).
pub(crate) fn run_record(db: &Connection, sql: &str, record: &serde_json::Map<String, serde_json::Value>) -> Result<Changes> {
    let mut st = db.prepare_cached(sql)?;
    bind_record(&mut st, record)?;
    st.raw_execute()?;
    Ok(Changes { changes: db.changes() as i64, last_insert_rowid: db.last_insert_rowid() })
}

#[cfg(test)]
fn exec(db: &Connection, sql: &str) -> Result<()> {
    db.execute_batch(sql).map_err(SqlError::from)
}

/// Another program's database, open to read only (an observer reads
/// what an actor's own app keeps), with its rows read as the store's are.
pub struct ReadOnly(Connection);

impl ReadOnly {
    pub fn open(file: &str) -> Result<ReadOnly> {
        let c = Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        Ok(ReadOnly(c))
    }

    /// A query's rows, its anonymous parameters bound in order.
    pub fn all(&self, sql: &str, args: &[Bind]) -> Result<Vec<Row>> {
        all(&self.0, sql, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_and_reads_as_node_does() {
        let db = Connection::open_in_memory().unwrap();
        exec(&db, "CREATE TABLE t (a TEXT, b, c INTEGER)").unwrap();
        run(&db, "INSERT INTO t VALUES (?, ?, ?)", &[Bind::Real(5.0), Bind::Real(5.0), Bind::Real(5.0)]).unwrap();
        let r = get(&db, "SELECT a, b, typeof(b) tb, c, typeof(c) tc FROM t", &[]).unwrap().unwrap();
        assert_eq!(r.get("a"), &Cell::Text("5.0".into()));
        assert_eq!((r.get("b"), r.text("tb")), (&Cell::Real(5.0), Some("real")));
        assert_eq!((r.get("c"), r.text("tc")), (&Cell::Int(5), Some("integer")));
        let record = |v: serde_json::Value| v.as_object().cloned().unwrap();
        assert!(matches!(run_record(&db, "INSERT INTO t VALUES (:a, :b, :c)", &record(serde_json::json!({ "b": true }))), Err(SqlError::Bind(m)) if m.contains("parameter 2")));
        assert!(
            matches!(run_record(&db, "INSERT INTO t VALUES (:a, :b, :c)", &record(serde_json::json!({ "z": null }))), Err(SqlError::Other(m)) if m == "Unknown named parameter 'z'")
        );
        run_record(&db, "INSERT INTO t VALUES (:a, $b, @c)", &record(serde_json::json!({ "a": "x" }))).unwrap();
        assert_eq!(get(&db, "SELECT * FROM t WHERE a = 'x'", &[]).unwrap().unwrap().get("b"), &Cell::Null);
        assert!(matches!(run(&db, "INSERT INTO t VALUES (?, ?, ?)", &[Bind::Null, Bind::Null, Bind::Null, Bind::Null]), Err(SqlError::Sqlite { errcode: 25, .. })));
        assert!(matches!(get(&db, "SELECT 9007199254740993 n", &[]), Err(SqlError::Range(_))));
        assert_eq!(get(&db, "SELECT 1 a, 2 a", &[]).unwrap().unwrap().0, vec![("a".to_string(), Cell::Int(2))]);
        assert_eq!((Bind::from(&serde_json::json!({ "k": 1 })), Bind::from(&serde_json::Value::Null)), (Bind::Text("{\"k\":1}".into()), Bind::Null));
        match all(&db, "SELECT * FROM nope", &[]) {
            Err(SqlError::Sqlite { message, errcode, errstr }) => assert_eq!((message.as_str(), errcode, errstr.as_str()), ("no such table: nope", 1, "SQL logic error")),
            other => panic!("{other:?}"),
        }
    }
}
