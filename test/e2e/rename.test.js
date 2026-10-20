// End to end: renaming a session on a real XenoNet Dev (the test instance,
// with the demo's made-up data; see xenonet.test.js), by its keys as a person
// would: F2 or the palette opens its name in the inspector, Enter saves it,
// Esc puts it back. The demo keeps the names of its sessions itself.
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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const api = (route) => fetch(`http://127.0.0.1:${port}${route}`).then((r) => r.json());
const titleOf = async (id) => (await api("/api/graph")).nodes.find((n) => n.id === id)?.title;
// The inspector's title, or the field in its place, as the page shows it.
const shown = () => s.eval(`return { title: document.querySelector("#inspector h2.insp-title")?.textContent ?? null, field: document.querySelector("#inspector .rename-input")?.value ?? null, focused: document.activeElement?.matches(".rename-input") ?? false, where: document.querySelector("#inspector .rename-where")?.textContent ?? null }`);

async function until(fn, want, ms = 8000) {
  const end = Date.now() + ms;
  let got;
  while (Date.now() < end && !want((got = await fn()))) await sleep(150);
  return got;
}

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

let session;

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  ({ nodes: [session] } = await s.driver("find", { query: "", kind: "session", limit: 1 }));
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("F2 opens the selected session's name, Enter saves it, and the map shows it", { timeout: limit(60000) }, shoot("rename-f2", async () => {
  await s.driver("select", session.id);
  const before = await shown();
  assert.equal(before.title, session.title, "the inspector shows the session's name");
  assert.ok(await s.eval(`return !!document.querySelector("#inspector [data-rename]")`), "with a pencil beside it");

  await s.input({ keys: "F2" });
  const open = await shown();
  assert.deepEqual([open.title, open.field, open.focused], [null, session.title, true], "the name becomes a field, focused");
  assert.equal(open.where, "The demo keeps the new name in XenoNet");
  assert.equal((await s.driver("state")).overlays.rename, true);

  await s.input({ text: "Refunds:  final pass " });
  await s.input({ keys: "Enter" });
  assert.equal(await until(() => titleOf(session.id), (t) => t === "Refunds: final pass"), "Refunds: final pass", "saved as one line, its spaces tidied");
  const saved = await until(shown, (x) => x.title === "Refunds: final pass");
  assert.deepEqual([saved.title, saved.field], ["Refunds: final pass", null], "the inspector shows the new name");
  assert.equal((await s.driver("state")).selection.title, "Refunds: final pass");
  assert.equal(await s.eval(`return document.querySelector("#toast").textContent`), "Renamed in the demo");
}));

test("Esc puts the name back and keeps the selection; an empty name isn't taken", { timeout: limit(60000) }, shoot("rename-cancel", async () => {
  await s.input({ keys: "F2" });
  await s.input({ text: "Not this" });
  await s.input({ keys: "Escape" });
  const back = await shown();
  assert.deepEqual([back.title, back.field], ["Refunds: final pass", null]);
  assert.equal((await s.driver("state")).selection?.id, session.id, "Esc closed the field, not the inspector");

  await s.input({ keys: "F2" });
  await s.input({ keys: "Delete" });
  await s.input({ keys: "Enter" });
  const refused = await shown();
  assert.deepEqual([refused.field, refused.where], ["", "A session needs a name"], "it stays open and says why");
  await s.input({ keys: "Escape" });
  assert.equal(await titleOf(session.id), "Refunds: final pass", "nothing was saved");
}));

test("the palette renames the selected session, and the driver does too", { timeout: limit(60000) }, shoot("rename-palette", async () => {
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "> rename" });
  await s.input({ keys: "Enter" });
  const open = await until(shown, (x) => x.field != null);
  assert.deepEqual([open.field, open.focused], ["Refunds: final pass", true], "the Enter that chose it isn't in the name");
  await s.input({ keys: "Escape" });

  const st = await s.driver("rename", session.id, session.title);
  assert.deepEqual([st.renamed.title, st.renamed.demo, st.renamed.saved], [session.title, true, []]);
  assert.equal(await until(() => titleOf(session.id), (t) => t === session.title), session.title, "back to its own name");
  await assert.rejects(s.driver("rename", session.id, "  "), /needs a name|can't be empty/);
}));

test("no page errors", { timeout: limit(30000) }, async () => {
  const { entries } = await s.logs({ level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
