// core/units.js in Rust (crates/xenonet-base/src/units.rs): money, tokens,
// durations, shares and long text as people read them. The console page
// keeps the JS module.
import { native } from "../index.js";

// Numbers as JS would read them; anything else is left to JS, as it was.
const num = (v) => (v == null ? undefined : Number(v));

export function money(n) {
  if (n != null && typeof n !== "number") return n > 0 && n < 0.01 ? "<$0.01" : `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
  return native.unitsMoney(num(n));
}

export const tokens = (n) => native.unitsTokens(num(n));

export const duration = (ms) => native.unitsDuration(num(ms));

export const plural = (n, word, many = `${word}s`) => native.unitsPlural(Number(n), n === 1, String(word), String(many));

export function share(part, whole) {
  if (!whole) return "";
  return native.unitsShare(part / whole);
}

// A cut through a character is left to JS, whose half of one Rust can't
// hand back (it prints as the U+FFFD Rust writes).
const cutsPair = (s, n) => n > 0 && n < s.length && s.charCodeAt(n - 1) >= 0xd800 && s.charCodeAt(n - 1) <= 0xdbff;

export function excerpt(s, n = 4000) {
  if (typeof s === "string" && Number.isInteger(n) && n >= 0 && !cutsPair(s, n)) return native.unitsExcerpt(s, n);
  return s && s.length > n ? s.slice(0, n) + `\n… (${s.length - n} more chars)` : s ?? "";
}
