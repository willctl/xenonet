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
    /// One actor per profile of the product's: its slug, or
    /// `<slug>-<profile>`.
    pub profiles: bool,
    /// One actor per pairing: its slug, then `<slug>-<suffix>` for each
    /// one paired after.
    pub per_pair: bool,
    /// Who makes it, its vendor first: no actor of another product may
    /// pass for one of them.
    pub makers: &'static [&'static str],
    /// Its actor reports what it spent, which the spend view shows under
    /// the product's short name.
    pub reports_spend: bool,
}

pub const PRODUCTS: [Product; 4] = [
    Product {
        id: "hermes-agent",
        slug: "hermes",
        name: "Hermes Agent",
        short: "Hermes",
        vendor: "Nous Research",
        observed: true,
        registers: true,
        remote: true,
        profiles: true,
        per_pair: false,
        makers: &["Nous Research"],
        reports_spend: true,
    },
    // Anysphere (Cursor) builds and runs Grok Bot for SpaceXAI.
    Product {
        id: "grok-bot",
        slug: "grok-bot",
        name: "Grok Bot",
        short: "Grok Bot",
        vendor: "SpaceXAI",
        observed: true,
        registers: true,
        remote: true,
        profiles: false,
        per_pair: false,
        makers: &["SpaceXAI", "xAI", "Anysphere", "Cursor"],
        reports_spend: false,
    },
    Product {
        id: "muse",
        slug: "muse",
        name: "Muse",
        short: "Muse",
        vendor: "Meta",
        observed: true,
        registers: true,
        remote: false,
        profiles: false,
        per_pair: false,
        makers: &["Meta"],
        reports_spend: false,
    },
    Product {
        id: "dots",
        slug: "dots",
        name: "dots",
        short: "dots",
        vendor: "OpenAI",
        observed: false,
        registers: false,
        remote: true,
        profiles: false,
        per_pair: true,
        makers: &["OpenAI"],
        reports_spend: false,
    },
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
    let suffixed = |p: &Product| (p.profiles || p.per_pair) && slug.strip_prefix(p.slug).is_some_and(|rest| rest.starts_with('-'));
    PRODUCTS.iter().find(|p| p.slug == slug || suffixed(p)).map(|p| p.id)
}

// ---------- pairing a product as a remote actor ----------

/// How a product you may pair as a remote actor (`remote`) reaches
/// XenoNet: its remote adapter's id, its provider, the transports it comes
/// through, how far that's tested ("experimental": its route is meant for
/// real use and untested end to end; "fixture": only its common code is
/// tested), why neither the account nor the actor can be verified through
/// it, and what it may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub product: &'static str,
    pub adapter: &'static str,
    pub provider: &'static str,
    pub transports: &'static [&'static str],
    pub support: &'static str,
    pub unavailable: &'static str,
    pub capabilities: &'static [&'static str],
}

/// The products' pairings, in the order the pairing dialog lists them.
pub const PAIRINGS: [Pairing; 3] = [
    // The tunnel forwards an MCP call to the local adapter; it passes no
    // assertion of the ChatGPT user or the dot that XenoNet could validate.
    Pairing {
        product: "dots",
        adapter: "openai-dots",
        provider: "openai",
        transports: &["openai-secure-mcp-tunnel", "local-stdio"],
        support: "experimental",
        unavailable: "transport-has-no-assertion",
        capabilities: &["read"],
    },
    Pairing {
        product: "grok-bot",
        adapter: "grok-bot",
        provider: "spacexai",
        transports: &["local-stdio"],
        support: "fixture",
        unavailable: "no-supported-route",
        capabilities: &["read"],
    },
    Pairing {
        product: "hermes-agent",
        adapter: "hermes-agent",
        provider: "nous",
        transports: &["local-stdio"],
        support: "fixture",
        unavailable: "no-supported-route",
        capabilities: &["read"],
    },
];

// ---------- handing a product's agent the actor gateway ----------

