import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => { const { port } = probe.address(); probe.close(() => resolve(port)); });
});
const s = xenonet("test", { port });
before(async () => { await s.launch({ show: "hidden" }); }, { timeout: 180000 });
after(async () => {
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("the refusal command opens a searchable browser with outcome filters and details", { timeout: 60000 }, async () => {
  assert.ok((await s.driver("commands")).some((c) => c.id === "refusals" && c.label === "Review provider refusals"));
  await s.driver("command", "refusals");
  let st = (await s.driver("refusals", "refresh")).refusals;
  assert.equal(st.total, 4);
  assert.equal(st.matched, 4);
  st = (await s.driver("refusals", "filter", { provider: "openai", kind: "possible-refusal" })).refusals;
  assert.equal(st.matched, 1);
  await s.driver("refusals", "select", st.records[0].id);
  assert.match(await s.eval('return document.querySelector("[data-rf-detail]").textContent'), /does not confirm a refusal/);
  st = (await s.driver("refusals", "filter", { provider: "", kind: "", sort: "oldest", context: "with" })).refusals;
  assert.equal(st.matched, 1);
  await s.driver("refusals", "select", st.records[0].id);
  await s.driver("refusals", "request");
  assert.match(await s.eval('return document.querySelector("[data-rf-request-body]").textContent'), /made-up demo request/);
  await s.driver("refusals", "filter", { q: "not in any record" });
  assert.match(await s.eval('return document.querySelector("[data-rf-results]").textContent'), /No records match/);
  assert.equal(await s.eval('return document.querySelector("[data-rf-next]").disabled'), true);
  await s.screenshot({ name: "e2e-refusals-empty", inline: false });
  await s.driver("refusals", "filter", { q: "", context: "", sort: "newest" });
  // A DOM input event exercises the form's debounce path.
  await s.eval('const input = document.querySelector("[name=q][data-rf-filter]"); input.value = "invalid_prompt"; input.dispatchEvent(new Event("input", { bubbles: true })); return true;');
  const end = Date.now() + 5000;
  while (Date.now() < end) {
    const count = await s.eval('return document.querySelectorAll("[data-rf-id]").length');
    if (count === 1) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.equal(await s.eval('return document.querySelectorAll("[data-rf-id]").length'), 1);
  await s.input({ keys: "Escape" });
  const closeDeadline = Date.now() + 5000;
  while ((await s.driver("state")).overlays.modal && Date.now() < closeDeadline) await new Promise((r) => setTimeout(r, 100));
  assert.equal((await s.driver("state")).overlays.modal, false);
  const logs = await s.logs({ level: "error" });
  assert.deepEqual(logs.entries.filter((e) => e.source === "page"), []);
});
