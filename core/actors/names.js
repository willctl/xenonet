// Actors' names and slugs (docs/design/actors/interface.md, section 2,
// "Registering"). A name is what agents and the card see, so it can't read
// as you or as XenoNet, nor as an app or product the actor isn't; names are
// compared without case, spaces, punctuation or accents, after folding
// look-alike forms (full-width letters, ligatures) and look-alike letters
// (a Cyrillic letter shaped like C, a Greek omicron, a Latin alpha, the
// digit 0) into the Latin ones they pass for. A name keeps to one script,
// as Unicode's highly restrictive profile has it (UTS #39), and holds
// nothing that draws as nothing. A slug is the actor's id (actor:<slug>):
// some are XenoNet's own words or its routes', and a product's slug is that
// product's alone.
import crypto from "node:crypto";
import { ACTOR_SLUG } from "../schema/ids.js";
import { looksLikeKey } from "./detail.js";
import { PRODUCTS, productOfSlug } from "./products.js";

export const nameKey = (s) => String(s).normalize("NFKC").toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

// Letters that pass for Latin ones, by the letter they pass for, as code
// points: from Cyrillic, Greek and Armenian, the Latin alphabet's own small
// capitals and IPA letters, Cherokee (whose small letters are drawn as
// small capitals of the same shapes), and the digit 0. "I" stands for what
// reads as a capital I or a small l alike (Latin I, the digit 1, a bar,
// Cyrillic I and palochka, Greek iota, the small capital I), which is read
// both ways.
const CHEROKEE = (...upper) => upper.flatMap((u) => [u, String.fromCodePoint(u).toLowerCase().codePointAt(0)]);
const PASSES_FOR = {
  a: [0x0410, 0x0430, 0x0391, 0x03b1, 0x0251, 0x1d00, ...CHEROKEE(0x13aa)],
  b: [0x0412, 0x0432, 0x042c, 0x044c, 0x0392, 0x0299, ...CHEROKEE(0x13f4)],
  c: [0x0421, 0x0441, 0x03f9, 0x03f2, 0x1d04, ...CHEROKEE(0x13df)],
  d: [0x0501, 0x1d05, ...CHEROKEE(0x13a0)],
  e: [0x0415, 0x0435, 0x0395, 0x1d07, ...CHEROKEE(0x13ac)],
  g: [0x0581, 0x0261, 0x0262, ...CHEROKEE(0x13c0)],
  h: [0x041d, 0x043d, 0x04ba, 0x04bb, 0x0397, 0x0570, 0x029c, ...CHEROKEE(0x13bb)],
  i: [0x0456, 0xa647, 0x03b9, 0x0131, 0x0269, ...CHEROKEE(0x13a5)],
  j: [0x0408, 0x0458, 0x03f3, 0x0237, 0x1d0a, ...CHEROKEE(0x13ab)],
  k: [0x041a, 0x043a, 0x039a, 0x03ba, 0x0138, 0x1d0b, ...CHEROKEE(0x13e6)],
  l: [0x053c, 0x029f, ...CHEROKEE(0x13de)],
  m: [0x041c, 0x043c, 0x039c, 0x1d0d, ...CHEROKEE(0x13b7)],
  n: [0x043f, 0x039d, 0x03b7, 0x0578, 0x0274],
  o: [0x041e, 0x043e, 0x039f, 0x03bf, 0x0555, 0x0585, 0x1d0f, 0x30],
  p: [0x0420, 0x0440, 0x03a1, 0x03c1, 0x1d18, ...CHEROKEE(0x13e2)],
  q: [0x051a, 0x051b, 0x0566],
  r: [0x0433, 0x0280, ...CHEROKEE(0x13a1)],
  s: [0x0405, 0x0455, 0x054f, 0xa731, ...CHEROKEE(0x13da)],
  t: [0x0422, 0x0442, 0x03a4, 0x03c4, 0x1d1b, ...CHEROKEE(0x13a2)],
  u: [0x03c5, 0x054d, 0x057d, 0x028b, 0x1d1c],
  v: [0x0474, 0x0475, 0x03bd, 0x1d20, ...CHEROKEE(0x13d9)],
  w: [0x051c, 0x051d, 0x0461, 0x03c9, 0x0561, 0x1d21, ...CHEROKEE(0x13b3, 0x13d4)],
  x: [0x0425, 0x0445, 0x03a7, 0x03c7],
  y: [0x0423, 0x0443, 0x04ae, 0x04af, 0x03a5, 0x03b3, 0x03d2, 0x028f, ...CHEROKEE(0x13a9)],
  z: [0x0396, 0x1d22, ...CHEROKEE(0x13c3)],
  I: [0x49, 0x31, 0x7c, 0x0406, 0x04c0, 0x04cf, 0x0399, 0x026a, 0x01c0],
};
const LOOKALIKES = new Map(Object.entries(PASSES_FOR).flatMap(([to, from]) => from.map((cp) => [String.fromCodePoint(cp), to])));

