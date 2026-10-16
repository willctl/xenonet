// Actors' names, slugs and tokens (core/actors/names.js, tokens.js and
// products.js; docs/design/actors/interface.md, section 2): no name that
// reads as you, XenoNet or a product the actor isn't, slugs that are XenoNet's
// or a product's own, and tokens made, hashed and found only in the
// Authorization header. Made-up names throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { checkName, nameKey, readsAs, slugFrom, slugRefusal, suggestSlug, hermesSlug, RESERVED_SLUGS } from "../core/actors/names.js";
import { newToken, hashToken, isToken, bearerOf, tokenInUrl, TOKEN_PREFIX } from "../core/actors/tokens.js";
import { PRODUCTS, productOf, productOfSlug, OBSERVED, WHY_NOT } from "../core/actors/products.js";

const LS = String.fromCharCode(0x2028);
const RLO = String.fromCharCode(0x202e);

test("a name: one line of 1 to 64 characters, cleaned of the spaces around it", () => {
  assert.deepEqual(checkName("  Wren  "), { ok: true, name: "Wren" });
  assert.deepEqual(checkName("Docs builder 2"), { ok: true, name: "Docs builder 2" });
  for (const bad of ["", "   ", "x".repeat(65), `Wren${LS}Wren`, "Wren\nXenoNet", `Wren${RLO}`, "Wren\tx", "!!!"]) assert.equal(checkName(bad).ok, false, JSON.stringify(bad));
  assert.equal(checkName(42).ok, false);
  const curly = String.fromCharCode(0x201c);
  for (const quoted of ['Wren" (registered, actor:you', "Wren `x`", "Wren\\", `${curly}Wren`]) assert.match(checkName(quoted).reason, /quotation marks or backslashes/, quoted);
  assert.equal(checkName("Wren's builder (docs)").ok, true, "an apostrophe and brackets are fine");
  assert.match(checkName(`deploy ${"ab12".repeat(10)}`).reason, /key or a token/);
});

test("a name never reads as you or as XenoNet, compared without case, spaces, punctuation or look-alike forms", () => {
  for (const name of ["You", "me", "USER", "Your user", "your-user", "Owner", "XenoNet", "X.E.N.O.N.E.T.", "ｘｅｎｏｎｅｔ", "System", "admin"]) {
    const r = checkName(name);
    assert.equal(r.ok, false, name);
    assert.match(r.reason, /reads as you or as XenoNet/, name);
  }
  assert.equal(nameKey("Your User!"), "youruser");
  assert.equal(checkName("Yours truly").ok, true, "only the whole name counts");
});

