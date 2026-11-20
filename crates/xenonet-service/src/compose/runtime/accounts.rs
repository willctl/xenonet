//! Which folder of an app a run starts in, and whether it may: a project's
//! rule names the Claude Code or ChatGPT account its sessions are to run
//! as, and XenoNet reads several folders of an app, each signed in to
//! one. A start into a project whose rule names an account runs in the
//! folder that holds it, or is refused with words; never under another.
//! A session resumed, copied or woken runs in the folder it was found in,
//! and as its start was placed: the start's placement is kept in the store
//! under the session (`keep`), carried to its copies, and applied to every
//! later turn (`for_turn`) after the rule is read again.

use super::Runtime;
use serde_json::{json, Value};

/// Where a start runs, and what it must not do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placed {
    /// The folder she named that holds the account (CLAUDE_CONFIG_DIR,
    /// CODEX_HOME), or None for the app's own.
    pub root: Option<String>,
    /// Whether a rule named the account the run is to sign in as. Such a
    /// run signs in with its folder's own sign-in, never the token
    /// XenoNet's runs share, whose account XenoNet can't tell.
    pub ruled: bool,
    /// What the app is told to keep the run from, when the rule refuses.
    pub deny: Vec<String>,
}

/// The placing, from a project's `rule`, the kind of account the app runs
/// under, each folder's sign-in (`roots`: None for the app's own folder,
/// with the email it holds) and whether the run is confined. `refuse` is
/// the arguments that keep the run from committing and pushing.
pub fn place(app: &str, kind: Option<&str>, rule: &Value, roots: &[(Option<String>, Option<String>)], confined: bool, refuse: &[String]) -> Result<Placed, String> {
    // A rule that refuses keeps the run from committing and pushing whether
    // or not it names an account.
    let deny = if rule.get("mode").and_then(Value::as_str) == Some("refuse") { refuse.to_vec() } else { Vec::new() };
    let Some(wanted) = kind.and_then(|k| rule.get(k)).and_then(Value::as_str) else { return Ok(Placed { deny, ..Placed::default() }) };
    if confined {
        return Err(format!(
            "This project's rule says {app} runs as {wanted}. A run in a sandbox signs in with the token XenoNet's runs share, and XenoNet can't tell whose account that is, so it can't start work here that way. Start it without the sandbox, or change the rule."
        ));
    }
    let root = xenonet_state::identities::pick_root(app, roots, wanted)?;
    Ok(Placed { root, ruled: true, deny })
}

/// `place` for a start of `app` in `cwd`: the rule of the project the
/// folder is in, and the folders now read for the app.
pub fn place_start(w: &Runtime, app: &str, cwd: &str, confined: bool) -> Result<Placed, String> {
    let state = w.state();
    let found = match state.identity_project(cwd) {
        Ok(f) => f,
        // A state that isn't wired (a test's, a bare service) has no rules.
        Err(e) if e.is_pending() => return Ok(Placed::default()),
        // Whether a rule covers the folder can't be told: a run that might
        // be ruled isn't made.
        Err(e) => return Err(format!("Whether a rule says which account {app} runs as in this folder can't be read ({}), so no run is made.", e.message)),
    };
    let Some(rule) = found.get("rule").filter(|r| r.is_object()) else { return Ok(Placed::default()) };
    let Some(a) = xenonet_adapter::apps::app(app) else { return Ok(Placed::default()) };
    let refuse = xenonet_adapter::platforms().get(app).map(|p| p.refuse_args()).unwrap_or_default();
    place(&a.name, a.identity.as_deref(), rule, &state.app_account_roots(app), confined, &refuse)
}

/// The run's sign-in, as `starts::signed_env` makes it, except for a run
/// that is placed by an account (`ruled`) or a session of a folder she named
/// (`rooted`): that run signs in with its folder's own sign-in, so it gets
/// no token, and none it inherited either, and none of the variables that
/// sign an app in some other way and outrank the folder's (a provider's API
/// key, a base URL).
pub fn sign_in(w: &Runtime, app: &str, s: Option<xenonet_adapter::delivery::SignIn>, own_folder: bool, env: &mut Vec<(String, String)>) -> Option<String> {
    if own_folder {
        xenonet_engine::delivery::resume::without_sign_in_vars(env, app);
    }
    let s = s?;
    if own_folder {
        env.retain(|(k, _)| k != s.variable && Some(k.as_str()) != s.fd_variable);
        return None;
    }
    super::starts::signed_env(w, &s, env)
}

