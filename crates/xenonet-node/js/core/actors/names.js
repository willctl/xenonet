// core/actors/names.js in Rust (crates/xenonet-actors/src/names.rs):
// actors' names, read as what they pass for, and slugs.
import { native } from "../../index.js";

export const nameKey = (s) => native.namesKey(String(s));
export const readsAs = (s) => native.namesReadsAs(String(s));
export const checkName = (raw, product = null) => native.namesCheck(raw, product);
export const productWordRefusal = (text, what) => native.namesProductWordRefusal(String(text), what) ?? null;
export const RESERVED_SLUGS = new Set(native.namesReservedSlugs());
export const slugFrom = (name) => native.namesSlugFrom(String(name ?? ""));
export const slugRefusal = (slug, product = null) => native.namesSlugRefusal(slug, product) ?? null;
export const suggestSlug = (base, { taken, product = null }) => native.namesSuggestSlug(base, product, (s) => !!taken(s)) ?? null;
export const hermesSlug = (profile) => native.namesHermesSlug(profile);
