// Actors' tokens (docs/design/actors/interface.md, section 2, "Tokens"):
// 32 random bytes written as xenonet_actor_<64 hex digits>. The prefix makes
// a leaked token easy to recognize, by people and by secret scanners, and
// hex never holds an underscore, so a double-click still selects the whole
// token. XenoNet keeps only the SHA-256: a token has 256 bits of randomness,
// so a fast hash is enough, and XenoNet finds an actor by the hash, so no
// comparison leaks timing. A token is shown once and travels only as
// Authorization: Bearer, never in a URL.
import crypto from "node:crypto";

export const TOKEN_PREFIX = "xenonet_actor_";
export const TOKEN = /^xenonet_actor_[0-9a-f]{64}$/;
// A gateway token (server/actor-gateway.js): a credential of the actor's
// apart from its own token, made in XenoNet's window, as
// xenonet_gateway_<64 hex digits>. It works only at XenoNet's actor gateway,
// which takes requests from outside this machine, and an actor's own token
// never works there: so a token that leaks from this Mac can't be used from
// the internet, and one made for the gateway can't stand in for the process
// checks here.
export const GATEWAY_PREFIX = "xenonet_gateway_";
export const GATEWAY_TOKEN = /^xenonet_gateway_[0-9a-f]{64}$/;
// Either kind anywhere in a file, as git grep -E reads it (the doctor's
// look for token files in repos).
export const TOKEN_GREP = "xenonet_(actor|gateway)_[0-9a-f]{64}";

export const isToken = (s) => typeof s === "string" && TOKEN.test(s);
export const isGatewayToken = (s) => typeof s === "string" && GATEWAY_TOKEN.test(s);
export const hashToken = (token) => crypto.createHash("sha256").update(String(token)).digest("hex");

// A new token, and the hash XenoNet keeps of it.
export function newToken() {
  const token = TOKEN_PREFIX + crypto.randomBytes(32).toString("hex");
  return { token, hash: hashToken(token) };
}

// A new gateway token, and the hash XenoNet keeps of it.
export function newGatewayToken() {
  const token = GATEWAY_PREFIX + crypto.randomBytes(32).toString("hex");
  return { token, hash: hashToken(token) };
}

// The token a request's Authorization header carries (RFC 6750): undefined
// when it has no such header, null when it isn't a Bearer, and otherwise the
// token as sent (which may still be one XenoNet doesn't know).
export function bearerOf(headers = {}) {
  const raw = headers.authorization;
  if (raw === undefined) return undefined;
  const m = /^Bearer[ \t]+(\S+)[ \t]*$/i.exec(String(raw));
  return m ? m[1] : null;
}

// Whether a URL carries a token, which XenoNet refuses: tokens "SHOULD NOT be
// passed in page URLs" (RFC 6750, section 5.3), where logs and history keep
// them. Any parameter named token or access_token, or any value holding a
// token's prefix.
export function tokenInUrl(url) {
  const held = (s) => s.includes(TOKEN_PREFIX) || s.includes(GATEWAY_PREFIX);
  for (const [k, v] of url.searchParams) if (/^(access_)?token$/i.test(k) || held(v)) return true;
  return held(url.pathname);
}
