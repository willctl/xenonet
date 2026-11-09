//! The actor products XenoNet knows (docs/design/actors/interface.md,
//! section 7): the actor each one is
//! (`slug`), what it's called, who makes it, whether XenoNet observes it,
//! whether its actor may report itself (register), and whether you may pair
//! it as a remote actor. A product's slug, and the name it goes by, are that
//! product's alone.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub id: &'static str,
    pub slug: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub vendor: &'static str,
    pub observed: bool,
    pub registers: bool,
    pub remote: bool,
}

pub const PRODUCTS: [Product; 4] = [
    Product { id: "hermes-agent", slug: "hermes", name: "Hermes Agent", short: "Hermes", vendor: "Nous Research", observed: true, registers: true, remote: true },
    Product { id: "grok-bot", slug: "grok-bot", name: "Grok Bot", short: "Grok Bot", vendor: "SpaceXAI", observed: true, registers: true, remote: true },
    Product { id: "muse", slug: "muse", name: "Muse", short: "Muse", vendor: "Meta", observed: true, registers: true, remote: false },
    Product { id: "dots", slug: "dots", name: "dots", short: "dots", vendor: "OpenAI", observed: false, registers: false, remote: true },
];

/// A product by its id, or None.
pub fn product_of(id: &str) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.id == id)
}

/// Why a product's actor can't report itself, for those that can't.
pub const WHY_NOT: [(&str, &str); 1] = [("dots", "OpenAI's dots don't report themselves to XenoNet")];

/// Whether a slug is a product's own: hermes and `hermes-<profile>` are
/// Hermes's (one actor per profile), dots and `dots-<suffix>` OpenAI's dots
/// (one actor per dot you pair), the others their own slug.
pub fn product_of_slug(slug: &str) -> Option<&'static str> {
    if slug == "hermes" || slug.starts_with("hermes-") {
        return Some("hermes-agent");
    }
    if slug == "dots" || slug.starts_with("dots-") {
        return Some("dots");
    }
    PRODUCTS.iter().find(|p| p.slug == slug).map(|p| p.id)
}

// ---------- how a product's processes are told apart ----------
//
// What the engine's process checks need to know of each product: the team
// that signs its app, the bundle its app lives in and that bundle's id,
// the programs that run its agent, and how a check's reason names it.
// Nothing of this is in the engine, so a product is added or changed here
// alone.

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($p).expect("a pattern that compiles"))
    }};
}

/// How a check's reason names a product's processes: the product, or for
/// dots the ChatGPT app they run in.
pub fn called(id: &str) -> Option<&'static str> {
    match id {
        "dots" => Some("the ChatGPT app"),
        other => product_of(other).map(|p| p.name),
    }
}

/// The maker who signs a product's app, for the products whose makers do:
/// Grok Bot is Cursor's, Muse Meta's. Hermes's Python is signed ad hoc, and
/// dots run inside the ChatGPT app, so neither has one.
pub fn signer(id: &str) -> Option<&'static str> {
    match id {
        "grok-bot" => Some("Cursor"),
        "muse" => Some("Meta"),
        _ => None,
    }
}

/// The developer team that signs a product's app (codesign's
/// TeamIdentifier), for the products `signer` names: Cursor (Anysphere)
/// and Meta.
pub fn team(id: &str) -> Option<&'static str> {
    match id {
        "grok-bot" => Some("DCNK4UB866"),
        "muse" => Some("V9WTTPBFK9"),
        _ => None,
    }
}

/// The role of the product's processes an actor's request has to come from
/// under: one of Hermes's agent hosts, and for the others the product's
/// app.
pub fn own_role(id: &str) -> &'static str {
    if id == "hermes-agent" {
        "host"
    } else {
        "app"
    }
}

/// The product whose app a bundle of this name is: Hermes.app (the desktop
/// app, and its setup app), Grok Bot, Muse, and the ChatGPT app, where
/// OpenAI's dots run. An app's own processes could be relaying for its
/// cloud.
pub fn app_of_bundle(name: &str) -> Option<&'static str> {
    match name {
        "Hermes" => Some("hermes-agent"),
        "Grok Bot" => Some("grok-bot"),
        "Muse" => Some("muse"),
        "ChatGPT" => Some("dots"),
        _ => None,
    }
}

