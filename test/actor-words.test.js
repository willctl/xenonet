// How an actor's product reads in the inspector, the Actors panel and the
// tooltip (src/actor-words.js): a product XenoNet knows by its own name, and
// any other as the actor's own word. Made-up products throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { productWhat, productMade, productLine, knownProduct } from "../src/actor-words.js";

test("a product XenoNet knows goes by its name and maker", () => {
  const hermes = { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" };
  assert.equal(knownProduct(hermes), true);
  assert.equal(productWhat(hermes), "Hermes Agent");
  assert.equal(productMade(hermes), "Nous Research · v0.21.5");
  assert.equal(productMade({ id: "grok-bot", vendor: "SpaceXAI", version: null }), "SpaceXAI");
});

test("any other product is the actor's own word, and says so: its id, maker and version", () => {
  const claimed = { id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" };
  assert.equal(knownProduct(claimed), false);
  assert.equal(productWhat(claimed), "“acme-sync”, its own word");
  assert.equal(productMade(claimed), "“Acme Tools” · v2.1.0, its own words");
  assert.equal(productMade({ id: "acme-sync", vendor: null, version: "2.1.0" }), "v2.1.0, its own word");
  assert.equal(productMade({ id: "acme-sync" }), null, "nothing to say");
  assert.equal(productWhat(null), null);
});

test("the Actors panel's line: name, maker and version, with any other product's said once to be its own words", () => {
  assert.equal(productLine({ id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }), "Hermes Agent · Nous Research · v0.21.5");
  assert.equal(productLine({ id: "grok-bot", vendor: "SpaceXAI", version: null }), "Grok Bot · SpaceXAI");
  assert.equal(productLine({ id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" }), "“acme-sync” · “Acme Tools” · v2.1.0, its own words");
  assert.equal(productLine({ id: "acme-sync", vendor: null, version: "2.1.0" }), "“acme-sync” · v2.1.0, its own words");
  assert.equal(productLine({ id: "acme-sync" }), productWhat({ id: "acme-sync" }), "just its id: as the inspector says it");
  assert.equal(productLine(null), null);
});
