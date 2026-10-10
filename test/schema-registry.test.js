// The schema registry and the check against it (core/schema), with schemas
// of the test's own under test/…, which no other test file sees: each test
// file runs in a process of its own.
import { test } from "node:test";
import assert from "node:assert/strict";
import { define, schemaFor, schemaIds, jsonSchemas } from "../core/schema/registry.js";
import { check, explain, pathText } from "../core/schema/check.js";
import { z, obj, ms, openEnum, passthrough } from "../core/schema/common.js";

const Leaf = define("test/leaf", 1, obj({ at: ms, tag: openEnum(["a", "b"]) }), { title: "Leaf", description: "A leaf" });
const Tree = define(
  "test/tree",
  1,
  obj({
    leaves: z.array(Leaf),
    maybe: Leaf.nullable().optional(),
    byName: z.record(z.string(), obj({ n: z.int() })),
    raw: passthrough(obj({ known: z.string() })).optional(),
    pick: z.discriminatedUnion("kind", [obj({ kind: z.literal("x"), x: z.number() }), obj({ kind: z.literal("y"), y: z.string() })]).optional(),
    either: z.union([obj({ a: z.string(), n: z.number() }), obj({ b: z.string() })]).optional(),
    both: z.intersection(Leaf, obj({ extra: z.boolean() })).optional(),
  }),
  { title: "Tree", description: "A tree of leaves", "x-xenonet": { note: "kept" } },
);

test("schemas are registered by path and major, and found by either form of id", () => {
  assert.equal(schemaFor("test/leaf/v1"), Leaf);
  assert.equal(schemaFor("https://xenonet.dev/schemas/test/tree/v1.json"), Tree);
  assert.throws(() => schemaFor("test/leaf/v2"), /no schema test\/leaf\/v2/);
  assert.throws(() => schemaFor("nonsense"), /no schema/);
  assert.throws(() => define("test/leaf", 1, obj({}), { title: "T", description: "D" }), /defined twice/);
  assert.throws(() => define("test/bare", 1, obj({}), { title: "T" }), /title and a description/);
  assert.deepEqual(schemaIds().filter((id) => id.startsWith("test/")), ["test/leaf/v1", "test/tree/v1"]);
});

test("written out: draft 2020-12, the contract's $ids, and $refs between them", () => {
  const all = jsonSchemas();
  const tree = all["test/tree/v1"];
  assert.equal(tree.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.equal(tree.$id, "https://xenonet.dev/schemas/test/tree/v1.json");
  assert.match(tree.$comment, /npm run schemas/);
  assert.deepEqual([tree.title, tree.description, tree["x-xenonet"]], ["Tree", "A tree of leaves", { note: "kept" }]);
  assert.deepEqual(tree.properties.leaves.items, { $ref: "https://xenonet.dev/schemas/test/leaf/v1.json" });
  assert.deepEqual(tree.additionalProperties, {}, "loose: unknown keys are allowed");
  assert.deepEqual(all["test/leaf/v1"].properties.tag.anyOf.map((o) => o.enum ?? o.type), [["a", "b"], "string"], "an open set: known values, and any string");
  assert.deepEqual(Object.keys(tree).slice(0, 5), ["$schema", "$id", "$comment", "title", "description"]);
});

test("a check passes what fits, and says where what doesn't", () => {
  const good = { leaves: [{ at: 1, tag: "a" }, { at: 2, tag: "anything" }], byName: { x: { n: 1 } }, pick: { kind: "y", y: "s" }, both: { at: 3, tag: "b", extra: true } };
  assert.deepEqual(check(Tree, good, { strict: true }), { ok: true, issues: [], undeclared: [] });
  assert.ok(check("test/tree/v1", good).ok, "by id too");

  const bad = check(Tree, { leaves: [{ at: "soon", tag: "a" }], byName: { "a b": { n: 1.5 } }, pick: { kind: "z" } });
  assert.equal(bad.ok, false);
  assert.deepEqual(
    bad.issues.map((i) => i.path),
    ["leaves[0].at", 'byName["a b"].n', "pick.kind"],
  );
  assert.match(explain(bad), /^leaves\[0\]\.at: Invalid input: expected number/m);

  // A union that fits no option: the closest one's problems.
  const union = check(Tree, { leaves: [], byName: {}, either: { a: "x", n: "1" } });
  assert.deepEqual(union.issues.map((i) => i.path), ["either.n"]);
  assert.deepEqual(check(Tree, null).issues.map((i) => i.path), [""], "the value itself");
  assert.match(explain(check(Tree, null)), /^\(the value\): /);
});

test("strictly, keys the schema doesn't declare are named, except in data passed through", () => {
  const data = {
    leaves: [{ at: 1, tag: "a", color: "red" }],
    byName: { k: { n: 1, also: true } },
    maybe: null,
    raw: { known: "x", theirs: 1 },
    pick: { kind: "x", x: 1, why: "?" },
    both: { at: 1, tag: "a", extra: false, more: 1 },
    gone: undefined,
    top: 1,
  };
  const result = check(Tree, data, { strict: true });
  assert.equal(result.ok, false);
  assert.deepEqual(result.issues, [], "zod itself lets them through");
  assert.deepEqual(result.undeclared.sort(), ["both.more", 'byName.k.also', "leaves[0].color", "pick.why", "top"].sort());
  assert.match(explain(result), /^leaves\[0\]\.color: not in the schema$/m);
  assert.deepEqual(check(Tree, data).undeclared, [], "only when asked");
});

test("paths read the way they're written in code", () => {
  assert.equal(pathText(["nodes", 3, "ranAs", "id"]), "nodes[3].ranAs.id");
  assert.equal(pathText(["layout", "project:/x"]), 'layout["project:/x"]');
  assert.equal(pathText([0, "a"]), "[0].a");
  assert.equal(pathText([]), "");
});
