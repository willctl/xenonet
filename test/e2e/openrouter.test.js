import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => { const probe = net.createServer((socket) => socket.destroy()).listen(0, "127.0.0.1", () => { const { port } = probe.address(); probe.close(() => resolve(port)); }); });
// A separate frozen, synthetic instance keeps this fixture's browser cache
// apart from the other tests while preserving their existing profiles.
const s = xenonet("showcase", { port });
before(async () => { await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden", restart: true }); }, { timeout: limit(180000) });
after(async () => { await s.quit().catch(() => {}); await fs.promises.rm(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 20, retryDelay: 100 }); });

test("Token use keeps OpenRouter visible without a key, opens setup, and refreshes error/usage states", { timeout: limit(60000) }, async () => {
  await s.eval('window.__originalSpendFetch = window.fetch; window.__spendFixture = { key: "missing" }; window.fetch = (url, ...args) => url === "/api/spend" ? Promise.resolve(new Response(JSON.stringify({ openrouter: window.__spendFixture }), { status: 200, headers: { "content-type": "application/json" } })) : window.__originalSpendFetch(url, ...args); return true;');
  try {
    let value = await s.driver("spend", "refresh"); assert.match(value.gauge, /openrouter.*Set key/i);
    await s.eval('const el = document.querySelector("[data-openrouter-setup]"); el.scrollIntoView({ block: "center", behavior: "instant" }); el.focus(); return true;');
    await s.input({ keys: "Enter" });
    const deadline = Date.now() + 5000;
    while (!(await s.eval('return !!document.querySelector("[data-dk-key]")')) && Date.now() < deadline) await new Promise((r) => setTimeout(r, 100));
    assert.equal(await s.eval('return !!document.querySelector("[data-dk-key]")'), true);
    await s.input({ keys: "Escape" });
    await s.eval('window.__spendFixture = { key: "rejected", error: "OpenRouter rejected the demo key" }; return true;');
    value = await s.driver("spend", "refresh"); assert.match(value.gauge, /Check key/i);
    await s.eval('window.__spendFixture = { key: "stored", error: "OpenRouter did not answer" }; return true;');
    value = await s.driver("spend", "refresh"); assert.match(value.gauge, /Unavailable/i);
    await s.eval('window.__spendFixture = { key: "stored", today: 1, week: 5, month: 12, limit: 20, remaining: 8, reset: "monthly" }; return true;');
    value = await s.driver("spend", "refresh"); assert.match(value.gauge, /\$12\.00/);
    assert.equal(await s.eval('return !!document.querySelector("[data-openrouter-setup]")'), false);
    await s.screenshot({ name: "e2e-openrouter-usage", inline: false });
  } finally { await s.eval('window.fetch = window.__originalSpendFetch; delete window.__originalSpendFetch; delete window.__spendFixture; return true;'); }
  const logs = await s.logs({ level: "error" }); assert.deepEqual(logs.entries.filter((e) => e.source === "page"), []);
});
