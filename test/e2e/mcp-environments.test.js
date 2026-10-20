import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => { const p = net.createServer().listen(0, "127.0.0.1", () => { const port = p.address().port; p.close(() => resolve(port)); }); });
const s = xenonet("test", { port });
before(async () => { await xenonet("test").quit(); await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" }); }, { timeout: limit(180000) });
after(async () => { await s.quit().catch(() => {}); });

test("focusing an MCP system shows metadata; tools and context are accessible without execution", { timeout: limit(60000) }, async () => {
  await s.driver("select", "resource:reference-library");
  const st = await s.driver("mcp", "resource:reference-library");
  assert.equal(st.mcp.info.author, "XenoNet demo");
  assert.equal(st.mcp.info.status, "connected");
  assert.equal(st.mcp.info.tools.length, 2);
  assert.equal(await s.eval('const row = document.querySelector("#insp-mcp .kv"); const label = document.createRange(); label.selectNodeContents(row.querySelector(".k")); return label.getBoundingClientRect().right <= row.querySelector(".v").getBoundingClientRect().left'), true, "the authentication label leaves room for its value");
  await s.driver("mcpTab", "tools");
  await s.driver("mcpSearch", "search");
  const names = await s.eval('return [...document.querySelectorAll(".mcp-tool summary code")].map((e) => e.textContent)');
  assert.deepEqual(names, ["search"]);
  await s.eval('document.querySelector(".mcp-tool").open = true');
  assert.match(await s.eval('return document.querySelector(".mcp-tool pre").textContent'), /"query"/);
  await s.driver("mcpTab", "context");
  assert.match(await s.eval('return document.getElementById("mcp-tab-content").textContent'), /library:\/\/topics\/\{topic\}/);
  await s.screenshot({ name: "e2e-mcp-context", inline: false });
  await s.input({ keys: "Escape" });
});

test("all environments draw across themes, persist independently, and never reload the page", { timeout: limit(180000) }, async () => {
  const origin = await s.eval("return performance.timeOrigin");
  for (const theme of ["xenonet", "phosphor", "ochre"]) {
    await s.driver("theme", theme);
    for (const id of ["aurora", "trench", "vault"]) {
      const st = await s.driver("environment", id);
      assert.equal(st.environment.id, id);
      assert.equal(st.theme.id, theme);
      assert.equal((await s.api("/api/environment")).current, id);
      assert.equal(await s.eval("return performance.timeOrigin"), origin);
      await s.screenshot({ name: `e2e-environment-${theme}-${id}`, inline: false });
    }
  }
  await s.driver("theme", "xenonet");
  assert.equal((await s.driver("state")).environment.id, "vault");
  const { entries } = await s.logs({ source: "page", level: "error" });
  assert.deepEqual(entries, []);
});

test("the environment cards preview with keyboard, cancel with Escape, and keep with Enter", { timeout: limit(60000) }, async () => {
  await s.driver("environment", "nebula");
  await s.driver("environmentPicker");
  await s.input({ keys: "ArrowRight" });
  let st = await s.driver("state");
  assert.equal(st.environment.id, "aurora");
  assert.equal(st.environment.inUse, "nebula");
  assert.equal((await s.api("/api/environment")).current, "nebula");
  await s.screenshot({ name: "e2e-environment-picker", inline: false });
  await s.input({ keys: "Escape" });
  assert.equal((await s.driver("state")).environment.id, "nebula");
  await s.driver("environmentPicker");
  await s.input({ keys: "End" });
  await s.input({ keys: "Enter" });
  // Enter keeps it with a request the server first checks came from you, by
  // looking up the caller's process, so it can land after the key returns.
  for (const until = Date.now() + 10000; (st = await s.driver("state")).environment.inUse !== "vault" && Date.now() < until;) await new Promise((r) => setTimeout(r, 100));
  assert.equal(st.environment.inUse, "vault");
  assert.equal(st.environment.preview, null);
  assert.equal(st.environment.picker, false);
});
