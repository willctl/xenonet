//! SQL as node:sqlite runs it, over rusqlite: what the engine's store
//! (core/engine/store.js) and the tests that look into it expect of a
//! DatabaseSync and its statements.
//!
//! - A number is bound as a REAL, as node:sqlite binds every JS number;
//!   a column with INTEGER affinity keeps a whole one as an integer, and
//!   one with TEXT affinity keeps "5.0". A BigInt is an integer.
//! - undefined, a boolean or an object can't be bound: a TypeError.
//! - Named parameters come as an object, its keys without their prefix
//!   (:x, $x or @x); a key no parameter has is an error, and a parameter
//!   the object leaves out is NULL. Positional ones fill the anonymous
//!   parameters in order; too few leave the rest NULL, too many are an
//!   error.
//! - Rows are read as JS objects: an integer as a number (one past 2^53 is
//!   an error), a real as a number, text, a blob as bytes; of two columns
//!   with one name, the last one's value.

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, Statement};

/// A value from the caller, as JS gave it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Undef,
    Null,
    Num(f64),
    Big(i64),
    Text(String),
    Bool(bool),
    Bytes(Vec<u8>),
    List(Vec<Arg>),
    Obj(Vec<(String, Arg)>),
    /// A function, a symbol, anything else JS has.
    Other,
}

impl Arg {
    pub fn text(s: impl Into<String>) -> Arg {
        Arg::Text(s.into())
    }
    pub fn num(n: f64) -> Arg {
        Arg::Num(n)
    }
    pub fn opt_text(s: Option<&str>) -> Arg {
        s.map_or(Arg::Null, |s| Arg::Text(s.into()))
    }
    pub fn opt_num(n: Option<f64>) -> Arg {
        n.map_or(Arg::Null, Arg::Num)
    }
    /// A field of an object, Undef when it isn't there.
    #[must_use]
    pub fn field(&self, k: &str) -> Arg {
        match self {
            Arg::Obj(fields) => fields.iter().rev().find(|(n, _)| n == k).map_or(Arg::Undef, |(_, v)| v.clone()),
            _ => Arg::Undef,
        }
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Arg::Undef | Arg::Null)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Arg::Num(n) => Some(*n),
            Arg::Big(n) => Some(*n as f64),
            _ => None,
        }
    }
    /// JS's truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Arg::Undef | Arg::Null => false,
            Arg::Num(n) => *n != 0.0 && !n.is_nan(),
            Arg::Big(n) => *n != 0,
            Arg::Text(s) => !s.is_empty(),
            Arg::Bool(b) => *b,
            _ => true,
        }
    }
    /// What `value ?? fallback` takes.
    #[must_use]
    pub fn or(self, fallback: Arg) -> Arg {
        if self.is_nullish() {
            fallback
        } else {
            self
        }
    }
    /// The value as JSON, as JSON.stringify writes it (undefined fields
    /// left out, null in a list).
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as V;
        match self {
            Arg::Undef | Arg::Null | Arg::Other => V::Null,
            Arg::Num(n) => xenonet_base::json::number(*n),
            Arg::Big(n) => V::from(*n),
            Arg::Text(s) => V::String(s.clone()),
            Arg::Bool(b) => V::Bool(*b),
            Arg::Bytes(b) => V::Object(b.iter().enumerate().map(|(i, x)| (i.to_string(), V::from(*x))).collect()),
            Arg::List(a) => V::Array(a.iter().map(Arg::to_json).collect()),
            Arg::Obj(f) => V::Object(f.iter().filter(|(_, v)| !matches!(v, Arg::Undef | Arg::Other)).map(|(k, v)| (k.clone(), v.to_json())).collect()),
        }
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Text(s.into())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Text(s)
    }
}

