// Actors' tokens (docs/design/actors/interface.md, section 2, "Tokens";
// docs/design/direction.md, "Authorization"): Biscuits minted by XenoNet's
// service, written as xenonet_actor_<base64url>. The prefix makes a leaked
// token easy to recognize, by people and by secret scanners. Only the
// service says whether one is a token; here it's only its shape. A token
// is shown once and travels only as Authorization: Bearer, never in a URL.
export const TOKEN_PREFIX = "xenonet_actor_";
export const TOKEN = /^xenonet_actor_[A-Za-z0-9_-]{64,16384}$/;
// A gateway token: the actor's grant narrowed to XenoNet's actor gateway,
// which takes requests from outside this machine, made in XenoNet's window
// as xenonet_gateway_<base64url>. An actor's own token never works there,
// so a token that leaks from this Mac can't be used from the internet.
export const GATEWAY_PREFIX = "xenonet_gateway_";
export const GATEWAY_TOKEN = /^xenonet_gateway_[A-Za-z0-9_-]{64,16384}$/;

export const isToken = (s) => typeof s === "string" && TOKEN.test(s);
export const isGatewayToken = (s) => typeof s === "string" && GATEWAY_TOKEN.test(s);

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