// What a name reads as, as keys like nameKey's: look-alike forms and
// accents folded, look-alike letters made the Latin ones they pass for,
// and what reads as I or l taken first as an i, then as an l ("CIaude").
export function readsAs(s) {
  const letters = [...String(s).normalize("NFKC").normalize("NFD").replace(/\p{M}/gu, "")].map((c) => LOOKALIKES.get(c) ?? c);
  const key = (ell) => nameKey(letters.map((c) => (c === "I" ? (ell ? "l" : "i") : c)).join(""));
  return [...new Set([key(false), key(true)])];
}

// The scripts whose letters a name may hold, one at a time; Han with the
// scripts Japanese, Chinese and Korean write with it, and Latin with any
// of those, as UTS #39's highly restrictive profile allows. A letter of a
// script not listed counts as a script of its own.
const SCRIPTS = ["Latin", "Greek", "Cyrillic", "Armenian", "Georgian", "Cherokee", "Coptic", "Glagolitic", "Lisu", "Hebrew", "Arabic", "Syriac", "Thaana", "Nko", "Devanagari", "Bengali", "Gurmukhi", "Gujarati", "Oriya", "Tamil", "Telugu", "Kannada", "Malayalam", "Sinhala", "Thai", "Lao", "Tibetan", "Myanmar", "Khmer", "Mongolian", "Ethiopic", "Canadian_Aboriginal", "Tifinagh", "Vai", "Yi", "Javanese", "Balinese", "Sundanese", "Adlam", "Osage", "Deseret", "Runic", "Ogham", "Hangul", "Hiragana", "Katakana", "Bopomofo", "Han"].map((name) => [name, new RegExp(`^\\p{Script=${name}}$`, "u")]);
const TOGETHER = [["Latin", "Han", "Hiragana", "Katakana"], ["Latin", "Han", "Bopomofo"], ["Latin", "Han", "Hangul"]].map((s) => new Set(s));
function oneScript(name) {
  const scripts = new Set();
  for (const c of name) if (/\p{L}/u.test(c)) scripts.add(SCRIPTS.find(([, re]) => re.test(c))?.[0] ?? "another");
  return scripts.size <= 1 || TOGETHER.some((set) => [...scripts].every((s) => set.has(s)));
}

// Characters that draw as nothing: those Unicode says to ignore when they
// can't be shown (Hangul fillers among them, which count as letters), and
// the blank Braille pattern.
const INVISIBLE = /[\p{Default_Ignorable_Code_Point}\u2800]/u;

// Names that read as you or as XenoNet.
const YOURS = new Set(["You", "Me", "User", "Your user", "Owner", "XenoNet", "System", "Admin"].map(nameKey));
// Apps no actor is.
const APPS = new Map(["Claude", "Claude Code", "Codex", "ChatGPT"].map((n) => [nameKey(n), n]));
// Names a product goes by, which only its own actor may take.
const PRODUCT_NAMES = new Map([
  ["Hermes", "hermes-agent"],
  ["Hermes Agent", "hermes-agent"],
  ["Grok Bot", "grok-bot"],
  ["Muse", "muse"],
  ["dots", "dots"],
].map(([n, p]) => [nameKey(n), p]));

