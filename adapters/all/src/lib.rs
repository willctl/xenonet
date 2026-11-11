//! Every adapter this XenoNet is built with, in the registry's order, and
//! putting them in place: the binary's main calls `install` before
//! anything asks the registry. `Registry` answers what the HTTP routes ask
//! of it (xenonet-api's `Apps`).

use serde_json::Value;
use std::sync::Arc;
use xenonet_adapter::apps;
use xenonet_adapter::Platform;

/// The adapters, made.
pub fn all() -> Vec<Arc<dyn Platform>> {
    vec![
        xenonet_claude_code::Adapter::platform(),
        xenonet_codex::Adapter::platform(),
        xenonet_grok_build::Adapter::platform(),
        xenonet_opencode::Adapter::platform(),
        xenonet_cursor::Adapter::platform(),
        xenonet_gemini_cli::Adapter::platform(),
        xenonet_copilot::Adapter::platform(),
        xenonet_hermes::Adapter::platform(),
        xenonet_grok_bot::Adapter::platform(),
        xenonet_muse::Adapter::platform(),
    ]
}

/// Puts every adapter in place (`xenonet_adapter::install`), and git's
/// reader of checkouts (`xenonet_adapter::repo`), once.
pub fn install() {
    xenonet_adapter::install(all());
    xenonet_git::install();
}

/// The registry as the routes ask it: which ids are sessions', the apps
/// XenoNet asks, and which app wrote a Claude Code hook.
#[derive(Clone, Copy, Debug, Default)]
pub struct Registry;

impl xenonet_api::cores::Apps for Registry {
    fn is_session_id(&self, id: &str) -> bool {
        apps::is_session_id(id)
    }

    fn session_id_of(&self, named: &str) -> String {
        xenonet_adapter::formats::opencode::xenonet_id(named)
    }

    fn asked(&self, what: &str, app: Option<&Value>, fallback: &'static str) -> Result<&'static str, String> {
        apps::asked_or(app, Some(fallback)).ok_or_else(|| apps::app_refusal_among(what, app.unwrap_or(&Value::Null), &apps::ASKED))
    }

    fn hook_written_by(&self, body: &Value) -> Option<&'static str> {
        apps::hook_written_by(body)
    }
}

/// The registry, for `Cores::apps`.
pub fn registry() -> Arc<dyn xenonet_api::cores::Apps> {
    Arc::new(Registry)
}
