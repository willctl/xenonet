// core/actors/tokens.js in Rust (crates/xenonet-actors/src/tokens.rs):
// actors' and gateway tokens, xenonet_actor_<64 hex digits> and
// xenonet_gateway_<64 hex digits>, of which XenoNet keeps only the SHA-256.
import { native } from "../../index.js";

const [tokenPrefix, tokenPattern, gatewayPrefix, gatewayPattern, grep] = native.tokenPatterns();

export const TOKEN_PREFIX = tokenPrefix;
export const TOKEN = new RegExp(tokenPattern);
export const GATEWAY_PREFIX = gatewayPrefix;
export const GATEWAY_TOKEN = new RegExp(gatewayPattern);
export const TOKEN_GREP = grep;

export const isToken = (s) => typeof s === "string" && native.isToken(s);
export const isGatewayToken = (s) => typeof s === "string" && native.isGatewayToken(s);
export const hashToken = (token) => native.hashToken(String(token));

// A new token, and the hash XenoNet keeps of it.
export const newToken = () => native.newToken();

// A new gateway token, and the hash XenoNet keeps of it.
export const newGatewayToken = () => native.newGatewayToken();

// The token a request's Authorization header carries: undefined when it
// has no such header, null when it isn't a Bearer, and otherwise the token
// as sent.
export function bearerOf(headers = {}) {
  const raw = headers.authorization;
  if (raw === undefined) return undefined;
  return native.bearerOf(String(raw)) ?? null;
}

// Whether a URL carries a token, which XenoNet refuses.
export const tokenInUrl = (url) => native.tokenInUrl([...url.searchParams], url.pathname);
