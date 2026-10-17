// core/actors/products.js in Rust (crates/xenonet-engine/src/products.rs):
// the actor products XenoNet knows, their slugs and names, which XenoNet
// observes and which report themselves.
import { native } from "../../index.js";

export const PRODUCTS = Object.fromEntries(native.actorProducts().map((p) => [p.id, p]));

export const productOf = (id) => {
  const known = typeof id === "string" ? native.actorProductOf(id) : null;
  return known ? PRODUCTS[known] : null;
};

export const WHY_NOT = Object.fromEntries(native.actorProductWhyNot());

export const OBSERVED = Object.values(PRODUCTS).filter((p) => p.observed);

export const productOfSlug = (slug) => native.actorProductOfSlug(String(slug)) ?? null;