/// The folder a session (`claude:<id>`) was found in, if she named it. A
/// folder that isn't one she named for the app is no folder to run in, and
/// neither is none that can be told: an id found in two folders.
pub fn session_root(w: &Runtime, app: &str, session: &str) -> Result<Option<String>, String> {
    let state = w.state();
    let Some(root) = state.session_root(session).map_err(|e| e.message)? else { return Ok(None) };
    if state.app_account_roots(app).iter().any(|(r, _)| r.as_deref() == Some(root.as_str())) {
        return Ok(Some(root));
    }
    Err(format!("{session} was found in {root}, which isn't a folder you named for {app}, so no turn is run for it."))
}

// ---------- what a start recorded, for every later turn ----------

/// The store's key for the placement of a session's start.
fn key(session: &str) -> String {
    format!("placed:{session}")
}

/// Keeps how `session` was placed, so every later turn of it (a wake, a copy)
/// is placed the same way. A placement that places nothing is not kept; one
/// that can't be kept refuses the run, since a turn later would have nothing
/// to hold it.
pub fn keep(w: &Runtime, session: &str, p: &Placed) -> Result<(), String> {
    if *p == Placed::default() {
        return Ok(());
    }
    let v = json!({ "root": p.root, "ruled": p.ruled, "deny": p.deny });
    w.store
        .0
        .db()
        .set_setting(&key(session), Some(&v))
        .map_err(|e| format!("where {session} runs couldn't be kept in the store ({}), and a session that can't be held to its account later isn't started", e.message))
}

/// How `session` was placed by its start, or by the session it is a copy of;
/// None for one that was placed nowhere. Err when the store can't say or
/// holds something that isn't a placement: such a session is not run.
pub fn of(w: &Runtime, session: &str) -> Result<Option<Placed>, String> {
    let read = |s: &str| -> Result<Option<Placed>, String> {
        let unreadable = |why: &str| format!("where {s} was placed to run can't be read ({why}), so no turn is run for it");
        let v = w.store.0.db().setting(&key(s)).map_err(|e| unreadable(&e.message))?;
        if v.is_null() {
            return Ok(None);
        }
        let root = match v.get("root") {
            Some(Value::Null) => None,
            Some(Value::String(r)) => Some(r.clone()),
            _ => return Err(unreadable("its folder isn't a folder")),
        };
        let ruled = v.get("ruled").and_then(Value::as_bool).ok_or_else(|| unreadable("it doesn't say whether a rule placed it"))?;
        let deny = v
            .get("deny")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(|d| d.as_str().map(str::to_string)).collect::<Option<Vec<_>>>())
            .ok_or_else(|| unreadable("its arguments aren't a list"))?;
        Ok(Some(Placed { root, ruled, deny }))
    };
    match read(session)? {
        Some(p) => Ok(Some(p)),
        None => match crate::compose::runtime::starts::fork_root(w, session) {
            Some(parent) => read(&parent),
            None => Ok(None),
        },
    }
}

/// A copy is placed as the turn that made it was, in its own right, so it
/// stays so after a restart forgets whose copy it is.
pub fn fork_named(w: &Runtime, child: &str, p: &Placed) -> Result<(), String> {
    keep(w, child, p)
}

