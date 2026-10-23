// End to end: Grok Build's sessions on a real XenoNet Dev (the test instance,
// with the demo's made-up data; see xenonet.test.js): their toggle under
// Filters hides and shows them, the legend and the map's colors name them,
// and the inspector and stats say what XenoNet knows of one.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("Grok Build's sessions are on the map, with a toggle of their own under Filters that hides and shows them", { timeout: limit(60000) }, shoot("grok-filter", async () => {
  let st = await s.driver("state");
  assert.equal(st.counts.apps.grok.sessions, 3, "the demo's three");
  assert.ok(st.counts.apps.grok.drawn > 0, "drawn");
  assert.equal(st.filters.grok, true);
  const toggle = () => s.eval(`const b = document.querySelector('[data-filter="grok"]'); return { hidden: b.hidden, on: b.classList.contains("on"), text: b.textContent, color: getComputedStyle(b.querySelector(".sw")).backgroundColor }`);
  let t = await toggle();
  assert.deepEqual([t.hidden, t.on, t.text], [false, true, "Grok Build3"], "shown once there are Grok Build sessions, with their count");
  assert.equal(t.color, "rgb(198, 255, 58)", "in XenoNet's Grok Build color");

  st = await s.driver("filters", { grok: false });
  assert.equal(st.filters.grok, false);
  assert.equal(st.counts.apps.grok.drawn, 0, "hidden from the map");
  assert.ok(st.counts.apps.claude.drawn > 0 && st.counts.apps.codex.drawn > 0, "the others stay");
  t = await toggle();
  assert.deepEqual([t.hidden, t.on], [false, false], "the toggle stays, off, to bring them back");

  // A real click on it, as a person would.
  await s.eval(`document.querySelector('[data-filter="grok"]').click()`);
  st = await s.driver("state");
  assert.equal(st.filters.grok, true);
  assert.ok(st.counts.apps.grok.drawn > 0, "back on the map");

  const legend = await s.eval(`return [...document.querySelectorAll("#legend .legend-group span")].map((x) => x.textContent.trim())`);
  assert.ok(legend.includes("Grok Build"), "the legend names them");
}));

test("a Grok Build session's inspector: its app, model, branch and subagent, and its stats", { timeout: limit(60000) }, shoot("grok-inspector", async () => {
  const { nodes } = await s.driver("find", { query: "dark mode", kind: "session", limit: 1 });
  assert.equal(nodes[0]?.app, "grok");
  const st = await s.driver("select", nodes[0].id);
  assert.equal(st.selection.id, nodes[0].id);
  const rows = () => s.eval(`return Object.fromEntries([...document.querySelectorAll("#inspector .kv")].map((r) => [r.querySelector(".k")?.textContent?.trim(), r.querySelector(".v")?.textContent?.trim()]))`);
  const chip = await s.eval(`return document.querySelector("#inspector .panel-title .chip")?.textContent`);
  assert.match(chip, /Session · Grok Build/);
  const insp = await rows();
  assert.equal(insp.model, "grok-build");
  assert.equal(insp.branch, "feat/dark-mode");
  assert.equal(insp.origin, "Grok Build");

  // Its subagent says whose it is.
  const sub = (await s.driver("find", { query: "contrast", kind: "session", limit: 1 })).nodes[0];
  await s.driver("select", sub.id);
  const subRows = await rows();
  assert.match(subRows.origin, /^Grok Build subagent/);
  assert.match(subRows.parent, /Add dark mode to the storefront/);

  // Its stats, read from its session updates: its turns and tools. Grok
  // Build's lines don't carry tokens, so it has no cost.
  const opened = await s.driver("stats", nodes[0].id);
  assert.equal(opened.overlays.modal, true);
  assert.ok(opened.stats?.turns > 0 && opened.stats.tools > 0, JSON.stringify(opened.stats));
  assert.match(await s.eval(`return document.querySelector(".st .m-chip")?.textContent`), /stats · Grok Build/);
  await s.input({ keys: "Escape" });
  await s.driver("select", null);

  const { entries } = await s.logs({ level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), [], "no page errors");
}));
