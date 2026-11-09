//! Codex's threads as the map lists them (a thread as a session), and a
//! thread's items from the app-server as the console shows them.

use serde_json::{Map, Value};
use xenonet_base::text::{js_number, js_string, truthy};
use xenonet_base::{json, time};
use xenonet_sources::lines::{opt_prop, prop, Js, TypeError};
use xenonet_sources::record::{excerpt, excerpt_json, nn, tpl, Obj, F};
use xenonet_sources::session::{CodexSession, Parent, ParentKind};

/// `Object.keys(v)[0]`: an object's first key, an array's first index.
fn first_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.keys().next().cloned(),
        Value::Array(a) if !a.is_empty() => Some("0".into()),
        Value::String(s) if !s.is_empty() => Some("0".into()),
        _ => None,
    }
}

pub(super) fn is_object(v: Js<'_>) -> bool {
    matches!(v, Some(Value::Object(_) | Value::Array(_)))
}

// When a thread was made or last ran, in ms: seconds when it's under 1e12,
// and a time written as a string read as one.
fn to_ms(t: Js<'_>) -> F {
    match t {
        None | Some(Value::Null) => F::null(),
        Some(Value::String(s)) => match time::parse_iso(s) {
            Some(ms) if ms != 0.0 => F::V(json::number(ms)),
            _ => F::null(),
        },
        Some(v) => {
            let n = js_number(Some(v));
            if n < 1e12 {
                let ms = n * 1000.0;
                if ms.is_finite() {
                    F::V(json::number(ms))
                } else {
                    F::Num(ms)
                }
            } else {
                F::V(v.clone())
            }
        }
    }
}

/// What kind of agent ran a thread (subAgentOf): a spawned sub-agent
/// records its parent and nickname; guardian checks, reviews and
/// compaction are background work.
pub fn sub_agent_of(t: Js<'_>) -> Result<(Value, Value, Value), TypeError> {
    let source = prop(t, "source")?;
    let sub = if truthy(source) && is_object(source) { prop(source, "subAgent")? } else { None };
    let spawn = if truthy(sub) && is_object(sub) { nn(prop(sub, "thread_spawn")?) } else { None };
    let kind = if truthy(spawn) {
        Value::String("spawned".into())
    } else {
        match sub {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(s)) => Value::String(s.clone()),
            Some(v) => match nn(prop(sub, "other")?) {
                Some(o) => o.clone(),
                None => Value::String(first_key(v).unwrap_or_else(|| "other".into())),
            },
        }
    };
    let parent = nn(opt_prop(spawn, "parent_thread_id")).cloned().unwrap_or(Value::Null);
    let nickname = nn(opt_prop(spawn, "agent_nickname")).cloned().unwrap_or(Value::Null);
    Ok((parent, nickname, kind))
}

/// A thread as a session (threadToSession).
pub fn thread_to_session(t: Js<'_>, archived: bool) -> Result<Obj, TypeError> {
    let get = |k: &str| prop(t, k);
    let status = match get("status")? {
        Some(Value::String(s)) => Value::String(s.clone()),
        st => nn(opt_prop(st, "type")).cloned().unwrap_or(Value::Null),
    };
    let (parent, nickname, kind) = sub_agent_of(t)?;
    let parent_thread = match nn(get("parentThreadId")?) {
        Some(p) => p.clone(),
        None => parent,
    };
    let preview = get("preview")?;
    let title = match get("name")? {
        Some(n) if truthy(Some(n)) => n.clone(),
        _ => {
            let first = match nn(preview) {
                None => None,
                Some(Value::String(p)) => Some(Value::String(p.split('\n').next().unwrap_or("").to_string())),
                Some(_) => return Err(TypeError("t.preview?.split is not a function".into())),
            };
            first.filter(|f| truthy(Some(f))).unwrap_or_else(|| Value::String("(untitled thread)".into()))
        }
    };
    let git = get("gitInfo")?;
    let origin = nn(opt_prop(git, "originUrl")).or(nn(opt_prop(git, "origin_url"))).cloned().unwrap_or(Value::Null);
    let source = get("source")?;
    let source_f = match source {
        Some(Value::String(s)) => F::s(s.clone()),
        Some(v) if truthy(Some(v)) => first_key(v).map_or(F::Undef, F::s),
        _ => F::null(),
    };
    let forked = get("forkedFromId")?;
    let parent = if truthy(forked) {
        Some(Parent { id: format!("codex:{}", tpl(forked)), kind: ParentKind::Fork })
    } else if truthy(Some(&parent_thread)) {
        Some(Parent { id: format!("codex:{}", js_string(&parent_thread)), kind: ParentKind::Subagent })
    } else {
        None
    };
    let nick = match nn(get("agentNickname")?) {
        Some(n) => n.clone(),
        None => nickname,
    };
    let background =
        source.and_then(Value::as_str) == Some("exec") || get("originator")?.and_then(Value::as_str) == Some("codex_exec") || (!kind.is_null() && kind.as_str() != Some("spawned"));
    let recency = nn(get("recencyAt")?).or(get("updatedAt")?);
    let or_null = |v: Js<'_>| nn(v).cloned().unwrap_or(Value::Null);
    Ok(CodexSession {
        raw_id: get("id")?.cloned(),
        title,
        preview: or_null(preview),
        cwd: or_null(get("cwd")?),
        created_at: to_ms(get("createdAt")?),
        updated_at: to_ms(recency),
        archived,
        status,
        model: or_null(get("model")?),
        provider: or_null(get("modelProvider")?),
        effort: or_null(get("reasoningEffort")?),
        branch: or_null(opt_prop(git, "branch")),
        origin,
        source: source_f,
        originator: or_null(get("originator")?),
        native_project_id: or_null(get("projectId")?),
        parent,
        agent_nickname: nick,
        agent_kind: kind,
        background,
    }
    .record())
}

