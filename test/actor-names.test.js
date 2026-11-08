// Actors' tokens (electron/lib/tokens.js; docs/design/actors/interface.md,
// section 2): made, hashed and found only in the Authorization header.
import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { newToken, hashToken, isToken, bearerOf, tokenInUrl, TOKEN_PREFIX } from "../electron/lib/tokens.js";
const LS = String.fromCharCode(0x2028);
const RLO = String.fromCharCode(0x202e);

// Look-alike letters, written as code points so it's plain which they are.
const u = (...cps) => String.fromCodePoint(...cps);
const CYR = { C: u(0x0421), o: u(0x043e), S: u(0x0405), e: u(0x0435), c: u(0x0441), d: u(0x0501), x: u(0x0445) };

test("a token: xenonet_actor_ and 64 hex digits, new each time, kept only as its SHA-256", () => {
  const a = newToken();
  const b = newToken();
  assert.match(a.token, /^xenonet_actor_[0-9a-f]{64}$/);
  assert.notEqual(a.token, b.token);
  assert.equal(a.hash, crypto.createHash("sha256").update(a.token).digest("hex"));
  assert.equal(hashToken(a.token), a.hash);
  assert.ok(!a.hash.includes(a.token.slice(TOKEN_PREFIX.length, TOKEN_PREFIX.length + 16)));
  assert.equal(isToken(a.token), true);
  for (const bad of [`${a.token}0`, a.token.toUpperCase(), a.token.slice(1), "xenonet_actor_", null, 7]) assert.equal(isToken(bad), false, String(bad));
});

test("a token comes as Authorization: Bearer, never in a URL", () => {
  const { token } = newToken();
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
