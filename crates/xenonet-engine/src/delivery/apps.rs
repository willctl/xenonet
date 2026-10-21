//! How live delivery reaches each app's sessions (core/delivery/apps.js):
//! the apps the registry gives `delivery`, each with its own ways in here,
//! by its id.

use crate::sql::Arg;
use std::sync::OnceLock;
use xenonet_base::apps::App;

/// The command for one headless turn of a session whose process has gone,
/// given the session's id and the settings file the turn listens with (if
/// any): the program and its arguments.
pub type Resume = fn(&str, &Arg) -> (String, Vec<Arg>);

/// An app's ways in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ways {
    /// Its headless resume, if it has one.
    pub resume: Option<Resume>,
    /// The start of the variables that mark a process as one of the app's
    /// sessions, which a resumed turn leaves out; where the app keeps its
    /// sessions (the registry's roots.env) stays.
    pub marks: Option<&'static str>,
    /// A session started with XenoNet as a channel takes messages pushed
    /// into it.
    pub channel: bool,
    /// The app's own server takes a message for a session: OpenCode's,
    /// which the router reaches through its `opencode`.
    pub server: bool,
}

// claude -p --resume <id> [--settings <listen file>]
fn claude_resume(id: &str, settings: &Arg) -> (String, Vec<Arg>) {
    let mut args = vec![Arg::text("-p"), Arg::text("--resume"), Arg::text(id)];
    if settings.truthy() {
        args.push(Arg::text("--settings"));
        args.push(settings.clone());
    }
    ("claude".into(), args)
}

// codex exec resume <id> -
fn codex_resume(id: &str, _: &Arg) -> (String, Vec<Arg>) {
    ("codex".into(), vec![Arg::text("exec"), Arg::text("resume"), Arg::text(id), Arg::text("-")])
}

/// An app's ways in, by its id, for the apps XenoNet has them for.
pub fn ways(id: &str) -> Option<Ways> {
    match id {
        "claude" => Some(Ways { resume: Some(claude_resume), marks: Some("CLAUDE"), channel: true, server: false }),
        "codex" => Some(Ways { resume: Some(codex_resume), marks: Some("CODEX_"), ..Ways::default() }),
        "opencode" => Some(Ways { server: true, ..Ways::default() }),
        _ => None,
    }
}

/// The apps live delivery reaches, in the registry's order, each with its
/// ways in. A test can give its own registry and ways.
pub fn delivery_apps<'a>(registry: &'a [App], ways: &dyn Fn(&str) -> Option<Ways>) -> Vec<(&'a App, Ways)> {
    registry.iter().filter(|a| a.has("delivery")).filter_map(|a| Some((a, ways(&a.id)?))).collect()
}

/// The registry's, read once.
pub fn apps() -> &'static [(&'static App, Ways)] {
    static A: OnceLock<Vec<(&'static App, Ways)>> = OnceLock::new();
    A.get_or_init(|| delivery_apps(xenonet_base::apps::apps(), &ways))
}

/// An app's ways in, by its id.
pub fn of(app: &str) -> Option<Ways> {
    apps().iter().find(|(a, _)| a.id == app).map(|(_, w)| *w)
}

/// Whether a session's id is an app's whose own server takes messages.
pub fn served(id: &str) -> bool {
    apps().iter().any(|(a, w)| w.server && id.strip_prefix(a.id.as_str()).is_some_and(|rest| rest.starts_with(':')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_delivery_reaches_and_how() {
        let ids: Vec<&str> = apps().iter().map(|(a, _)| a.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex", "opencode"]);
        assert!(of("claude").unwrap().channel && !of("codex").unwrap().channel);
        assert!(of("opencode").unwrap().server && of("opencode").unwrap().resume.is_none());
        assert!(of("gemini-cli").is_none());
        assert!(served("opencode:ses-1") && !served("opencode") && !served("opencodes:x") && !served("claude:x"));
    }
}
