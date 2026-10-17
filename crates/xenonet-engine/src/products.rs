//! The actor products XenoNet knows (core/actors/products.js;
//! docs/design/actors/interface.md, section 7): the actor each one is
//! (`slug`), what it's called, who makes it, whether XenoNet observes it,
//! whether its actor may report itself (register), and whether you may pair
//! it as a remote actor. A product's slug, and the name it goes by, are that
//! product's alone. Here, below the actors, because the engine names them
//! too.

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

/// Whether a slug is a product's own: hermes and hermes-<profile> are
/// Hermes's (one actor per profile), dots and dots-<suffix> OpenAI's dots
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