// What a message's content says: its parts' text, an image as "[image]".
pub(super) fn text_of(content: Js<'_>) -> String {
    match content {
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for c in parts {
                let text = nn(opt_prop(Some(c), "text"));
                out.push(match text {
                    Some(t) => js_string(t),
                    None => {
                        let ty = opt_prop(Some(c), "type").and_then(Value::as_str);
                        (if matches!(ty, Some("image" | "localImage" | "local_image")) { "[image]" } else { "" }).to_string()
                    }
                });
            }
            out.join("\n")
        }
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

// [...a, ...b]: what spreading a value gives, or the TypeError JS throws.
pub(super) fn spread(v: Js<'_>, what: &str) -> Result<Vec<Value>, TypeError> {
    match nn(v) {
        None => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(s)) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        Some(_) => Err(TypeError(format!("{what} is not iterable"))),
    }
}

// Reasoning's parts joined: a string as it is, else its text.
pub(super) fn reasoning(parts: Vec<Value>, optional: bool) -> Result<String, TypeError> {
    let mut out = Vec::with_capacity(parts.len());
    for x in &parts {
        out.push(match x {
            Value::String(s) => s.clone(),
            _ => {
                let t = if optional { opt_prop(Some(x), "text") } else { prop(Some(x), "text")? };
                nn(t).map_or_else(String::new, js_string)
            }
        });
    }
    Ok(out.join("\n"))
}

pub(super) fn base(id: F, ts: F, kind: Js<'_>) -> Obj {
    vec![("id", id), ("ts", ts), ("kind", F::of(kind))]
}

pub(super) fn with(mut o: Obj, fields: Obj) -> Obj {
    o.extend(fields);
    o
}

// error ?? result?.content ?? result ?? arguments
pub(super) fn mcp_text(it: Js<'_>) -> Result<F, TypeError> {
    let result = prop(it, "result")?;
    let v = nn(prop(it, "error")?).or(nn(opt_prop(result, "content"))).or(nn(result)).or(prop(it, "arguments")?);
    Ok(excerpt_json(v, 4000))
}

