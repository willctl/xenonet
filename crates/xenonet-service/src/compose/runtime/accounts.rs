//! Which folder of an app a run starts in, and whether it may: a project's
//! rule names the Claude Code or ChatGPT account its sessions are to run
//! as, and XenoNet reads several folders of an app, each signed in to
//! one. A start into a project whose rule names an account runs in the
//! folder that holds it, or is refused with words; never under another.
//! A session resumed, copied or woken runs in the folder it was found in.

use super::Runtime;
use serde_json::Value;

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
    let Some(wanted) = kind.and_then(|k| rule.get(k)).and_then(Value::as_str) else { return Ok(Placed::default()) };
    if confined {
        return Err(format!(
            "This project's rule says {app} runs as {wanted}. A run in a sandbox signs in with the token XenoNet's runs share, and XenoNet can't tell whose account that is, so it can't start work here that way. Start it without the sandbox, or change the rule."
        ));
    }
    let root = xenonet_state::identities::pick_root(app, roots, wanted)?;
    let deny = if rule.get("mode").and_then(Value::as_str) == Some("refuse") { refuse.to_vec() } else { Vec::new() };
    Ok(Placed { root, ruled: true, deny })
}

/// `place` for a start of `app` in `cwd`: the rule of the project the
/// folder is in, and the folders now read for the app.
pub fn place_start(w: &Runtime, app: &str, cwd: &str, confined: bool) -> Result<Placed, String> {
    let state = w.state();
    let found = state.identity_project(cwd).map_err(|e| e.message)?;
    let Some(rule) = found.get("rule").filter(|r| r.is_object()) else { return Ok(Placed::default()) };
    let Some(a) = xenonet_adapter::apps::app(app) else { return Ok(Placed::default()) };
    let refuse = xenonet_adapter::platforms().get(app).map(|p| p.refuse_args()).unwrap_or_default();
    place(&a.name, a.identity.as_deref(), rule, &state.app_account_roots(app), confined, &refuse)
}

/// The folder a session (`claude:<id>`) was found in, if she named it.
pub fn session_root(w: &Runtime, session: &str) -> Option<String> {
    w.state().session_root(session)
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
    fn a_confined_run_is_not_placed_under_an_account_it_cant_check() {
        let why = place("Claude Code", Some("claude"), &json!({ "claude": "nova@example.com" }), &roots(), true, &deny()).unwrap_err();
        assert!(why.contains("sandbox") && why.contains("nova@example.com"), "{why}");
        // No account in the rule, no objection.
        assert!(place("Claude Code", Some("claude"), &json!({ "git": "a@b.example" }), &roots(), true, &deny()).is_ok());
    }
}
