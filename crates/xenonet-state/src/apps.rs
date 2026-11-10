//! What the map asks of the apps through the adapters in place, where more
//! than one of its modules asks it.

use std::sync::OnceLock;

/// The tailed app `app` is read as: itself when XenoNet tails it, else the
/// one whose reader reads the apps no reader is for (Claude Code's); None
/// while no adapter's reader does.
pub fn read_as(app: &str) -> Option<&'static str> {
    xenonet_adapter::apps::tailed_or_text(Some(app), None).or_else(untailed_reader)
}

// The app whose reader reads the others', found once the adapters are in
// place.
fn untailed_reader() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    let platforms = xenonet_adapter::platforms();
    platforms.iter().next()?;
    *FOUND.get_or_init(|| platforms.tailed().iter().find(|t| t.reads_untailed()).map(|t| t.id()))
}

/// The app Anthropic's plan usage is of: the one whose products include
/// Anthropic's OAuth API, which the reading comes from.
pub fn anthropic_usage_app() -> &'static str {
    xenonet_adapter::apps::apps().iter().find(|a| a.products.iter().any(|p| p == "anthropic/oauth")).map_or("", |a| a.id.as_str())
}

/// The name a session's app goes by, as the read-as app has it: Claude
/// Code's for an app XenoNet doesn't tail.
pub fn read_as_name(app: Option<&str>) -> &'static str {
    read_as(app.unwrap_or_default()).and_then(xenonet_adapter::apps::app).map_or("", |a| a.name.as_str())
}
