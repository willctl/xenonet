// What to give an app so its agent reaches XenoNet's actor gateway
// (server/actor-gateway.js): the steps, each with what it copies (the
// message to paste into the app's chat, the token or the address), for a
// new token, or for a new address alone, when the gateway's address
// changed and the app's token still works. Shared by XenoNet's window and
// the xenonet command.
const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
};

// { app, steps: [{ text, copy }], message }: copy is "message", "token",
// "url", "wake" (a way to set up its wake) or null. `token` is null for a
// new address alone. For an agent that speaks A2A (product "a2a"), `url` is
// XenoNet's Agent Card, and the token goes as A2A's bearer.
export function handoff(product, { url, token = null }) {
  const host = hostOf(url);
  const step = (text, copy = null) => ({ text, copy });
  if (product === "a2a")
    return token
      ? { app: "A2A agent", steps: [step("Give the agent XenoNet's Agent Card URL.", "url"), step("Have it send the token in the header Authorization: Bearer <token>.", "token")], message: null }
      : { app: "A2A agent", steps: [step("Give the agent XenoNet's new Agent Card URL. Its token still works.", "url")], message: null };
  // The token goes into Grok Bot's secure secret card, which its model never
  // sees, never into its chat.
  if (product === "grok-bot")
    return token
      ? {
          app: "Grok Bot",
          steps: [
            step("Paste this into a Grok Bot chat.", "message"),
            step("When Grok Bot asks for the token, paste it into its secret card, not the chat, and save it.", "token"),
            step("For answers within seconds, give XenoNet a way to wake it.", "wake"),
          ],
          message: `Add an MCP server named "xenonet" by URL: ${url}. Ask me for its token with your secure secret card, named XENONET_GATEWAY_TOKEN, never in chat, and send it in the header Authorization: Bearer <the token>. Then call its whoami tool and tell me what it says.`,
        }
      : {
          app: "Grok Bot",
          steps: [step("Paste this into a Grok Bot chat.", "message")],
          message: `XenoNet's address changed. Change the URL of your MCP server "xenonet" to ${url} and keep its Authorization header as it is. Then call its whoami tool and tell me what it says.`,
        };
  if (product === "muse")
    return token
      ? {
          app: "Muse",
          steps: [step("Paste this into a Muse chat, then approve the connector card.", "message"), step("When Muse asks for the key, paste the token into the card, not the chat.", "token")],
          message: `Create a custom connector named "XenoNet" that uses an API key, for the host ${host}, sending the key as a bearer token in the Authorization header. I'll paste the key into the key field when you ask. Then call ${url} with JSON-RPC (method tools/call, tool whoami) and tell me what it says.`,
        }
      : {
          app: "Muse",
          steps: [step("Paste this into a Muse chat.", "message"), step("If Muse can't change the connector, reconnect Muse for a new token.")],
          message: `XenoNet's address changed to ${url}. Change your "XenoNet" connector's host to ${host} and keep its key. Then call whoami at the new address and tell me what it says.`,
        };
  return token
    ? { app: null, steps: [step("In the app, add an MCP server with this URL.", "url"), step("Send the token in the header Authorization: Bearer <token>.", "token")], message: null }
    : { app: null, steps: [step("Change the URL of its XenoNet MCP server to this.", "url")], message: null };
}

// How to give XenoNet a way to wake an app's agent (core/actors/wake.js): for
// Grok Bot, a routine that starts from a webhook, made from its chat, whose
// URL and key go into XenoNet's window. { steps, message } or null for an app
// XenoNet knows no way for.
export function wakeHelp(product) {
  if (product === "grok-bot")
    return {
      steps: ["Paste this into a Grok Bot chat to make the routine.", "Open the routine's settings and copy its Webhook URL and Webhook key below."],
      message: 'Create a routine named "XenoNet inbox" that starts from a webhook. Its instruction: read your XenoNet inbox with your xenonet tools and answer anything that needs an answer. Tell me when it\'s made.',
    };
  return null;
}
