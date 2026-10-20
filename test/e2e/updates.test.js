import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const s = xenonet("test");
before(async () => { await s.quit(); await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" }); }, { timeout: limit(180000) });
after(async () => { await s.quit().catch(() => {}); });

test("Updates is reachable from the palette and driver, and source builds cannot update", { timeout: limit(60000) }, async () => {
  await s.driver("command", "updates");
  const state = await s.driver("updates");
  assert.equal(state.status, "development");
  assert.equal((await s.driver("updates", "check")).status, "development");
  assert.equal(await s.eval('return document.querySelector("[data-update=check]").disabled'), true);
  assert.match(await s.eval('return document.querySelector("#modal").textContent'), /running XenoNet from source/);
  await s.screenshot({ name: "e2e-updates", inline: false });
  assert.deepEqual((await s.logs({ source: "page", level: "error" })).entries, []);
});

test("the update panel renders progress, restart and manual Mac downloads", { timeout: limit(60000) }, async () => {
  await s.eval(`
    const { createUpdatesPanel } = await import('/src/updates.js');
    window.updatePreview = { status: 'downloading', current: '0.5.0', version: '0.6.0', progress: 42, automaticInstall: true, automaticChecks: true };
    const api = { act: async () => window.updatePreview, onState: (fn) => { window.updatePreviewChanged = fn; return () => {}; } };
    window.updatePanel = createUpdatesPanel({ modals: window.__xenonet.modals, desktop: { updates: api }, toast: () => {} });
    await window.updatePanel.open();
  `);
  assert.match(await s.eval('return document.querySelector("#modal").textContent'), /42%/);
  assert.equal(await s.eval('return document.querySelector("[data-update=check]").disabled'), true);
  await s.eval('window.updatePreviewChanged({ ...window.updatePreview, status: "ready" })');
  assert.equal(await s.eval('return document.querySelector("[data-update=install]").textContent'), "Restart and update");
  await s.screenshot({ name: "e2e-update-ready", inline: false });
  await s.eval('window.updatePreviewChanged({ ...window.updatePreview, status: "available", automaticInstall: false })');
  assert.match(await s.eval('return document.querySelector("#modal").textContent'), /manual install/);
  assert.equal(await s.eval('return !!document.querySelector("[data-update=download]")'), false);
  await s.input({ keys: "Escape" });
});
