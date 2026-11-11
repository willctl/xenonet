//! An outside agent's Agent Card: fetched once when the person registers
//! the agent, pinned by a hash of what it says, and checked against that
//! pin before every call.
//!
//! The hash is SHA-256 over the card's JSON written canonically (object
//! keys sorted, no white space), so a server that writes the same card
//! with its keys in another order keeps its pin, and any change to what
//! it says, a field XenoNet doesn't read included, needs the person again.

use crate::guard::{self, Private};
use crate::A2aError;
use ::a2a::{AgentCard, TRANSPORT_PROTOCOL_JSONRPC};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::time::Duration;
use url::Url;

/// The most of a card XenoNet reads.
pub const CARD_MAX: usize = 64 << 10;
/// How long fetching a card may take.
pub const FETCH: Duration = Duration::from_secs(15);
/// Where a card is, under an agent's origin (A2A 1.0, section 8.2).
pub const WELL_KNOWN: &str = "/.well-known/agent-card.json";

/// The JSON-RPC interface XenoNet calls, from a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub url: Url,
    /// Its host, as `guard::host_of` writes it.
    pub host: String,
    /// Whether that's the card's own host. When it isn't, the person
    /// approves it at the window, with presence, or nothing is called.
    pub same_host: bool,
}

/// A card as XenoNet fetched it.
#[derive(Clone, Debug)]
pub struct Card {
    pub url: Url,
    pub card: AgentCard,
    /// The pin, in hex.
    pub hash: String,
    pub interface: Interface,
}

/// What XenoNet keeps of an outbound actor's card, and what the person
/// allowed for it, when they registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
    pub card_url: String,
    pub hash: String,
    pub interface_url: String,
    /// The interface's host, when it isn't the card's and the person
    /// approved it.
    pub approved_host: Option<String>,
    /// The one host that isn't public which the person let XenoNet reach.
    pub private: Private,
}

impl Pinned {
    /// What the person approves when they register `card`: its pin, its
    /// interface, and the interface's host when it's another.
    pub fn of(card: &Card, private: Private) -> Pinned {
        Pinned {
            card_url: card.url.to_string(),
            hash: card.hash.clone(),
            interface_url: card.interface.url.to_string(),
            approved_host: (!card.interface.same_host).then(|| card.interface.host.clone()),
            private,
        }
    }

    /// `card`, fetched again, still says what was pinned, and its
    /// interface is on a host the person approved.
    pub fn check(&self, card: &Card) -> Result<(), A2aError> {
        if card.hash != self.hash {
            return Err(A2aError::new("changed", "this agent's card changed since you approved it; look at it again in XenoNet's window before XenoNet calls it"));
        }
        if card.interface.url.as_str() != self.interface_url {
            return Err(A2aError::new("changed", "this agent's card names another address to call than the one you approved"));
        }
        if !card.interface.same_host && self.approved_host.as_deref() != Some(card.interface.host.as_str()) {
            return Err(A2aError::new("host", format!("this agent's card says to call {}, another host than the card's, which you haven't approved", card.interface.host)));
        }
        Ok(())
    }
}

/// `v` written canonically: object keys sorted at every depth.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&o[k], out);
            }
            out.push('}');
        }
        Value::Array(l) => {
            out.push('[');
            for (i, x) in l.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The pin of a card's JSON.
pub fn pin(v: &Value) -> String {
    let mut text = String::new();
    canonical(v, &mut text);
    Sha256::digest(text.as_bytes()).iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The card's address: as given, or its origin's well-known path when the
/// person gave just the agent's address.
pub fn card_url(text: &str) -> Result<Url, A2aError> {
    let mut url = guard::https(text)?;
    if url.path() == "/" && url.query().is_none() {
        url.set_path(WELL_KNOWN);
    }
    Ok(url)
}

/// Reads a body up to `max` bytes.
pub(crate) async fn body(mut r: reqwest::Response, max: usize, what: &str) -> Result<Vec<u8>, A2aError> {
    let mut out = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| crate::transport(&e))? {
        if out.len() + chunk.len() > max {
            return Err(A2aError::new("protocol", format!("{what} is over {} KiB", max >> 10)));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Fetches the card at `text` under the guard's rules, and reads its
/// JSON-RPC interface for A2A 1.x.
pub async fn fetch(text: &str, private: &Private, roots: Option<&[reqwest::Certificate]>) -> Result<Card, A2aError> {
    let url = card_url(text)?;
    guard::check(&url, private)?;
    let client = guard::client(&private.for_url(&url), roots, true)?;
    let r = client.get(url.clone()).header("accept", "application/json").timeout(FETCH).send().await.map_err(|e| crate::transport(&e))?;
    if r.status().is_redirection() {
        return Err(A2aError::new("redirect", "the card's address redirects, which XenoNet doesn't follow"));
    }
    if !r.status().is_success() {
        return Err(A2aError::new("card", format!("{} answered {} for its card", guard::host_of(&url), r.status().as_u16())));
    }
    // Where the card came from, after redirects within its host.
    let at = r.url().clone();
    let bytes = body(r, CARD_MAX, "the card").await?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| A2aError::new("card", format!("the card isn't JSON: {e}")))?;
    let card: AgentCard = serde_json::from_value(v.clone()).map_err(|e| A2aError::new("card", format!("that isn't an A2A 1.0 Agent Card: {e}")))?;
    if card.capabilities.streaming != Some(true) {
        return Err(A2aError::new("card", "this agent's card says it doesn't stream, and XenoNet follows a task only by its stream"));
    }
    let Some(iface) = card
        .supported_interfaces
        .iter()
        .find(|i| i.protocol_binding.eq_ignore_ascii_case(TRANSPORT_PROTOCOL_JSONRPC) && (i.protocol_version == "1" || i.protocol_version.starts_with("1.")))
    else {
        return Err(A2aError::new("card", "this agent's card offers no JSON-RPC interface for A2A 1.x, the one XenoNet speaks"));
    };
    let iface_url = guard::https(&iface.url)?;
    let host = guard::host_of(&iface_url);
    let same_host = host == guard::host_of(&at);
    Ok(Card { url: at, hash: pin(&v), card, interface: Interface { url: iface_url, host, same_host } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_pin_ignores_key_order_and_nothing_else() {
        let a = json!({ "name": "Scout", "skills": [{ "id": "s", "tags": ["x"] }], "version": "1" });
        let b: Value = serde_json::from_str(r#"{"version":"1","skills":[{"tags":["x"],"id":"s"}],"name":"Scout"}"#).unwrap();
        assert_eq!(pin(&a), pin(&b));
        assert_ne!(pin(&a), pin(&json!({ "name": "Scout", "skills": [{ "id": "s", "tags": ["x"] }], "version": "2" })));
        assert_ne!(
            pin(&a),
            pin(&json!({ "name": "Scout", "skills": [{ "id": "s", "tags": ["x"] }], "version": "1", "iconUrl": "https://x" })),
            "a field XenoNet doesn't read counts too"
        );
    }

    #[test]
    fn an_agents_address_means_its_card() {
        assert_eq!(card_url("https://agent.example.com").unwrap().as_str(), "https://agent.example.com/.well-known/agent-card.json");
        assert_eq!(card_url("https://agent.example.com/cards/scout.json").unwrap().path(), "/cards/scout.json");
        assert_eq!(card_url("http://agent.example.com").unwrap_err().code, "http");
    }
}
