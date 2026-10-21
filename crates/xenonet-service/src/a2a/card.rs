//! XenoNet's Agent Card (A2A 1.0, spec section 4.4): what an A2A client
//! reads at `/.well-known/agent-card.json` before it calls. One interface,
//! the JSON-RPC binding at `<address>/a2a`, for version 1.0 only; a gateway
//! token as HTTP bearer; streaming, and no push notifications.

use serde_json::{json, Value};

/// What the card says beyond its address.
#[derive(Clone, Debug)]
pub struct CardSettings {
    /// The network's name, or this host's, or the public address's host.
    pub name: String,
    /// XenoNet's version.
    pub version: String,
}

pub const PATH: &str = "/a2a";
pub const CARD_PATH: &str = "/.well-known/agent-card.json";
/// The key the card's security scheme is under.
pub const SCHEME: &str = "gateway";
/// The skill's id.
pub const SKILL: &str = "message-session";
const DOCS: &str = "https://github.com/willctl/xenonet/blob/main/docs/guide/actors.md#agents-that-speak-a2a";

/// The address as an origin: no trailing slash, path or query.
pub fn origin(public_address: &str) -> String {
    let a = public_address.trim();
    let (scheme, rest) = a.split_once("://").unwrap_or(("https", a));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{}://{host}", scheme.to_ascii_lowercase())
}

/// The host of an address, for a card with no other name.
pub fn host_of(public_address: &str) -> String {
    origin(public_address).split_once("://").map(|(_, h)| h.to_string()).unwrap_or_default()
}

/// The 1.0 Agent Card for XenoNet at `public_address` (an origin such as
/// `https://mac.example.ts.net`).
pub fn agent_card(settings: &CardSettings, public_address: &str) -> Value {
    let base = origin(public_address);
    json!({
        "name": format!("XenoNet on {}", settings.name),
        "description": "Reach the coding sessions (Claude Code, Codex and others) that XenoNet connects here. Each A2A agent is an actor with its own token and scopes: it reaches only the sessions its user allowed, and a session's reply is the result of the agent's task.",
        "supportedInterfaces": [{ "url": format!("{base}{PATH}"), "protocolBinding": "JSONRPC", "protocolVersion": super::rpc::VERSION }],
        "version": settings.version,
        "documentationUrl": DOCS,
        "capabilities": { "streaming": true, "pushNotifications": false, "extendedAgentCard": false },
        "securitySchemes": {
            SCHEME: { "httpAuthSecurityScheme": { "scheme": "Bearer", "description": "A gateway token made in XenoNet's window for this agent, sent as Authorization: Bearer <token>" } }
        },
        "securityRequirements": [{ "schemes": { SCHEME: { "list": [] } } }],
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": SKILL,
            "name": "Message a coding session",
            "description": "Sends your message to a session your scope allows, and the session's reply completes the task. Name the session in the message's metadata as \"target\": its id (claude:…, codex:…), or repo:<name> for the sessions in that repo. With no target, the message goes to your default target, or, in a context you used before, back to whoever answered last. A context is one XenoNet thread. Text parts only.",
            "tags": ["coding", "sessions", "messages", "claude-code", "codex"],
            "examples": [
                "{\"message\": {\"role\": \"ROLE_USER\", \"parts\": [{\"text\": \"Which branch has the login fix?\"}], \"metadata\": {\"target\": \"repo:atlas\"}}}",
                "Ask the session working on the docs site whether the build is green."
            ],
            "inputModes": ["text/plain"],
            "outputModes": ["text/plain"]
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_from_addresses() {
        assert_eq!(origin("https://Mac.example.ts.net/mcp"), "https://Mac.example.ts.net");
        assert_eq!(origin("mac.example.ts.net"), "https://mac.example.ts.net");
        assert_eq!(origin("http://127.0.0.1:47850/"), "http://127.0.0.1:47850");
        assert_eq!(host_of("https://gw.example.com/mcp"), "gw.example.com");
    }

    #[test]
    fn the_card_says_one_interface_and_bearer() {
        let c = agent_card(&CardSettings { name: "studio".into(), version: "0.2.0".into() }, "https://gw.example.com/mcp");
        assert_eq!(c["name"], "XenoNet on studio");
        assert_eq!(c["supportedInterfaces"], json!([{ "url": "https://gw.example.com/a2a", "protocolBinding": "JSONRPC", "protocolVersion": "1.0" }]));
        assert_eq!(c["capabilities"], json!({ "streaming": true, "pushNotifications": false, "extendedAgentCard": false }));
        assert_eq!(c["securitySchemes"]["gateway"]["httpAuthSecurityScheme"]["scheme"], "Bearer");
        assert_eq!(c["securityRequirements"], json!([{ "schemes": { "gateway": { "list": [] } } }]));
        assert_eq!(c["skills"][0]["name"], "Message a coding session");
        // Every field the spec requires of a card and a skill.
        for k in ["name", "description", "supportedInterfaces", "version", "capabilities", "defaultInputModes", "defaultOutputModes", "skills"] {
            assert!(c.get(k).is_some(), "{k}");
        }
        for k in ["id", "name", "description", "tags"] {
            assert!(c["skills"][0].get(k).is_some(), "{k}");
        }
    }
}
