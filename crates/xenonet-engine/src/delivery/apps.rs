//! How live delivery reaches each app's sessions: the apps the registry
//! gives `delivery`, each with the ways in its adapter brings
//! (`xenonet_adapter::delivery::Ways`).

use xenonet_adapter::apps::App;

pub use xenonet_adapter::delivery::{Resume, Ways};

/// The apps live delivery reaches, in the registry's order, each with its
/// ways in. A test can give its own registry and ways.
pub fn delivery_apps<'a>(registry: &'a [App], ways: &dyn Fn(&str) -> Option<Ways>) -> Vec<(&'a App, Ways)> {
    registry.iter().filter(|a| a.has("delivery")).filter_map(|a| Some((a, ways(&a.id)?))).collect()
}

/// The adapters' ways in, by app id.
fn installed(id: &str) -> Option<Ways> {
    xenonet_adapter::platforms().get(id).and_then(|p| p.delivery())
}

/// The registry's apps with `delivery` whose adapter is in place, with
/// its ways in.
pub fn apps() -> Vec<(&'static App, Ways)> {
    delivery_apps(xenonet_adapter::apps::apps(), &installed)
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
        xenonet_platforms::install();
        let ids: Vec<&str> = apps().iter().map(|(a, _)| a.id.as_str()).collect();
        assert_eq!(ids, ["claude", "codex", "grok", "opencode"]);
        assert!(of("claude").unwrap().channel && !of("codex").unwrap().channel);
        assert!(of("opencode").unwrap().server && of("opencode").unwrap().resume.is_none());
        assert!(of("gemini-cli").is_none());
        assert!(served("opencode:ses-1") && !served("opencode") && !served("opencodes:x") && !served("claude:x"));
    }
}
