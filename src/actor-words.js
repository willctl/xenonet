// How an actor's product reads (src/actor-inspector.js, through actorWhat
// the tooltip, and the Actors panel's rows): a product XenoNet knows goes
// by its own name and maker, and any other is the actor's own word, quoted
// and said to be, since any program can send any id, maker and version
// (docs/design/actors/interface.md, section 2). The request card words it
// the same way.
import { PRODUCT_NAMES } from "./actors-model.js";

export const knownProduct = (product) => Object.hasOwn(PRODUCT_NAMES, product?.id ?? "");

// What it is, in a few words: Hermes Agent, or “acme-sync”, its own word.
export function productWhat(product) {
  if (!product?.id) return null;
  return knownProduct(product) ? PRODUCT_NAMES[product.id] : `“${product.id}”, its own word`;
}

// Who makes it and which version: Nous Research · v0.21.5, or “Acme” ·
// v2.1.0, its own words.
export function productMade(product) {
  const known = knownProduct(product);
  const parts = [product?.vendor && (known ? product.vendor : `“${product.vendor}”`), product?.version && `v${product.version}`].filter(Boolean);
  if (!parts.length) return null;
  return known ? parts.join(" · ") : `${parts.join(" · ")}, its own word${parts.length > 1 ? "s" : ""}`;
}

// What it is, who makes it and which version, on one line, as the Actors
// panel's rows have it: Hermes Agent · Nous Research · v0.21.5, or
// “acme-sync” · “Acme” · v2.1.0, its own words, said once for them all.
export function productLine(product) {
  if (!product?.id) return null;
  if (knownProduct(product)) return [PRODUCT_NAMES[product.id], productMade(product)].filter(Boolean).join(" · ");
  const parts = [`“${product.id}”`, product.vendor && `“${product.vendor}”`, product.version && `v${product.version}`].filter(Boolean);
  return `${parts.join(" · ")}, its own word${parts.length > 1 ? "s" : ""}`;
}
