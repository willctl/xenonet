// Actors' tokens (electron/lib/tokens.js; docs/design/actors/interface.md,
// section 2): their shape, and found only in the Authorization header.
import { test } from "node:test";
import assert from "node:assert/strict";
import { isToken, isGatewayToken, bearerOf, tokenInUrl } from "../electron/lib/tokens.js";
const LS = String.fromCharCode(0x2028);
const RLO = String.fromCharCode(0x202e);

// Look-alike letters, written as code points so it's plain which they are.
const u = (...cps) => String.fromCodePoint(...cps);
const CYR = { C: u(0x0421), o: u(0x043e), S: u(0x0405), e: u(0x0435), c: u(0x0441), d: u(0x0501), x: u(0x0445) };

test("a token is written as xenonet_actor_ and base64url; only the service says it's one", () => {
  const token = `xenonet_actor_${"Ab-_".repeat(20)}`;
  assert.equal(isToken(token), true);
  assert.equal(isGatewayToken(`xenonet_gateway_${"x".repeat(64)}`), true);
  for (const bad of [`${token}=`, "xenonet_actor_short", `xenonet_gateway_${"x".repeat(64)}`, `xenonet_actor_${"a".repeat(16385)}`, null, 7]) assert.equal(isToken(bad), false, String(bad));
});

test("a token comes as Authorization: Bearer, never in a URL", () => {
  const token = `xenonet_actor_${"Zz9-".repeat(20)}`;
  assert.equal(bearerOf({}), undefined);
  assert.equal(bearerOf({ authorization: `Bearer ${token}` }), token);
  assert.equal(bearerOf({ authorization: `bearer   ${token} ` }), token, "the scheme is matched without case");
  assert.equal(bearerOf({ authorization: `Basic ${token}` }), null);
  assert.equal(bearerOf({ authorization: "Bearer" }), null);
  assert.equal(bearerOf({ authorization: `Bearer ${token} extra` }), null);
  const url = (s) => new URL(s, "http://127.0.0.1");
  assert.equal(tokenInUrl(url(`/api/actors/me?token=${token}`)), true);
  assert.equal(tokenInUrl(url("/api/actors/me?access_token=x")), true);
  assert.equal(tokenInUrl(url(`/api/actors/me?q=${token}`)), true);
  assert.equal(tokenInUrl(url(`/api/actors/${token}`)), true);
  assert.equal(tokenInUrl(url("/api/actors/events?since=0")), false);
});
