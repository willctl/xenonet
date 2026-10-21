//! What to give an app so its agent reaches XenoNet's actor gateway
//! (core/actors/handoff.js, which stays JS for XenoNet's window; this is
//! the command's copy): the steps, each with what it copies (the message
//! to paste into the app's chat, the token or the address), for a new
//! token, or for a new address alone, when the gateway's address changed
//! and the app's token still works.

use serde_json::{json, Value};

/// `new URL(url).host`: the host, lowercased, and its port unless it's the
/// scheme's own; "" for what isn't a URL.
pub fn host_of(url: &str) -> String {
    let Some((scheme, rest)) = url.trim().split_once("://") else { return String::new() };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) || !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return String::new();
    }
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h).to_ascii_lowercase();
    if host.is_empty() {
        return String::new();
    }
    let default = match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        "ftp" => Some("21"),
        _ => None,
    };
    match host.rsplit_once(':') {
        Some((h, port)) if !h.ends_with(']') || h.starts_with('[') => {
            if port.is_empty() || Some(port) == default {
                h.to_string()
            } else {
                host.clone()
            }
        }
        _ => host,
    }
}

fn step(text: &str, copy: Option<&str>) -> Value {
    json!({ "text": text, "copy": copy })
}

/// `handoff(product, { url, token })`: { app, steps: [{ text, copy }],
/// message }. copy is "message", "token", "url", "wake" or null; `token`
/// is None for a new address alone.
pub fn handoff(product: Option<&str>, url: &str, token: Option<&str>) -> Value {
    let host = host_of(url);
    match (product, token) {
        // An agent that speaks A2A: `url` is XenoNet's Agent Card.
        (Some("a2a"), Some(_)) => json!({
            "app": "A2A agent",
            "steps": [step("Give the agent XenoNet's Agent Card URL.", Some("url")), step("Have it send the token in the header Authorization: Bearer <token>.", Some("token"))],
            "message": null,
        }),
        (Some("a2a"), None) => json!({ "app": "A2A agent", "steps": [step("Give the agent XenoNet's new Agent Card URL. Its token still works.", Some("url"))], "message": null }),
        (Some("grok-bot"), Some(token)) => json!({
            "app": "Grok Bot",
            "steps": [step("Paste this into a new Grok Bot chat.", Some("message")), step("Once it's added, delete that chat: the token stays in it.", None), step("For answers within seconds, give XenoNet a way to wake it.", Some("wake"))],
            "message": format!("Add an MCP server named \"xenonet\" by URL: {url} with the header Authorization: Bearer {token}. Then call its whoami tool and tell me what it says."),
        }),
        (Some("grok-bot"), None) => json!({
            "app": "Grok Bot",
            "steps": [step("Paste this into a Grok Bot chat.", Some("message"))],
            "message": format!("XenoNet's address changed. Change the URL of your MCP server \"xenonet\" to {url} and keep its Authorization header as it is. Then call its whoami tool and tell me what it says."),
        }),
        (Some("muse"), Some(_)) => json!({
            "app": "Muse",
            "steps": [step("Paste this into a Muse chat, then approve the connector card.", Some("message")), step("When Muse asks for the key, paste the token into the card, not the chat.", Some("token"))],
            "message": format!("Create a custom connector named \"XenoNet\" that uses an API key, for the host {host}, sending the key as a bearer token in the Authorization header. I'll paste the key into the key field when you ask. Then call {url} with JSON-RPC (method tools/call, tool whoami) and tell me what it says."),
        }),
        (Some("muse"), None) => json!({
            "app": "Muse",
            "steps": [step("Paste this into a Muse chat.", Some("message")), step("If Muse can't change the connector, reconnect Muse for a new token.", None)],
            "message": format!("XenoNet's address changed to {url}. Change your \"XenoNet\" connector's host to {host} and keep its key. Then call whoami at the new address and tell me what it says."),
        }),
        (_, Some(_)) => {
            json!({ "app": null, "steps": [step("In the app, add an MCP server with this URL.", Some("url")), step("Send the token in the header Authorization: Bearer <token>.", Some("token"))], "message": null })
        }
        (_, None) => json!({ "app": null, "steps": [step("Change the URL of its XenoNet MCP server to this.", Some("url"))], "message": null }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_as_url_reads_them() {
        assert_eq!(host_of("https://xenonet-demo.ngrok-free.app/mcp"), "xenonet-demo.ngrok-free.app");
        assert_eq!(host_of("https://User@Example.COM:443/x"), "example.com");
        assert_eq!(host_of("http://127.0.0.1:47850/mcp"), "127.0.0.1:47850");
        assert_eq!(host_of("xenonet.example.com"), "");
    }

    #[test]
    fn a_new_address_alone() {
        let h = handoff(Some("muse"), "https://xenonet-demo.ngrok-free.app/mcp", None);
        assert_eq!(h["app"], "Muse");
        assert!(h["message"].as_str().unwrap().contains("Change your \"XenoNet\" connector's host to xenonet-demo.ngrok-free.app and keep its key"));
        assert_eq!(handoff(None, "https://x.example/mcp", None)["steps"][0]["copy"], "url");
    }

    #[test]
    fn an_a2a_agent_gets_the_card_and_the_bearer() {
        let h = handoff(Some("a2a"), "https://x.example/.well-known/agent-card.json", Some("t"));
        assert_eq!(h["app"], "A2A agent");
        assert_eq!(h["steps"][0]["copy"], "url");
        assert_eq!(h["steps"][1]["copy"], "token");
        assert_eq!(h["message"], Value::Null, "the token never goes in a message to paste");
    }
}
