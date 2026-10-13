import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => { const probe = net.createServer().listen(0, "127.0.0.1", () => { const { port } = probe.address(); probe.close(() => resolve(port)); }); });
const s = xenonet("test", { port });
before(async () => { await s.launch({ show: "hidden" }); }, { timeout: 180000 });
after(async () => { await s.quit().catch(() => {}); fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 }); });
async function click(selector) {
  const spot = await s.eval(`const el = document.querySelector(${JSON.stringify(selector)}); el.scrollIntoView({ block: "center" }); const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 };`);
  return s.input({ click: spot });
}
async function until(check) {
  const end = Date.now() + 15000;
  while (Date.now() < end) { if (await check()) return; await new Promise((r) => setTimeout(r, 100)); }
  throw new Error("Integration view did not reach the expected state.");
}
const view = async () => (await s.driver("integrations", "refresh")).integrations;

test("routing command previews exact input, invalidates edits, records scores and outcomes, and filters history", { timeout: 60000 }, async () => {
  const commands = await s.driver("commands");
  for (const id of ["route-task", "decisions", "sandbox", "pi-runs"]) assert.ok(commands.some((c) => c.id === id));
  await s.driver("command", "route-task");
  assert.equal((await view()).total, 1);
  await s.driver("integrations", "draft", { request: "Inspect this made-up repository" });
  await click("[data-in-preview]");
  await until(async () => (await view()).preview);
  assert.match(await s.eval('return document.querySelector("[data-in-preview-body]").textContent'), /Inspect this made-up repository/);
  assert.match(await s.eval('return document.querySelector("[data-in-preview-body]").textContent'), /"zdr": true/);
  await s.eval('const input = document.querySelector("textarea[name=request]"); input.value = "Changed demo task"; input.dispatchEvent(new Event("input", { bubbles: true })); return true;');
  assert.equal((await view()).preview, false);
  assert.equal(await s.eval('return document.querySelector("[data-in-confirm]").disabled'), true);
  await click("[data-in-preview]"); await until(async () => (await view()).preview);
  await click("[data-in-confirm]"); await until(async () => (await view()).total === 2);
  let st = await view(); const record = st.records.find((r) => r.taskHash !== "demo");
  assert.equal(record.selected, "codex"); assert.equal(record.status, "needs_confirmation"); assert.equal(JSON.stringify(record).includes("Changed demo task"), false);
  await s.driver("integrations", "select", record.id);
  await s.eval('const input = document.querySelector("[data-in-outcome]"); input.value = "useful"; input.dispatchEvent(new Event("change", { bubbles: true })); return true;');
  await until(async () => (await view()).records.find((r) => r.id === record.id).outcome === "useful");
  st = (await s.driver("integrations", "filter", { q: "not a record" })).integrations; assert.equal(st.matched, 0);
  assert.match(await s.eval('return document.querySelector("[data-in-list]").textContent'), /No records match/);
  st = (await s.driver("integrations", "filter", { q: "", status: "needs_confirmation", sort: "oldest" })).integrations;
  assert.equal(st.matched, 2); await s.screenshot({ name: "e2e-integrations-routing", inline: false });
});

test("sandbox preview, detached launch, policy inspection and reviewed deletion work with the demo gateway", { timeout: 60000 }, async () => {
  await s.driver("command", "sandbox");
  await s.driver("integrations", "draft", { backend: process.platform === "win32" ? "wsl" : "native", distro: "Ubuntu", agent: "pi", image: "demo/pi:1", provider: "demo-provider", request: "Review a made-up workspace" });
  await click("[data-in-check]");
  await until(async () => (await s.eval('return document.querySelector("[data-in-message]").textContent')).includes("gateway ready"));
  await click("[data-in-preview]"); await until(async () => (await view()).preview);
  assert.match(await s.eval('return document.querySelector("[data-in-preview-body]").textContent'), /--detach/);
  await click("[data-in-confirm]"); await until(async () => (await view()).total === 1);
  const record = (await view()).records[0]; assert.equal(record.kind, "sandbox"); assert.equal(record.status, "created");
  await s.driver("integrations", "select", record.id);
  await click("[data-in-inspect]");
  await until(async () => (await s.eval('return document.querySelector("[data-in-inspection]").textContent')).includes("execution-policy-denial"));
  assert.match(await s.eval('return document.querySelector("[data-in-inspection]").textContent'), /not a model refusal/);
  await s.screenshot({ name: "e2e-integrations-sandbox", inline: false });
  await click("[data-in-delete-preview]"); assert.equal((await view()).records[0].status, "observed");
  // Refresh replaces the confirmation. Review it again, then delete.
  await click("[data-in-delete-preview]"); await click("[data-in-delete]");
  await until(async () => (await view()).records[0].status === "deleted");
});

test("Pi command exposes metadata, opt-in controls and safe HTML, and console actions cannot execute", { timeout: 60000 }, async () => {
  await s.driver("command", "pi-runs"); const st = await view();
  assert.equal(st.records[0].kind, "pi-export"); assert.equal(st.total, 1);
  await s.driver("integrations", "select", st.records[0].id);
  assert.match(await s.eval('return document.querySelector("[data-in-detail]").textContent'), /demo-pi-run/);
  await click("[data-in-enable]");
  await until(async () => (await s.eval('return document.querySelector("[data-in-form]").textContent')).includes("reading enabled"));
  await s.eval('const input = document.querySelector("[data-in-import]"); const transfer = new DataTransfer(); transfer.items.add(new File([JSON.stringify({ run: { id: "demo-imported-run", status: "completed", model: "demo", prompt: "PRIVATE_IMPORT" }, trace: { raw: "PRIVATE_IMPORT" } })], "demo-run.json", { type: "application/json" })); input.files = transfer.files; input.dispatchEvent(new Event("change", { bubbles: true })); return true;');
  await until(async () => (await view()).total === 2);
  assert.equal(JSON.stringify((await view()).records).includes("PRIVATE_IMPORT"), false);
  await s.eval('const input = document.querySelector("[data-in-import]"); const transfer = new DataTransfer(); transfer.items.add(new File(["invalid JSON"], "bad.json")); input.files = transfer.files; input.dispatchEvent(new Event("change", { bubbles: true })); return true;');
  await until(async () => !!(await s.eval('return document.querySelector("[data-in-message]").textContent')));
  assert.equal((await view()).total, 2);
  await s.driver("integrations", "filter", { q: "<img onerror=alert(1)>" });
  assert.equal(await s.eval('return document.querySelectorAll(".in-panel img").length'), 0);
  await s.screenshot({ name: "e2e-integrations-pi", inline: false });
  await assert.rejects(s.driver("integrations", "launch", { request: "blocked" }), /Unknown integration browser action/);
  const logs = await s.logs({ level: "error" }); assert.deepEqual(logs.entries.filter((e) => e.source === "page"), []);
});