/// Whether a Python at this path (with / for \) is one a product runs its
/// agent with: Hermes's own folder (the managed Python, its venv) or the
/// inside of Hermes.app.
pub fn hosts_python(path: &str) -> bool {
    path.contains("/.hermes/") || path.contains("/Hermes.app/")
}

/// The product whose agent host a Python at `path` running `args` is:
/// Hermes's managed Python running its CLI, its terminal UI's backend or
/// its agent.
pub fn python_host(path: &str, args: &str) -> Option<&'static str> {
    (hosts_python(path) && re!(r"(?-u:\b)(hermes_cli|tui_gateway|run_agent)(?-u:\b)").is_match(args)).then_some("hermes-agent")
}

/// The product whose terminal UI Node running `args` is: Hermes's, whose
/// child is its agent host.
pub fn node_tui(args: &str) -> Option<&'static str> {
    re!(r"/(ui-tui/dist|hermes_cli/tui_dist)/entry\.js(?-u:\b)").is_match(args).then_some("hermes-agent")
}

/// The products whose app bundles are vouched for by their Info.plist: a
/// Hermes.app is Hermes's, a Grok Bot.app Grok Bot's and a Muse.app Meta's
/// Muse only when its bundle id says so, since other makers' apps can have
/// the same names (there are other Mac apps called Muse), and anyone can
/// name a bundle Grok Bot.app.
pub const VOUCHED: [&str; 3] = ["hermes-agent", "grok-bot", "muse"];

/// The outermost bundle of the product's name a program's path lies in,
/// for a vouched product, or None.
pub fn bundle_of(product: &str, program: &str) -> Option<String> {
    let r = match product {
        "hermes-agent" => re!(r"^(.*?/Hermes\.app)/"),
        "grok-bot" => re!(r"^(.*?/Grok Bot\.app)/"),
        "muse" => re!(r"^(.*?/Muse\.app)/"),
        _ => return None,
    };
    r.captures(program).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

/// The bundle of a vouched product's name a program lies in, or None.
pub fn vouched_bundle(program: &str) -> Option<String> {
    VOUCHED.iter().find_map(|v| bundle_of(v, program))
}

/// Whether a bundle id is the product's own.
pub fn vouched_id(product: &str, id: &str) -> bool {
    match product {
        "hermes-agent" => re!(r"^com\.nousresearch\.hermes(\.|$)").is_match(id),
        "grok-bot" => re!(r"^com\.anysphere\.sand(\.|$)").is_match(id),
        "muse" => re!(r"^com\.meta\.endo(\.|$)").is_match(id),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_products_processes_are_told_by_its_paths_and_arguments() {
        let py = "/Users/nova/.hermes/tools/python-3.14.7/bin/python3.14";
        assert_eq!(python_host(py, "-m tui_gateway.entry"), Some("hermes-agent"));
        assert_eq!(python_host(py, "-m pip install requests"), None);
        assert_eq!(python_host("/opt/homebrew/bin/python3", "-m run_agent"), None, "a Python of the system's");
        assert_eq!(node_tui("node /opt/hermes/ui-tui/dist/entry.js"), Some("hermes-agent"));
        assert_eq!(node_tui("node /x/entry.js"), None);
        assert_eq!(app_of_bundle("Grok Bot"), Some("grok-bot"));
        assert_eq!(app_of_bundle("XenoNet"), None);
        assert_eq!(bundle_of("grok-bot", "/Applications/Grok Bot.app/Contents/MacOS/Grok Bot").as_deref(), Some("/Applications/Grok Bot.app"));
        assert_eq!(vouched_bundle("/Users/me/Hermes.app/Contents/Resources/Hermes.app/x").as_deref(), Some("/Users/me/Hermes.app"), "the outermost");
        assert_eq!(vouched_bundle("/Applications/XenoNet.app/Contents/MacOS/XenoNet"), None);
        assert!(vouched_id("muse", "com.meta.endo.helper") && !vouched_id("muse", "com.meta.endosome"));
        assert_eq!((called("dots"), called("grok-bot"), called("nobody")), (Some("the ChatGPT app"), Some("Grok Bot"), None));
        assert_eq!((signer("grok-bot"), team("muse"), signer("hermes-agent")), (Some("Cursor"), Some("V9WTTPBFK9"), None));
        assert_eq!((own_role("hermes-agent"), own_role("muse")), ("host", "app"));
    }
}