/// What a product's agent is given to reach XenoNet's actor gateway, for a
/// new token (`token`) or a new address alone (`address`): the steps, each
/// with what it copies ("message", "token", "url", "wake" or None), and
/// the message to paste into its chat, where `{url}` is the gateway's
/// address and `{host}` its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handoff {
    pub product: &'static str,
    pub token: Steps,
    pub address: Steps,
    /// How to give XenoNet a way to wake its agent, when there's one: the
    /// steps, and the message to paste.
    pub wake: Option<([&'static str; 2], &'static str)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Steps {
    pub steps: &'static [(&'static str, Option<&'static str>)],
    pub message: Option<&'static str>,
}

pub const HANDOFFS: [Handoff; 2] = [
    // The token goes into Grok Bot's secure secret card, which its model
    // never sees, never into its chat. Its wake is a routine that starts
    // from a webhook, made from its chat, whose URL and key go into
    // XenoNet's window.
    Handoff {
        product: "grok-bot",
        token: Steps {
            steps: &[
                ("Paste this into a Grok Bot chat.", Some("message")),
                ("When Grok Bot asks for the token, paste it into its secret card, not the chat, and save it.", Some("token")),
                ("For answers within seconds, give XenoNet a way to wake it.", Some("wake")),
            ],
            message: Some("Add an MCP server named \"xenonet\" by URL: {url}. Ask me for its token with your secure secret card, named XENONET_GATEWAY_TOKEN, never in chat, and send it in the header Authorization: Bearer <the token>. Then call its whoami tool and tell me what it says."),
        },
        address: Steps {
            steps: &[("Paste this into a Grok Bot chat.", Some("message"))],
            message: Some("XenoNet's address changed. Change the URL of your MCP server \"xenonet\" to {url} and keep its Authorization header as it is. Then call its whoami tool and tell me what it says."),
        },
        wake: Some((
            ["Paste this into a Grok Bot chat to make the routine.", "Open the routine's settings and copy its Webhook URL and Webhook key below."],
            "Create a routine named \"XenoNet inbox\" that starts from a webhook. Its instruction: read your XenoNet inbox with your xenonet tools and answer anything that needs an answer. Tell me when it's made.",
        )),
    },
    Handoff {
        product: "muse",
        token: Steps {
            steps: &[("Paste this into a Muse chat, then approve the connector card.", Some("message")), ("When Muse asks for the key, paste the token into the card, not the chat.", Some("token"))],
            message: Some("Create a custom connector named \"XenoNet\" that uses an API key, for the host {host}, sending the key as a bearer token in the Authorization header. I'll paste the key into the key field when you ask. Then call {url} with JSON-RPC (method tools/call, tool whoami) and tell me what it says."),
        },
        address: Steps {
            steps: &[("Paste this into a Muse chat.", Some("message")), ("If Muse can't change the connector, reconnect Muse for a new token.", None)],
            message: Some("XenoNet's address changed to {url}. Change your \"XenoNet\" connector's host to {host} and keep its key. Then call whoami at the new address and tell me what it says."),
        },
        wake: None,
    },
];

/// The handoff of a product, by its id.
pub fn handoff_of(id: &str) -> Option<&'static Handoff> {
    HANDOFFS.iter().find(|h| h.product == id)
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

    #[test]
    fn a_slug_is_its_products_own() {
        for (slug, product) in [
            ("hermes", Some("hermes-agent")),
            ("hermes-work", Some("hermes-agent")),
            ("hermesx", None),
            ("dots", Some("dots")),
            ("dots-1a2b", Some("dots")),
            ("grok-bot", Some("grok-bot")),
            ("grok-bot-2", None),
            ("muse", Some("muse")),
            ("muse-x", None),
            ("wren", None),
        ] {
            assert_eq!(product_of_slug(slug), product, "{slug}");
        }
        // The products you may pair, and only those, have a pairing.
        let paired: Vec<&str> = PAIRINGS.iter().map(|p| p.product).collect();
        assert!(PRODUCTS.iter().all(|p| p.remote == paired.contains(&p.id)));
        assert!(HANDOFFS.iter().all(|h| product_of(h.product).is_some()) && PRODUCTS.iter().all(|p| p.makers.first() == Some(&p.vendor)));
    }
}
