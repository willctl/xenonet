//! What to give an app so its agent reaches XenoNet's actor gateway
//! (core/actors/handoff.js): the steps, each with what it copies (the
//! message to paste into the app's chat, the token or the address), for a
//! new token, or for a new address alone, when the gateway's address
//! changed and the app's token still works. Shared by XenoNet's window and
//! the xenonet command.

use xenonet_adapter::products::{handoff_of, product_of};

/// A step, and what it copies: "message", "token", "url", "wake" (a way to
/// set up its wake) or None.
pub type Step = (&'static str, Option<&'static str>);

pub struct Handoff {
    pub app: Option<&'static str>,
    pub steps: Vec<Step>,
    pub message: Option<String>,
}

/// For `product`, at `url` (whose host is `host`), with a new `token` or
/// none (a new address alone). For an agent that speaks A2A (product
/// "a2a"), `url` is XenoNet's Agent Card, and the token goes as A2A's bearer.
pub fn handoff(product: Option<&str>, url: &str, host: &str, token: Option<&str>) -> Handoff {
    // A product's own steps, as the products table has them.
    if let Some(h) = product.and_then(handoff_of) {
        let steps = if token.is_some() { h.token } else { h.address };
        return Handoff { app: product_of(h.product).map(|p| p.name), steps: steps.steps.to_vec(), message: steps.message.map(|m| filled(m, url, host)) };
    }
    match (product, token) {
        (Some("a2a"), Some(_)) => Handoff {
            app: Some("A2A agent"),
            steps: vec![("Give the agent XenoNet's Agent Card URL.", Some("url")), ("Have it send the token in the header Authorization: Bearer <token>.", Some("token"))],
            message: None,
        },
        (Some("a2a"), None) => Handoff { app: Some("A2A agent"), steps: vec![("Give the agent XenoNet's new Agent Card URL. Its token still works.", Some("url"))], message: None },
        (_, Some(_)) => Handoff {
            app: None,
            steps: vec![("In the app, add an MCP server with this URL.", Some("url")), ("Send the token in the header Authorization: Bearer <token>.", Some("token"))],
            message: None,
        },
        (_, None) => Handoff { app: None, steps: vec![("Change the URL of its XenoNet MCP server to this.", Some("url"))], message: None },
    }
}

/// How to give XenoNet a way to wake an app's agent (core/actors/wake.js),
/// as the products table has it: for Grok Bot, a routine that starts from
/// a webhook, made from its chat, whose URL and key go into XenoNet's
/// window. None for an app XenoNet knows no way for.
pub fn wake_help(product: Option<&str>) -> Option<([&'static str; 2], &'static str)> {
    handoff_of(product?)?.wake
}

/// A handoff's message with the gateway's `{url}` and `{host}` in it, in
/// one pass, so neither is read for the other.
fn filled(message: &str, url: &str, host: &str) -> String {
    let mut out = String::with_capacity(message.len() + url.len());
    let mut rest = message;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{url}") {
            out.push_str(url);
            rest = after;
        } else if let Some(after) = tail.strip_prefix("{host}") {
            out.push_str(host);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_a2a_agent_gets_the_card_and_the_bearer() {
        let h = handoff(Some("a2a"), "https://x.example/.well-known/agent-card.json", "x.example", Some("t"));
        assert_eq!(h.app, Some("A2A agent"));
        assert_eq!(h.steps.iter().map(|(_, copy)| *copy).collect::<Vec<_>>(), [Some("url"), Some("token")]);
        assert_eq!(h.message, None, "the token never goes in a message to paste");
    }

    #[test]
    fn grok_bot_gets_its_token_in_its_secret_card_never_its_chat() {
        let h = handoff(Some("grok-bot"), "https://x.example/mcp", "x.example", Some("xenonet_gateway_x"));
        assert_eq!(h.steps.iter().map(|(_, copy)| *copy).collect::<Vec<_>>(), [Some("message"), Some("token"), Some("wake")]);
        let message = h.message.unwrap();
        assert!(!message.contains("xenonet_gateway_x"), "the token never goes in Grok Bot's chat");
        assert!(message.starts_with("Add an MCP server named \"xenonet\" by URL: https://x.example/mcp. Ask me for its token with your secure secret card"));
        assert!(wake_help(Some("grok-bot")).is_some() && wake_help(Some("muse")).is_none() && wake_help(None).is_none());
    }

    #[test]
    fn muse_gets_the_host_and_the_url_and_another_app_the_plain_steps() {
        let h = handoff(Some("muse"), "https://x.example/{host}", "x.example", Some("t"));
        assert_eq!(h.app, Some("Muse"));
        assert_eq!(
            h.message.as_deref(),
            Some("Create a custom connector named \"XenoNet\" that uses an API key, for the host x.example, sending the key as a bearer token in the Authorization header. I'll paste the key into the key field when you ask. Then call https://x.example/{host} with JSON-RPC (method tools/call, tool whoami) and tell me what it says.")
        );
        let h = handoff(Some("hermes-agent"), "https://x.example/mcp", "x.example", None);
        assert_eq!((h.app, h.steps.len(), h.message), (None, 1, None));
        assert_eq!(filled("{a} {url}{", "U", "H"), "{a} U{");
    }
}
