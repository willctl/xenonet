// Ids as the contracts fix them (docs/design/contracts.md, sections 1 and 2).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  SCHEMA_BASE,
  shortId,
  schemaId,
  schemaUri,
  parseSchemaId,
  isSchemaPath,
  isHostId,
  parseQualified,
  qualify,
  formatQualified,
  isQualified,
  hostOf,
  localOf,
  actorId,
  parseActorId,
  isActorId,
  YOU,
  normalizeRemote,
  repoId,
  parseRepoId,
} from "../core/schema/ids.js";
import { normalizeRemote as fromProjects } from "../core/projects.js";

const SESSION = "claude:4f1c0000-0000-4000-8000-000000000001";

test("a schema's $id and its short form, both ways", () => {
  assert.equal(SCHEMA_BASE, "https://xenonet.dev/schemas");
  assert.equal(shortId("actor/snapshot", 1), "actor/snapshot/v1");
  assert.equal(schemaId("actor/snapshot", 1), "https://xenonet.dev/schemas/actor/snapshot/v1.json");
  assert.equal(schemaId("state/v2"), "https://xenonet.dev/schemas/state/v2.json", "from the short form");
  assert.equal(schemaUri("vendor/openai/codex/rollout-line/v3"), `${SCHEMA_BASE}/vendor/openai/codex/rollout-line/v3.json`);
  assert.deepEqual(parseSchemaId("drift/record/v1"), { id: "drift/record/v1", path: "drift/record", major: 1 });
  assert.deepEqual(parseSchemaId(`${SCHEMA_BASE}/live/snapshot/v12.json`), { id: "live/snapshot/v12", path: "live/snapshot", major: 12 });
  for (const bad of [null, "", "session", "session/v0", "session/v01", "Session/v1", "a//b/v1", "/a/v1", "a/v1/v2", `${SCHEMA_BASE}/x/v1`, "http://example.com/x/v1.json"]) assert.equal(parseSchemaId(bad), null, String(bad));
  assert.throws(() => shortId("session", 0), /major/);
  assert.throws(() => shortId("session", 1.5), /major/);
  assert.throws(() => shortId("Bad Path", 1), /path/);
  assert.throws(() => shortId("theme/v1", 2), /path/, "a path doesn't end in a version");
  assert.ok(isSchemaPath("vendor/anthropic/claude-code/transcript-line"));
  assert.ok(!isSchemaPath("") && !isSchemaPath(null));
});

test("host ids are chosen names, and an id from another host carries one", () => {
  for (const ok of ["studio", "vm1", "a1", "lab-mac"]) assert.ok(isHostId(ok), ok);
  for (const bad of ["a", "1vm", "Studio", "vm_1", "-vm", "x".repeat(33), "", null, 7]) assert.ok(!isHostId(bad), String(bad));

  assert.deepEqual(parseQualified(SESSION), { host: null, local: SESSION }, "unqualified: this XenoNet's own");
  assert.deepEqual(parseQualified(`@vm1:${SESSION}`), { host: "vm1", local: SESSION }, "the host runs to the first colon");
  assert.deepEqual(parseQualified("@vm1:project:/users/will/projects/atlas"), { host: "vm1", local: "project:/users/will/projects/atlas" });
  for (const bad of ["", "@", "@vm1", "@vm1:", "@:x", "@Vm1:x", "@v:x", "@vm1:@vm2:x", null, 3]) assert.equal(parseQualified(bad), null, String(bad));
  assert.ok(isQualified(`@vm1:${SESSION}`) && !isQualified(SESSION));
  assert.deepEqual([hostOf(`@vm1:${SESSION}`), hostOf(SESSION), hostOf("@")], ["vm1", null, null]);
  assert.deepEqual([localOf(`@vm1:${SESSION}`), localOf(SESSION)], [SESSION, SESSION]);
});

test("qualifying: another host's ids get its prefix, this XenoNet's own stay as they are", () => {
  assert.equal(qualify(SESSION, "vm1"), `@vm1:${SESSION}`);
  assert.equal(qualify(SESSION), SESSION, "no host: local");
  assert.equal(qualify(SESSION, "studio", { self: "studio" }), SESSION, "this host's own");
  assert.equal(qualify(`@vm1:${SESSION}`, "studio"), `@vm1:${SESSION}`, "already qualified: keeps its host");
  assert.equal(qualify(`@studio:${SESSION}`, "vm1", { self: "studio" }), SESSION, "this XenoNet's own, back from another host");
  assert.equal(formatQualified({ host: "vm1", local: "core" }), "@vm1:core");
  assert.equal(formatQualified({ local: "core" }), "core");
  assert.throws(() => qualify(SESSION, "VM 1"), /not a host id/);
  assert.throws(() => qualify("", "vm1"), /not an id/);
  // Round trip.
  for (const id of [SESSION, `@vm1:${SESSION}`, "@lab:actor:wren"]) assert.equal(formatQualified(parseQualified(id)), id);
});

test("actor ids, here and from another host", () => {
  assert.equal(YOU, "actor:you");
  assert.equal(actorId("wren"), "actor:wren");
  assert.equal(actorId("wren", "vm1"), "@vm1:actor:wren");
  assert.throws(() => actorId("Wren"), /slug/);
  assert.throws(() => actorId("-x"), /slug/);
  assert.throws(() => actorId("x".repeat(33)), /slug/);
  assert.deepEqual(parseActorId("actor:you"), { host: null, slug: "you" });
  assert.deepEqual(parseActorId("@vm1:actor:kestrel-2"), { host: "vm1", slug: "kestrel-2" });
  for (const bad of ["actor:", "actor:Wren", "claude:x", "@vm1:claude:x", "actor:a b", "xactor:a", null]) assert.ok(!isActorId(bad), String(bad));
  assert.ok(isActorId("actor:0") && isActorId(`actor:${"a".repeat(32)}`));
});

test("repo ids from git remotes, however they're written", () => {
  assert.equal(normalizeRemote, fromProjects, "the scan uses the same function");
  for (const url of ["git@github.com:willctl/xenonet.git", "https://github.com/willctl/xenonet.git", "https://user@github.com/willctl/xenonet", "ssh://git@github.com/willctl/xenonet.git", "git+https://github.com/WillCtl/XenoNet.git"]) assert.equal(repoId(url), "repo:github.com/willctl/xenonet", url);
  assert.equal(repoId(null), null);
  assert.equal(repoId(""), null);
  assert.equal(parseRepoId("repo:github.com/willctl/xenonet"), "github.com/willctl/xenonet");
  assert.equal(parseRepoId("project:/x"), null);
  assert.equal(parseRepoId("repo:"), null);
});
