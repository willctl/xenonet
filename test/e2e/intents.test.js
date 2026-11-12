import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../harness/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => { const { port } = probe.address(); probe.close(() => resolve(port)); });
});
const s = xenonet("test", { port });
before(async () => { await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" }); }, { timeout: limit(180000) });
after(async () => {
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

const status = () => s.eval('return document.querySelector("[data-in-status]").textContent');

test("the intents view lists the person's intents, drafts one, and leaves Make and Cancel to the person", { timeout: limit(60000) }, async () => {
  assert.ok((await s.driver("commands")).some((c) => c.id === "intents"));
  await s.driver("command", "intents");
  let st = (await s.driver("intents", "refresh")).intents;
  assert.equal(st.open, true);
  assert.deepEqual(st.intents, []);
  assert.equal((await s.driver("state")).overlays.modal, true);
  // The driver fills the form in; it never sends it.
  st = (await s.driver("intents", "draft", { goal: "Tidy the docs", app: "codex", hours: 2 })).intents;
  assert.deepEqual([st.draft.goal, st.draft.app, st.draft.hours], ["Tidy the docs", "codex", 2]);
  assert.equal(await s.eval('return document.querySelector("[data-in-goal]").value'), "Tidy the docs");
  // Any machine the delegation names is a target too.
  st = (await s.driver("intents", "draft", { target: "any" })).intents;
  assert.equal(st.draft.target, "any");
  assert.equal(await s.eval('return document.querySelector("[data-in-target]").value'), "any");
  assert.equal(await s.eval('return document.querySelector("[data-in-arbiter]") !== null'), true);
  await assert.rejects(s.driver("intents", "make"), /Unknown intents action/);
  await assert.rejects(s.driver("intents", "cancel", "int_x"), /Unknown intents action/);
  assert.equal(await s.eval('return document.querySelector("[data-in-make]").hasAttribute("data-yours")'), true);
  // The person's click: this instance is in no network, so the intent
  // isn't made, and the view says why.
  const at = await s.eval('const b = document.querySelector("[data-in-make]"); b.scrollIntoView({ block: "center" }); const r = b.getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };');
  await s.input({ click: at });
  const end = Date.now() + 5000;
  while (!/network/.test(await status()) && Date.now() < end) await new Promise((r) => setTimeout(r, 100));
  assert.match(await status(), /network/);
  assert.deepEqual((await s.driver("intents", "refresh")).intents.intents, []);
  await s.screenshot({ name: "e2e-intents", inline: false });
  st = (await s.driver("intents", "close")).intents;
  assert.equal(st.open, false);
  const logs = await s.logs({ level: "error" });
  assert.deepEqual(logs.entries.filter((e) => e.source === "page"), []);
});