impl From<f64> for Arg {
    fn from(n: f64) -> Self {
        Arg::Num(n)
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
    pub fn arg(&self) -> Arg {
        match self {
            Cell::Null => Arg::Null,
            Cell::Int(i) => Arg::Big(*i),
            Cell::Real(f) => Arg::Num(*f),
            Cell::Text(s) => Arg::Text(s.clone()),
            Cell::Blob(b) => Arg::Bytes(b.clone()),
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

fn bind_one(stmt: &mut Statement<'_>, index: usize, v: &Arg) -> Result<()> {
    let value = match v {
        Arg::Null => SqlValue::Null,
        Arg::Num(n) => SqlValue::Real(*n),
        Arg::Big(n) => SqlValue::Integer(*n),
        Arg::Text(s) => SqlValue::Text(s.clone()),
        Arg::Bytes(b) => SqlValue::Blob(b.clone()),
        _ => return Err(SqlError::Bind(format!("Provided value cannot be bound to SQLite parameter {index}."))),
    };
    if index == 0 || index > stmt.parameter_count() {
        return Err(SqlError::Sqlite { message: "column index out of range".into(), errcode: 25, errstr: "column index out of range".into() });
    }
    stmt.raw_bind_parameter(index, value).map_err(SqlError::from)
}

/// Binds a call's arguments as node:sqlite does.
pub fn bind(stmt: &mut Statement<'_>, args: &[Arg]) -> Result<()> {
    for i in 1..=stmt.parameter_count() {
        stmt.raw_bind_parameter(i, SqlValue::Null)?;
    }
    let mut rest = args;
    if let Some(Arg::Obj(fields)) = args.first() {
        for (k, v) in fields {
            if matches!(v, Arg::Other) {
                continue;
            }
            let mut index = None;
            for prefix in [":", "$", "@"] {
                if let Some(i) = stmt.parameter_index(&format!("{prefix}{k}"))? {
                    index = Some(i);
                    break;
                }
            }
            let Some(i) = index else { return Err(SqlError::Other(format!("Unknown named parameter '{k}'"))) };
            bind_one(stmt, i, v)?;
        }
        rest = &args[1..];
    }
    let mut anon = 1;
    for v in rest {
        while anon <= stmt.parameter_count() && stmt.parameter_name(anon).is_some() {
            anon += 1;
        }
        bind_one(stmt, anon, v)?;
        anon += 1;
    }
    Ok(())
}

const MAX_SAFE: i64 = 9_007_199_254_740_991;

fn read_rows(stmt: &mut Statement<'_>, first: bool) -> Result<Vec<Row>> {
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
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
pub fn all(db: &Connection, sql: &str, args: &[Arg]) -> Result<Vec<Row>> {
    let mut st = db.prepare_cached(sql)?;
    bind(&mut st, args)?;
    read_rows(&mut st, false)
}

/// Its first row, or None.
pub fn get(db: &Connection, sql: &str, args: &[Arg]) -> Result<Option<Row>> {
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

pub fn run(db: &Connection, sql: &str, args: &[Arg]) -> Result<Changes> {
    let mut st = db.prepare_cached(sql)?;
    bind(&mut st, args)?;
    st.raw_execute()?;
    Ok(Changes { changes: db.changes() as i64, last_insert_rowid: db.last_insert_rowid() })
}

/// Checks a statement prepares, as DatabaseSync.prepare does.
pub fn prepare(db: &Connection, sql: &str) -> Result<()> {
    db.prepare_cached(sql)?;
    Ok(())
}

pub fn exec(db: &Connection, sql: &str) -> Result<()> {
    db.execute_batch(sql).map_err(SqlError::from)
}

/// `fn` in a transaction of its own, unless one is under way already.
pub fn transaction<T>(db: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if !db.is_autocommit() {
        return f();
    }
    exec(db, "BEGIN")?;
    match f().and_then(|out| exec(db, "COMMIT").map(|()| out)) {
        Ok(out) => Ok(out),
        Err(e) => {
            let _ = exec(db, "ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_and_reads_as_node_does() {
        let db = Connection::open_in_memory().unwrap();
        exec(&db, "CREATE TABLE t (a TEXT, b, c INTEGER)").unwrap();
        run(&db, "INSERT INTO t VALUES (?, ?, ?)", &[Arg::Num(5.0), Arg::Num(5.0), Arg::Num(5.0)]).unwrap();
        let r = get(&db, "SELECT a, b, typeof(b) tb, c, typeof(c) tc FROM t", &[]).unwrap().unwrap();
        assert_eq!(r.get("a"), &Cell::Text("5.0".into()));
        assert_eq!((r.get("b"), r.text("tb")), (&Cell::Real(5.0), Some("real")));
        assert_eq!((r.get("c"), r.text("tc")), (&Cell::Int(5), Some("integer")));
        assert!(matches!(run(&db, "INSERT INTO t VALUES (?, ?, ?)", &[Arg::Undef]), Err(SqlError::Bind(m)) if m.contains("parameter 1")));
        assert!(matches!(run(&db, "INSERT INTO t VALUES (:a, :b, :c)", &[Arg::Obj(vec![("z".into(), Arg::Null)])]), Err(SqlError::Other(m)) if m == "Unknown named parameter 'z'"));
        run(&db, "INSERT INTO t VALUES (:a, $b, @c)", &[Arg::Obj(vec![("a".into(), Arg::text("x"))])]).unwrap();
        assert_eq!(get(&db, "SELECT * FROM t WHERE a = 'x'", &[]).unwrap().unwrap().get("b"), &Cell::Null);
        assert!(matches!(run(&db, "INSERT INTO t VALUES (?, ?, ?)", &[Arg::Null, Arg::Null, Arg::Null, Arg::Null]), Err(SqlError::Sqlite { errcode: 25, .. })));
        assert!(matches!(get(&db, "SELECT 9007199254740993 n", &[]), Err(SqlError::Range(_))));
        assert_eq!(get(&db, "SELECT 1 a, 2 a", &[]).unwrap().unwrap().0, vec![("a".to_string(), Cell::Int(2))]);
        match all(&db, "SELECT * FROM nope", &[]) {
            Err(SqlError::Sqlite { message, errcode, errstr }) => assert_eq!((message.as_str(), errcode, errstr.as_str()), ("no such table: nope", 1, "SQL logic error")),
            other => panic!("{other:?}"),
        }
    }
}