/// How a turn for `session` of `app` in `cwd` is placed: as its start was,
/// with the rule of the project it is in read again, and the folder it was
/// found in. Or why it is not run: the rule now names another account than
/// the one it runs as, its record can't be read, it ran in a folder that is
/// not one she named, or it is confined (a sandboxed run signs in with
/// XenoNet's shared token and stays in the app's own folder).
pub fn for_turn(w: &Runtime, app: &str, session: &str, cwd: Option<&str>, confined: bool) -> Result<Placed, String> {
    let recorded = of(w, session)?;
    let found = session_root(w, app, session)?;
    // Where the start ran is where it stays; the map finding it elsewhere
    // is a session that isn't the one that was placed.
    if let (Some(f), Some(r)) = (&found, &recorded) {
        if r.root.as_ref() != Some(f) {
            return Err(format!("{session} was placed to run in {} but is found in {f}, so no turn is run for it.", r.root.as_deref().unwrap_or("the app's own folder")));
        }
    }
    let root = match &recorded {
        Some(r) => r.root.clone(),
        None => found,
    };
    if confined && root.is_some() {
        return Err(format!(
            "{session} runs in {}, signed in on its own. A run in a sandbox signs in with the token XenoNet's runs share, so it can't be run in that folder that way.",
            root.as_deref().unwrap_or_default()
        ));
    }
    let now = match cwd.filter(|c| !c.is_empty()) {
        Some(c) => place_start(w, app, c, confined)?,
        None => Placed::default(),
    };
    if now.ruled && now.root != root {
        return Err(format!(
            "This project's rule now says {app} runs in {}, but {session} runs in {}, so no turn is run for it. Change the rule back, or start a new session.",
            now.root.as_deref().unwrap_or("the app's own folder"),
            root.as_deref().unwrap_or("the app's own folder")
        ));
    }
    let mut deny = recorded.as_ref().map(|r| r.deny.clone()).unwrap_or_default();
    for d in now.deny {
        if !deny.contains(&d) {
            deny.push(d);
        }
    }
    let ruled = now.ruled || recorded.as_ref().is_some_and(|r| r.ruled);
    Ok(Placed { root, ruled, deny })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roots() -> Vec<(Option<String>, Option<String>)> {
        vec![(None, Some("nova@example.com".into())), (Some("/h/.claude-work".into()), Some("nova-work@example.com".into()))]
    }

    fn deny() -> Vec<String> {
        vec!["--disallowedTools".into(), "Bash(git commit:*)".into()]
    }

    #[test]
    fn a_start_runs_in_the_folder_that_holds_the_account() {
        xenonet_platforms::install();
        let rule = json!({ "claude": "nova-work@example.com" });
        let p = place("Claude Code", Some("claude"), &rule, &roots(), false, &deny()).unwrap();
        assert_eq!(p, Placed { root: Some("/h/.claude-work".into()), ruled: true, deny: Vec::new() });
        // The default folder when that holds it.
        let p = place("Claude Code", Some("claude"), &json!({ "claude": "nova@example.com" }), &roots(), false, &deny()).unwrap();
        assert_eq!((p.root, p.ruled), (None, true));
    }

    #[test]
    fn no_folder_holding_it_is_a_refusal_in_every_mode() {
        for mode in ["warn", "refuse"] {
            let rule = json!({ "claude": "ada@acme.example", "mode": mode });
            let why = place("Claude Code", Some("claude"), &rule, &roots(), false, &deny()).unwrap_err();
            assert!(why.contains("ada@acme.example") && why.contains("no folder"), "{why}");
        }
    }

    #[test]
    fn a_rule_without_the_account_places_nothing() {
        let rule = json!({ "git": "nova@example.com", "claude": "x@y.example" });
        assert_eq!(place("Codex", Some("chatgpt"), &rule, &roots(), false, &deny()).unwrap(), Placed::default());
        assert_eq!(place("Hermes", None, &rule, &roots(), false, &deny()).unwrap(), Placed::default());
    }

    #[test]
    fn a_refuse_rule_keeps_the_run_from_committing_and_pushing() {
        let rule = json!({ "claude": "nova@example.com", "mode": "refuse" });
        assert_eq!(place("Claude Code", Some("claude"), &rule, &roots(), false, &deny()).unwrap().deny, deny());
    }

    #[test]
    fn a_refuse_rule_that_names_no_account_still_keeps_the_run_from_committing() {
        let rule = json!({ "git": "nova@example.com", "mode": "refuse" });
        let p = place("Claude Code", Some("claude"), &rule, &roots(), false, &deny()).unwrap();
        assert_eq!(p, Placed { root: None, ruled: false, deny: deny() });
        // An app with no account kind too, and a warning rule keeps nothing.
        assert_eq!(place("Hermes", None, &rule, &roots(), false, &deny()).unwrap().deny, deny());
        assert_eq!(place("Claude Code", Some("claude"), &json!({ "git": "nova@example.com", "mode": "warn" }), &roots(), false, &deny()).unwrap(), Placed::default());
    }

    #[test]
    fn a_confined_run_is_not_placed_under_an_account_it_cant_check() {
        let why = place("Claude Code", Some("claude"), &json!({ "claude": "nova@example.com" }), &roots(), true, &deny()).unwrap_err();
        assert!(why.contains("sandbox") && why.contains("nova@example.com"), "{why}");
        // No account in the rule, no objection.
        assert!(place("Claude Code", Some("claude"), &json!({ "git": "a@b.example" }), &roots(), true, &deny()).is_ok());
    }
}
