//! What to give an app so its agent reaches XenoNet's actor gateway
//! (core/actors/handoff.js): the steps, each with what it copies (the
//! message to paste into the app's chat, the token or the address), for a
//! new token, or for a new address alone, when the gateway's address
//! changed and the app's token still works. Shared by XenoNet's window and
//! the xenonet command.

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
    match (product, token) {
        (Some("a2a"), Some(_)) => Handoff {
            app: Some("A2A agent"),
            steps: vec![("Give the agent XenoNet's Agent Card URL.", Some("url")), ("Have it send the token in the header Authorization: Bearer <token>.", Some("token"))],
            message: None,
        },
        (Some("a2a"), None) => Handoff { app: Some("A2A agent"), steps: vec![("Give the agent XenoNet's new Agent Card URL. Its token still works.", Some("url"))], message: None },
        // The token goes into Grok Bot's secure secret card, which its
        // model never sees, never into its chat.
        (Some("grok-bot"), Some(_)) => Handoff {
            app: Some("Grok Bot"),
            steps: vec![
                ("Paste this into a Grok Bot chat.", Some("message")),
                ("When Grok Bot asks for the token, paste it into its secret card, not the chat, and save it.", Some("token")),
                ("For answers within seconds, give XenoNet a way to wake it.", Some("wake")),
            ],
            message: Some(format!("Add an MCP server named \"xenonet\" by URL: {url}. Ask me for its token with your secure secret card, named XENONET_GATEWAY_TOKEN, never in chat, and send it in the header Authorization: Bearer <the token>. Then call its whoami tool and tell me what it says.")),
        },
        (Some("grok-bot"), None) => Handoff {
            app: Some("Grok Bot"),
            steps: vec![("Paste this into a Grok Bot chat.", Some("message"))],
            message: Some(format!("XenoNet's address changed. Change the URL of your MCP server \"xenonet\" to {url} and keep its Authorization header as it is. Then call its whoami tool and tell me what it says.")),
        },
        (Some("muse"), Some(_)) => Handoff {
            app: Some("Muse"),
            steps: vec![("Paste this into a Muse chat, then approve the connector card.", Some("message")), ("When Muse asks for the key, paste the token into the card, not the chat.", Some("token"))],
            message: Some(format!("Create a custom connector named \"XenoNet\" that uses an API key, for the host {host}, sending the key as a bearer token in the Authorization header. I'll paste the key into the key field when you ask. Then call {url} with JSON-RPC (method tools/call, tool whoami) and tell me what it says.")),
        },
        (Some("muse"), None) => Handoff {
            app: Some("Muse"),
            steps: vec![("Paste this into a Muse chat.", Some("message")), ("If Muse can't change the connector, reconnect Muse for a new token.", None)],
            message: Some(format!("XenoNet's address changed to {url}. Change your \"XenoNet\" connector's host to {host} and keep its key. Then call whoami at the new address and tell me what it says.")),
        },
        (_, Some(_)) => Handoff { app: None, steps: vec![("In the app, add an MCP server with this URL.", Some("url")), ("Send the token in the header Authorization: Bearer <token>.", Some("token"))], message: None },
        (_, None) => Handoff { app: None, steps: vec![("Change the URL of its XenoNet MCP server to this.", Some("url"))], message: None },
    }
}

/// How to give XenoNet a way to wake an app's agent (core/actors/wake.js):
/// for Grok Bot, a routine that starts from a webhook, made from its chat,
/// whose URL and key go into XenoNet's window. None for an app XenoNet
/// knows no way for.
pub fn wake_help(product: Option<&str>) -> Option<([&'static str; 2], &'static str)> {
    (product == Some("grok-bot")).then_some((
        ["Paste this into a Grok Bot chat to make the routine.", "Open the routine's settings and copy its Webhook URL and Webhook key below."],
        "Create a routine named \"XenoNet inbox\" that starts from a webhook. Its instruction: read your XenoNet inbox with your xenonet tools and answer anything that needs an answer. Tell me when it's made.",
    ))
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
    }
}
