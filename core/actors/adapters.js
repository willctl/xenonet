// Remote provider adapters (docs/design/cloud-assistants.md, section 3):
// the part of a remote actor's connection that belongs to its product.
// Common code (connections.js) owns consent, grants, projects, the actor's
// record and the log; an adapter only says which transports its product
// reaches XenoNet through, and checks what its provider can prove.
//
// Each adapter has:
//
//   id                 its own id
//   product            the product it's for (products.js), or null for
//                      a program of your own
//   transports         the routes it may use (TRANSPORTS)
//   support            "experimental": its route is meant for real use and
//                      untested end to end; "fixture": only its common code
//                      is tested, against made-up evidence
//   verifyAccount(e)   the account the actor works under, from evidence a
//                      transport supplied: { state: "verified", issuer,
//                      provider, workspace, subject } from a validated
//                      provider assertion, or { state: "unavailable",
//                      reason } when there's nothing to check. Never a
//                      guess, and never from a local sign-in file
//   verifyActor(e)     the actor's own id at its provider, the same way:
//                      { state: "verified", issuer, accountBinding,
//                      externalActorId } or unavailable
//   capabilities()     what a connection through it may do: read, in v1
//
// None of the transports here passes XenoNet a provider assertion it can
// check, so every account link they make is unverified, or your word
// (owner_asserted). A product's name sent by a caller never joins an actor;
// only your choice in XenoNet's window does.

export const TRANSPORTS = {
  "openai-secure-mcp-tunnel": { id: "openai-secure-mcp-tunnel", name: "OpenAI Secure MCP Tunnel", provider: "openai" },
  "local-stdio": { id: "local-stdio", name: "A local MCP client, over stdio", provider: "local" },
};

const unavailable = (reason) => async () => ({ state: "unavailable", reason });

export const ADAPTERS = {
  dots: {
    id: "openai-dots",
    product: "dots",
    provider: "openai",
    transports: ["openai-secure-mcp-tunnel", "local-stdio"],
    support: "experimental",
    // The tunnel forwards an MCP call to the local adapter; it passes no
    // assertion of the ChatGPT user or the dot that XenoNet could validate.
    // A tunnel's own organization and credential are OpenAI Platform's,
    // which say nothing of the user.
    verifyAccount: unavailable("transport-has-no-assertion"),
    verifyActor: unavailable("transport-has-no-assertion"),
    capabilities: () => ["read"],
  },
  "grok-bot": {
    id: "grok-bot",
    product: "grok-bot",
    provider: "spacexai",
    transports: ["local-stdio"],
    support: "fixture",
    verifyAccount: unavailable("no-supported-route"),
    verifyActor: unavailable("no-supported-route"),
    capabilities: () => ["read"],
  },
  "hermes-agent": {
    id: "hermes-agent",
    product: "hermes-agent",
    provider: "nous",
    transports: ["local-stdio"],
    support: "fixture",
    verifyAccount: unavailable("no-supported-route"),
    verifyActor: unavailable("no-supported-route"),
    capabilities: () => ["read"],
  },
  custom: {
    id: "custom",
    product: null,
    provider: "local",
    transports: ["openai-secure-mcp-tunnel", "local-stdio"],
    support: "experimental",
    verifyAccount: unavailable("no-provider"),
    verifyActor: unavailable("no-provider"),
    capabilities: () => ["read"],
  },
};

// The adapter for a product (by its id), or for a program of your own
// (null); null for a product that can't be paired.
export function adapterFor(product, adapters = ADAPTERS) {
  if (product == null) return adapters.custom ?? null;
  return Object.values(adapters).find((a) => a.product === product) ?? null;
}

// What an adapter's verification said, checked: a verified answer must name
// its issuer and subject, or it's taken as unavailable. Adapters are code
// XenoNet ships, but a mistake in one must not read as proof.
export async function verified(fn, evidence) {
  let out;
  try {
    out = await fn(evidence);
  } catch {
    return { state: "unavailable", reason: "check-failed" };
  }
  if (out?.state !== "verified") return { state: "unavailable", reason: typeof out?.reason === "string" ? out.reason : "unverified" };
  const fields = ["issuer", ...(out.externalActorId !== undefined ? ["accountBinding", "externalActorId"] : ["subject"])];
  if (!fields.every((k) => typeof out[k] === "string" && out[k])) return { state: "unavailable", reason: "incomplete" };
  return out;
}
