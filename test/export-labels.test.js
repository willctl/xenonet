// The map's own labels during an export (core/export-labels.js): a star
// name on every labelled kind of node, a slash after a group's, and no
// label on a session or on a node the names don't cover.
import { test } from "node:test";
import assert from "node:assert/strict";
import { exportLabel, LABELLED_KINDS } from "../core/export-labels.js";

const labels = { "core": "Sugus", "project:/home/dana/projects/atlas-api": "Kevis", "project:/home/dana/projects": "Thalor", "zone:/home/dana/clients": "Mira", "page:refunds": "Doun", "resource:github": "Vrae", "host:dana-mbp.local": "Shaeshushi", "native:codex-1": "Lior", "wiki:/home/dana/wiki": "Feim" };

test("every labelled kind gets its star name in capitals, and a group its slash", () => {
  assert.equal(exportLabel({ id: "core", kind: "core", name: "Projects" }, labels), "SUGUS");
  assert.equal(exportLabel({ id: "project:/home/dana/projects/atlas-api", kind: "project", name: "atlas-api" }, labels), "KEVIS");
  assert.equal(exportLabel({ id: "project:/home/dana/projects", kind: "project", name: "projects", group: true }, labels), "THALOR /");
  assert.equal(exportLabel({ id: "zone:/home/dana/clients", kind: "zone", name: "clients" }, labels), "MIRA");
  assert.equal(exportLabel({ id: "page:refunds", kind: "page", name: "Partial refunds" }, labels), "DOUN");
  assert.equal(exportLabel({ id: "resource:github", kind: "resource", name: "github" }, labels), "VRAE");
  assert.equal(exportLabel({ id: "host:dana-mbp.local", kind: "host", name: "dana-mbp.local" }, labels), "SHAESHUSHI");
  assert.equal(exportLabel({ id: "native:codex-1", kind: "native", name: "Codex" }, labels), "LIOR");
  assert.equal(exportLabel({ id: "wiki:/home/dana/wiki", kind: "wiki", name: "Agent Wiki" }, labels), "FEIM");
  for (const kind of LABELLED_KINDS) assert.ok(["core", "project", "zone", "native", "wiki", "page", "resource", "host"].includes(kind), kind);
});

test("a session, a node the names don't cover, and nothing at all stay hidden", () => {
  assert.equal(exportLabel({ id: "claude:0190", kind: "session", title: "Add partial refunds to the orders API" }, labels), null);
  assert.equal(exportLabel({ id: "project:/home/dana/projects/harbor", kind: "project", name: "harbor" }, labels), null, "a project without a name");
  assert.equal(exportLabel({ id: "actor:hermes", kind: "actor", name: "Hermes" }, labels), null, "actors aren't drawn by the map's labels");
  assert.equal(exportLabel({ id: "core", kind: "core" }, null), null);
  assert.equal(exportLabel(null, labels), null);
  assert.equal(exportLabel({ id: "core", kind: "core" }, { core: 7 }), null, "only a string is a name");
});