test("a name never reads as an app or product the actor isn't; a product's actor may take its own", () => {
  for (const name of ["Claude", "claude code", "Claude-Code", "Codex", "ChatGPT"]) assert.match(checkName(name).reason, /which an actor isn't/, name);
  for (const [name, product] of [["Hermes", "Hermes Agent"], ["hermes agent", "Hermes Agent"], ["Grok Bot", "Grok Bot"], ["grok-bot", "Grok Bot"], ["MUSE", "Muse"], ["dots", "dots"]]) {
    const r = checkName(name);
    assert.equal(r.ok, false, name);
    assert.ok(r.reason.includes(`${product}'s`), `${name}: ${r.reason}`);
  }
  assert.deepEqual(checkName("Hermes", "hermes-agent"), { ok: true, name: "Hermes" });
  assert.equal(checkName("Grok Bot", "hermes-agent").ok, false, "Hermes's actor isn't Grok Bot");
  assert.equal(checkName("Wren", "hermes-agent").ok, true, "a product's actor may have a name of your choosing");
});

// Look-alike letters, written as code points so it's plain which they are.
const u = (...cps) => String.fromCodePoint(...cps);
const CYR = { C: u(0x0421), o: u(0x043e), S: u(0x0405), e: u(0x0435), c: u(0x0441), d: u(0x0501), x: u(0x0445) };

test("look-alike letters don't get a name past the rules: other scripts', Latin's own, digits, accents, I for l", () => {
  for (const [name, reads] of [
    [`${CYR.C}laude`, /reads as Claude,/],
    [`Y${CYR.o}u`, /reads as you or as XenoNet/],
    [`${CYR.x}en${CYR.o}net`, /reads as you or as XenoNet/],
    [`${CYR.C}laude Code`, /reads as Claude Code/],
    [`${CYR.c}${CYR.o}${CYR.d}${CYR.e}${CYR.x}`, /reads as Codex/],
    [`Cl${u(0x0251)}ude`, /reads as Claude,/],
    [`${u(0x03a5)}${u(0x03bf)}${u(0x03c5)}`, /reads as you or as XenoNet/],
    [`${u(0x13da, 0x13a9, 0x13da, 0x13a2, 0x13ac, 0x13b7)}`, /reads as you or as XenoNet/],
    [`${u(0x13da, 0x13a9, 0x13da, 0x13a2, 0x13ac, 0x13b7)}`.toLowerCase(), /reads as you or as XenoNet/],
    ["Clàude", /reads as Claude,/],
    ["Y0u", /reads as you or as XenoNet/],
    ["C1aude", /reads as Claude,/],
    ["CIaude", /reads as Claude,/],
    [`H${CYR.e}rmes`, /Hermes Agent's/],
  ]) {
    const r = checkName(name);
    assert.equal(r.ok, false, name);
    assert.match(r.reason, reads, name);
  }
  assert.match(checkName("XENONET").reason, /reads as you or as XenoNet/, "capitals are still read as the letters they are");
  for (const name of ["Ivy", "Iris", "Docs builder 2", "Heron 1"]) assert.deepEqual(checkName(name), { ok: true, name }, name);
  assert.deepEqual(readsAs("Ivy"), ["ivy", "lvy"], "I is read as i and as l");
});

test("a name keeps to one script, and holds nothing that draws as nothing", () => {
  const filler = u(0x3164);
  for (const name of [`Claude${filler}`, `You${filler}`, `XenoNet${filler}`, `Wren${filler}`, `${u(0x115f)}Wren`, `Wren${u(0xffa0)}`, `Wren${u(0x2800)}`, `Wren${u(0xad)}`, `Wren${u(0xfe0f)}`]) assert.match(checkName(name).reason, /draw as nothing/, JSON.stringify(name));
  for (const name of [`Wr${CYR.e}n`, `${u(0x03bb)} helper`]) assert.match(checkName(name).reason, /keeps to one script/, name);
  for (const name of ["Héron", "Kite Ø", u(0x041a, 0x0438, 0x0442), u(0x65e5, 0x672c, 0x30dc, 0x30c3, 0x30c8), `Wren ${u(0x65e5, 0x672c)}`, u(0xd06c, 0xb9b0), u(0x05d3, 0x05d2)]) assert.deepEqual(checkName(name), { ok: true, name }, name);
});

test("a slug from a name: lowercase, accents gone, dashes for the rest, cut to 32", () => {
  assert.equal(slugFrom("Wren"), "wren");
  assert.equal(slugFrom("Café Bot!"), "cafe-bot");
  assert.equal(slugFrom("  --Docs   builder 2--  "), "docs-builder-2");
  assert.equal(slugFrom("A".repeat(40)), "a".repeat(32));
  assert.equal(slugFrom(`${"word ".repeat(6)}x`).length <= 32, true);
  assert.equal(slugFrom(`${"a".repeat(31)}-b`), "a".repeat(31), "never ends in a dash");
  assert.equal(slugFrom("日本語"), "actor", "a name with no letters of a-z still gets one");
});

test("a slug: the slug rule, never XenoNet's own words or its routes', and a product's only for that product", () => {
  assert.equal(slugRefusal("wren"), null);
  assert.equal(slugRefusal("wren-2"), null);
  for (const bad of ["Wren", "-wren", "w".repeat(33), "wren_2", "", null]) assert.match(slugRefusal(bad), /a slug is/, String(bad));
  for (const word of ["you", "me", "xenonet", "root", "anthropic", "openai", "events", "requests", "register", "heartbeat", "targets", "observers", "tokens"]) {
    assert.ok(RESERVED_SLUGS.has(word));
    assert.match(slugRefusal(word), /is XenoNet's own/, word);
  }
  for (const slug of ["hermes", "hermes-work", "grok-bot", "muse", "dots"]) assert.match(slugRefusal(slug), /'s$/, slug);
  assert.equal(slugRefusal("hermes", "hermes-agent"), null);
  assert.equal(slugRefusal("hermes-work", "hermes-agent"), null, "a Hermes profile's actor");
  assert.match(slugRefusal("wren", "hermes-agent"), /has a slug of its own: hermes, or hermes-<profile>/);
  assert.equal(slugRefusal("grok-bot", "grok-bot"), null);
  assert.equal(slugRefusal("hermesy"), null, "only hermes and hermes-…");
});

test("a free slug suggested for one that's taken, and a Hermes profile's slug", () => {
  const taken = new Set(["wren", "wren-2"]);
  assert.equal(suggestSlug("wren", { taken: (s) => taken.has(s) }), "wren-3");
  const long = "a".repeat(32);
  assert.equal(suggestSlug(long, { taken: () => false }), `${"a".repeat(30)}-2`);
  assert.equal(hermesSlug(), "hermes");
  assert.equal(hermesSlug("default"), "hermes");
  assert.equal(hermesSlug("work"), "hermes-work", "a name that's a slug as it stands");
  // Any other gets four hex digits of its name, so names that differ only
  // in case or punctuation, or have no letters of a-z, stay apart.
  const tag = (name) => crypto.createHash("sha256").update(name).digest("hex").slice(0, 4);
  assert.equal(hermesSlug("Work"), `hermes-work-${tag("Work")}`);
  assert.equal(hermesSlug("my_work"), `hermes-my-work-${tag("my_work")}`);
  assert.equal(hermesSlug("工作"), `hermes-actor-${tag("工作")}`);
  assert.equal(new Set(["work", "Work", "WORK", "my-work", "my_work", "my work", "工作", "работа"].map(hermesSlug)).size, 8);
  const cut = hermesSlug("a very long profile name that goes on and on");
  assert.ok(cut.length <= 32, cut);
  assert.match(cut, /^hermes-a-very-long-profile-[0-9a-f]{4}$/);
  assert.notEqual(cut, hermesSlug("a very long profile name that goes on and off"), "two long names stay apart");
  const slugLong = "a-long-profile-name-that-is-ok";
  assert.equal(hermesSlug(slugLong), `hermes-a-long-profile-name-${tag(slugLong)}`, "a slug as it stands, too long with hermes- before it, is cut and tagged");
  for (const name of ["Work", "my_work", "工作", cut]) assert.equal(slugRefusal(hermesSlug(name), "hermes-agent"), null, name);
});

test("the products: their slugs and names, which observe and which report themselves", () => {
  assert.deepEqual(OBSERVED.map((p) => p.id), ["hermes-agent", "grok-bot", "muse"], "dots on hold");
  assert.deepEqual(Object.values(PRODUCTS).filter((p) => p.registers).map((p) => p.id), ["hermes-agent", "grok-bot", "muse"], "dots don't report themselves");
  assert.deepEqual(Object.keys(WHY_NOT), ["dots", "numbat"]);
  assert.equal(productOf("hermes-agent").slug, "hermes");
  assert.equal(productOf("toaster"), null);
  assert.equal(productOf("constructor"), null, "only its own keys");
  assert.deepEqual(["hermes", "hermes-x", "grok-bot", "muse", "dots", "numbat-cursor", "wren"].map(productOfSlug), ["hermes-agent", "hermes-agent", "grok-bot", "muse", "dots", "numbat", null]);
});

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