// Characters that break a line or reorder text, which a name can't hold.
const NOT_ONE_LINE = /[\u0000-\u001f\u007f-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/;

// A name as XenoNet takes it, for an actor that is `product` (an id, or null
// for one of your own): { ok, name }, or { ok: false, reason }.
export function checkName(raw, product = null) {
  if (typeof raw !== "string") return { ok: false, reason: "a name is text" };
  const name = raw.trim();
  if (NOT_ONE_LINE.test(name)) return { ok: false, reason: "a name is one line of plain text" };
  // Agents read a name quoted ("Wren" (actor, …)), so nothing in it may
  // close the quote or pass for one.
  if (/["`\\\u201c\u201d]/.test(name)) return { ok: false, reason: "a name can't hold quotation marks or backslashes, since agents read it quoted" };
  if (!name || [...name].length > 64) return { ok: false, reason: "a name is 1 to 64 characters" };
  if (INVISIBLE.test(name)) return { ok: false, reason: "a name can't hold characters that draw as nothing" };
  if (!nameKey(name)) return { ok: false, reason: "a name has letters or digits in it" };
  const keys = readsAs(name);
  if (keys.some((k) => YOURS.has(k))) return { ok: false, reason: "that name reads as you or as XenoNet, which no actor may" };
  const app = keys.find((k) => APPS.has(k));
  if (app) return { ok: false, reason: `that name reads as ${APPS.get(app)}, which an actor isn't` };
  const owner = keys.map((k) => PRODUCT_NAMES.get(k)).find((p) => p && p !== product);
  if (owner) return { ok: false, reason: `that name is ${PRODUCTS[owner].name}'s, and this actor isn't ${PRODUCTS[owner].name}` };
  if (!oneScript(name)) return { ok: false, reason: "a name keeps to one script, so that no letter in it passes for another's" };
  if (looksLikeKey(name)) return { ok: false, reason: "that name looks like it holds a key or a token" };
  return { ok: true, name };
}

// The makers of those apps and products, and XenoNet's: Anysphere (Cursor)
// builds and runs Grok Bot for SpaceXAI.
const MAKERS = new Map(["Anthropic", "OpenAI", "Nous Research", "SpaceXAI", "xAI", "Anysphere", "Cursor", "Meta", "willctl"].map((n) => [nameKey(n), n]));

// Why a product of an actor's own can't have this id or vendor (`what`),
// or null when it can. They're the actor's own word, and show as such, but
// none may read as you or XenoNet, as an app or product XenoNet knows, or as
// one of their makers, compared as names are.
export function productWordRefusal(text, what) {
  const keys = readsAs(text);
  if (keys.some((k) => YOURS.has(k))) return `the product's ${what} reads as you or as XenoNet`;
  const app = keys.find((k) => APPS.has(k));
  if (app) return `the product's ${what} reads as ${APPS.get(app)}, which this actor isn't`;
  const product = keys.map((k) => PRODUCT_NAMES.get(k)).find(Boolean);
  if (product) return `the product's ${what} reads as ${PRODUCTS[product].name}, which this actor isn't`;
  const maker = keys.find((k) => MAKERS.has(k));
  if (maker) return `the product's ${what} reads as ${MAKERS.get(maker)}, who doesn't make this actor`;
  return null;
}

// Slugs that are XenoNet's own words, or its routes' (/api/actors/me…): a
// route is matched before /api/actors/<slug>, so an actor with its word
// for a slug couldn't be read there.
export const RESERVED_SLUGS = new Set(["you", "me", "user", "owner", "xenonet", "system", "admin", "root", "claude", "codex", "chatgpt", "anthropic", "openai", "events", "requests", "register", "heartbeat", "targets", "observers", "tokens", "gateway"]);

// A slug made from a name: lowercase, accents dropped, other characters
// made dashes (one for a run of them, none at the ends), cut to 32.
export function slugFrom(name) {
  const s = String(name ?? "")
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+/, "")
    .slice(0, 32)
    .replace(/-+$/, "");
  return s || "actor";
}

// Why an actor that is `product` can't have this slug, or null when it can.
export function slugRefusal(slug, product = null) {
  if (typeof slug !== "string" || !ACTOR_SLUG.test(slug)) return "a slug is 1 to 32 lowercase letters, digits and dashes, starting with a letter or a digit";
  if (RESERVED_SLUGS.has(slug)) return `the slug ${slug} is XenoNet's own`;
  const owner = productOfSlug(slug);
  if (owner && owner !== product) return `the slug ${slug} is ${PRODUCTS[owner].name}'s`;
  if (Object.hasOwn(PRODUCTS, product ?? "") && owner !== product) return `${PRODUCTS[product].name}'s actor has a slug of its own: ${PRODUCTS[product].slug}${product === "hermes-agent" ? ", or hermes-<profile> for a profile" : ""}`;
  return null;
}

// A free slug like `base`, for a 409 taken: wren-2, wren-3…, cut to fit.
export function suggestSlug(base, { taken, product = null }) {
  for (let n = 2; n < 1000; n++) {
    const tail = `-${n}`;
    const s = `${base.slice(0, 32 - tail.length).replace(/-+$/, "")}${tail}`;
    if (!slugRefusal(s, product) && !taken(s)) return s;
  }
  return null;
}

// The actor of a Hermes profile: hermes for the default one, else
// hermes-<profile> when the profile's name is a slug as it stands (work),
// and otherwise, or when it's cut to 32, with four hex digits of its name
// added (Work, my_work, 工作, a long name). Names that differ only in case
// or punctuation, or that have no letters of a-z, would otherwise share
// one actor. Hermes's observer and a Hermes plugin registering later both
// take it, so they name the same actor.
export function hermesSlug(profile) {
  if (!profile || profile === "default") return "hermes";
  const full = `hermes-${slugFrom(profile)}`;
  if (full.length <= 32 && slugFrom(profile) === profile) return full;
  const tag = crypto.createHash("sha256").update(String(profile)).digest("hex").slice(0, 4);
  return `${full.slice(0, 27).replace(/-+$/, "")}-${tag}`;
}