/// An app-server's item as the console shows it (normalizeItem).
pub fn normalize_item(entry: Js<'_>) -> Result<Obj, TypeError> {
    static EMPTY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(Map::new()));
    let it = nn(prop(entry, "item")?).or(Some(&*EMPTY));
    let ts = nn(prop(entry, "startedAtMs")?).or(nn(prop(entry, "completedAtMs")?));
    let get = |k: &str| prop(it, k);
    let ty = get("type")?;
    let b = base(F::of(get("id")?), F::or_null(ts), ty);
    Ok(match ty.and_then(Value::as_str) {
        Some("userMessage") => with(b, vec![("role", F::s("user")), ("text", F::s(text_of(get("content")?)))]),
        Some("agentMessage") => with(b, vec![("role", F::s("assistant")), ("text", F::V(nn(get("text")?).cloned().unwrap_or(Value::String(String::new()))))]),
        Some("reasoning") => {
            let mut parts = spread(get("summary")?, "it.summary")?;
            parts.extend(spread(get("content")?, "it.content")?);
            with(b, vec![("role", F::s("reasoning")), ("text", F::s(reasoning(parts, false)?))])
        }
        Some("plan") => with(b, vec![("role", F::s("assistant")), ("title", F::s("plan")), ("text", F::V(nn(get("text")?).cloned().unwrap_or(Value::String(String::new()))))]),
        Some("commandExecution") => {
            let code = get("exitCode")?;
            let meta = nn(code).map_or(F::null(), |c| F::s(format!("exit {}", js_string(c))));
            with(
                b,
                vec![
                    ("role", F::s("tool")),
                    ("title", F::of(get("command")?)),
                    ("status", F::of(get("status")?)),
                    ("meta", meta),
                    ("text", excerpt(get("aggregatedOutput")?, 4000)),
                ],
            )
        }
        Some("mcpToolCall") => {
            let title = format!("{}.{}", tpl(get("server")?), tpl(get("tool")?));
            let text = mcp_text(it)?;
            with(b, vec![("role", F::s("tool")), ("title", F::s(title)), ("status", F::of(get("status")?)), ("text", text)])
        }
        Some("dynamicToolCall") => {
            with(b, vec![("role", F::s("tool")), ("title", F::of(get("tool")?)), ("status", F::of(get("status")?)), ("text", excerpt_json(get("arguments")?, 4000))])
        }
        Some("fileChange") => {
            let changes = get("changes")?;
            let n = match nn(changes) {
                None => F::V(json::number(0.0)),
                Some(Value::Array(a)) => F::V(json::number(a.len() as f64)),
                Some(_) => return Err(TypeError("(it.changes ?? []).map is not a function".into())),
            };
            let mut lines = Vec::new();
            for c in nn(changes).and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
                let kind = prop(Some(c), "kind")?;
                let k = nn(opt_prop(kind, "type")).or(nn(kind)).map_or_else(String::new, js_string);
                lines.push(format!("{k} {}", tpl(prop(Some(c), "path")?)));
            }
            let count = match n {
                F::V(v) => js_string(&v),
                _ => "0".into(),
            };
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("edit {count} file(s)"))), ("status", F::of(get("status")?)), ("text", F::s(lines.join("\n")))])
        }
        Some("webSearch") => {
            let q = nn(get("query")?).map_or_else(String::new, js_string);
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("search: {q}"))), ("text", F::s(""))])
        }
        Some("contextCompaction") => with(b, vec![("role", F::s("system")), ("text", F::s("context compacted"))]),
        _ => with(b, vec![("role", F::s("system")), ("title", F::of(ty)), ("text", excerpt_json(it, 1500))]),
    })
}

/// A page of the app-server's items, oldest first.
pub fn normalize_items(data: Js<'_>) -> Result<Vec<Obj>, TypeError> {
    let items = match nn(data) {
        None => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(TypeError("(page.data ?? []).map is not a function".into())),
    };
    let mut out = items.iter().map(|e| normalize_item(Some(e))).collect::<Result<Vec<_>, _>>()?;
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get<'a>(o: &'a Obj, k: &str) -> &'a F {
        &o.iter().find(|(n, _)| *n == k).unwrap().1
    }

    #[test]
    fn a_thread_is_a_session() {
        let t = json!({ "id": "t1", "preview": "Fix the build\nand more", "createdAt": 1790000000, "status": "active", "gitInfo": { "origin_url": "o" }, "source": "cli" });
        let s = thread_to_session(Some(&t), false).unwrap();
        assert_eq!(get(&s, "title"), &F::s("Fix the build"));
        assert_eq!(get(&s, "createdAt"), &F::V(json!(1790000000000u64)));
        assert_eq!(get(&s, "origin"), &F::s("o"));
        let spawned = json!({ "id": "t2", "source": { "subAgent": { "thread_spawn": { "parent_thread_id": "t1", "agent_nickname": "b" } } } });
        let s = thread_to_session(Some(&spawned), true).unwrap();
        assert_eq!([get(&s, "parentId"), get(&s, "agentKind"), get(&s, "source")], [&F::s("codex:t1"), &F::s("spawned"), &F::s("subAgent")]);
        assert_eq!(sub_agent_of(Some(&json!({ "source": { "subAgent": { "compact": {} } } }))).unwrap().2, json!("compact"));
        assert_eq!(sub_agent_of(Some(&json!({ "source": { "subAgent": 5 } }))).unwrap().2, json!("other"));
        assert!(thread_to_session(Some(&Value::Null), false).is_err());
    }

    #[test]
    fn reads_items() {
        let page = json!([{ "item": { "id": "i1", "type": "agentMessage", "text": "Hello" } }, { "item": { "id": "i0", "type": "userMessage", "content": [{ "type": "text", "text": "Hi" }, { "type": "localImage" }] }, "startedAtMs": 10 }]);
        let items = normalize_items(Some(&page)).unwrap();
        assert_eq!(get(&items[0], "text"), &F::s("Hi\n[image]"));
        assert_eq!(get(&items[0], "ts"), &F::V(json!(10)));
        let fc =
            normalize_item(Some(&json!({ "item": { "type": "fileChange", "changes": [{ "path": "a", "kind": { "type": "update" } }, { "path": "b", "kind": "add" }] } }))).unwrap();
        assert_eq!(get(&fc, "text"), &F::s("update a\nadd b"));
        assert_eq!(get(&fc, "id"), &F::Undef);
    }
}
