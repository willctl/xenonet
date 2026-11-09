//! Codex desktop's projects, read-only from its state database
//! (`state_<n>.sqlite` under Codex's home): their rows, as node:sqlite gives
//! them, checked leniently and read into project records.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};
use xenonet_adapter::reader::VendorCheck;
use xenonet_base::json as js;
use xenonet_sources::lines::{prop, Js, TypeError};
use xenonet_sources::record::{nn, obj_json, Obj, F};

/// The state database XenoNet reads, and its format's version.
pub const STATE_DB: u32 = 5;

/// The numbers of the state databases in Codex's folder that aren't the
/// one XenoNet reads (state_6.sqlite), as they're written.
pub fn other_state_dbs(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter_map(|n| n.strip_prefix("state_")?.strip_suffix(".sqlite").map(str::to_string))
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .filter(|n| n.parse::<f64>().is_ok_and(|x| x != f64::from(STATE_DB)))
        .collect()
}

/// A value from SQLite, as node:sqlite gives it to JS.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    V(Value),
    Bytes(Vec<u8>),
}

/// What the state database says: the projects' rows and their roots', or
/// the tables and columns it reads that aren't there.
#[derive(Clone, Debug, PartialEq)]
pub enum StateRows {
    Rows { projects: Vec<Vec<(&'static str, Cell)>>, roots: Vec<Vec<(&'static str, Cell)>> },
    Missing(Vec<String>),
}

fn rows(db: &Connection, sql: &str, cols: [&'static str; 2]) -> rusqlite::Result<Vec<Vec<(&'static str, Cell)>>> {
    let mut st = db.prepare(sql)?;
    let mut rs = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rs.next()? {
        let mut row = Vec::new();
        for (i, c) in cols.iter().enumerate() {
            let cell = match r.get_ref(i)? {
                ValueRef::Null => Cell::V(Value::Null),
                ValueRef::Integer(n) => {
                    // node:sqlite refuses a whole number JS can't hold.
                    if n.unsigned_abs() > 9_007_199_254_740_991 {
                        return Err(rusqlite::Error::IntegralValueOutOfRange(i, n));
                    }
                    Cell::V(js::number(n as f64))
                }
                ValueRef::Real(x) => Cell::V(js::number(x)),
                ValueRef::Text(t) => Cell::V(Value::String(String::from_utf8_lossy(t).into_owned())),
                ValueRef::Blob(b) => Cell::Bytes(b.to_vec()),
            };
            row.push((*c, cell));
        }
        out.push(row);
    }
    Ok(out)
}

// A query that failed in a database that opened: the tables and columns
// it reads that aren't there. A file that isn't a database has none.
fn missing_columns(db: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for (table, columns) in [("projects", ["id", "name"]), ("project_roots", ["project_id", "path"])] {
        let have: rusqlite::Result<Vec<String>> = (|| {
            let mut st = db.prepare(&format!("pragma table_info({table})"))?;
            let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(names)
        })();
        let Ok(have) = have else { return out };
        if have.is_empty() {
            out.push(table.to_string());
        } else {
            for c in columns {
                if !have.iter().any(|h| h == c) {
                    out.push(format!("{table}[].{c}"));
                }
            }
        }
    }
    out
}

/// Reads Codex desktop's projects from its state database, read-only.
/// None when there's no database (or it can't be opened).
pub fn read_state_db(file: &str) -> Option<StateRows> {
    if !std::path::Path::new(file).exists() {
        return None;
    }
    let db = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let read = rows(&db, "select id, name from projects", ["id", "name"])
        .and_then(|p| Ok((p, rows(&db, "select project_id, path from project_roots order by position", ["project_id", "path"])?)));
    Some(match read {
        Ok((projects, roots)) => StateRows::Rows { projects, roots },
        Err(_) => StateRows::Missing(missing_columns(&db)),
    })
}

// a === b, for values from JSON: objects are never the same one.
fn strict_eq(a: Js<'_>, b: Js<'_>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// Codex desktop's projects from the state database's rows, as the
/// lenient reader left them: each with its roots, in their order.
pub fn native_projects(rows: Js<'_>) -> Result<Vec<Obj>, TypeError> {
    let roots = nn(prop(rows, "project_roots")?).cloned();
    let projects = match nn(prop(rows, "projects")?) {
        None => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(TypeError("(rows.projects ?? []).map is not a function".into())),
    };
    let mut out = Vec::new();
    for p in &projects {
        let id = prop(Some(p), "id")?;
        let roots: &[Value] = match &roots {
            None => &[],
            Some(Value::Array(a)) => a,
            Some(_) => return Err(TypeError("roots.filter is not a function".into())),
        };
        let mut mine = Vec::new();
        for r in roots {
            if strict_eq(prop(Some(r), "project_id")?, id) {
                mine.push(F::of(prop(Some(r), "path")?));
            }
        }
        out.push(vec![("id", F::of(id)), ("name", F::of(prop(Some(p), "name")?)), ("roots", F::Arr(mine))]);
    }
    Ok(out)
}

/// Codex desktop's projects, read-only from its state database. A
/// database of another number is a version the schema wasn't made for,
/// and a column it reads gone is drift; any failure only takes the
/// grouping away.
pub fn projects(codex_home: &str, vendor: &dyn VendorCheck) -> Vec<Value> {
    let names: Vec<String> = std::fs::read_dir(codex_home).map(|d| d.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect()).unwrap_or_default();
    for n in other_state_dbs(&names) {
        vendor.pick("openai/codex/state-db", Some(&n));
    }
    let file = std::path::Path::new(codex_home).join(format!("state_{STATE_DB}.sqlite"));
    let Some(read) = read_state_db(&file.to_string_lossy()) else { return Vec::new() };
    let rows = match read {
        StateRows::Missing(m) => {
            for path in m {
                vendor.missing("openai/codex/state-db", &path, Some(&STATE_DB.to_string()));
            }
            return Vec::new();
        }
        StateRows::Rows { projects, roots } => {
            let cells = |rows: Vec<Vec<(&'static str, Cell)>>| -> Value {
                Value::Array(
                    rows.into_iter()
                        .map(|r| {
                            Value::Object(
                                r.into_iter()
                                    .map(|(k, c)| {
                                        (
                                            k.to_string(),
                                            match c {
                                                Cell::V(v) => v,
                                                Cell::Bytes(b) => json!({ "type": "Buffer", "data": b }),
                                            },
                                        )
                                    })
                                    .collect::<Map<String, Value>>(),
                            )
                        })
                        .collect(),
                )
            };
            json!({ "projects": cells(projects), "project_roots": cells(roots) })
        }
    };
    let checked = vendor.check("openai/codex/state-db", rows, Some(&STATE_DB.to_string()), None);
    match native_projects(checked.as_ref()) {
        Ok(list) => list.iter().map(obj_json).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::reader::Unchecked;

    #[test]
    fn lists_other_state_dbs() {
        let names: Vec<String> = ["state_5.sqlite", "state_6.sqlite", "state_05.sqlite", "state_x.sqlite", "notes"].iter().map(ToString::to_string).collect();
        assert_eq!(other_state_dbs(&names), ["6"]);
    }

    #[test]
    fn a_home_without_a_database_has_no_projects() {
        assert_eq!(projects("/nowhere", &Unchecked), [] as [Value; 0]);
    }
}
