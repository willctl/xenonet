// End to end: Codex's rate limits as the console's gauges show them, on a
// real XenoNet Dev (the test instance, with the demo's made-up data; see
// xenonet.test.js). /api/live sends them in XenoNet's own shape, each window's
// pct, windowMinutes and resetsAt in milliseconds, and the gauges read that
// shape.
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

// The Codex gauges as drawn, beside what they should say for `cx` (from
// /api/live), worked out in the page so dates read in its own locale.
const look = (cx) =>
  s.eval(`return (() => {
    const cx = ${JSON.stringify(cx)};
    const drawn = [...document.querySelectorAll("#gauges .gauge")].map((g) => ({ label: g.querySelector(".g-k")?.textContent, value: g.querySelector(".g-v")?.textContent, title: g.title })).filter((g) => g.label?.startsWith("codex"));
    const want = [
      { label: "codex · 5h", value: Math.round(cx.primary.pct) + "%" },
      { label: "codex · week", value: Math.round(cx.secondary.pct) + "%", title: "resets " + new Date(cx.secondary.resetsAt).toLocaleString() },
    ];
    return { drawn, want };
  })()`);

test("Codex's gauges show the windows /api/live sends in XenoNet's shape, and when the week resets", { timeout: limit(60000) }, shoot("limits-codex", async () => {
  let cx = null;
  for (const end = Date.now() + 20000; Date.now() < end && !cx?.primary; await sleep(300)) cx = (await s.api("/api/live")).limits?.codex ?? null;
  assert.ok(cx?.primary && cx?.secondary, "the demo's Codex sessions report both windows");
  for (const w of [cx.primary, cx.secondary]) {
    assert.deepEqual(Object.keys(w).sort(), ["pct", "resetsAt", "windowMinutes"]);
    assert.ok(w.resetsAt > Date.now() - 86400e3 && w.resetsAt < Date.now() + 30 * 86400e3, `resetsAt is in milliseconds: ${w.resetsAt}`);
  }
  assert.deepEqual([cx.primary.windowMinutes, cx.secondary.windowMinutes], [300, 10080]);

  // The page takes the live state every couple of seconds, and the demo's
  // readings move on: look until the page and the API agree.
  let seen;
  for (const end = Date.now() + 15000; Date.now() < end; await sleep(400)) {
    cx = (await s.api("/api/live")).limits.codex;
    seen = await look(cx);
    const [primary, week] = seen.want;
    const [p, w] = seen.drawn;
    if (p?.label === primary.label && p?.value === primary.value && w?.label === week.label && w?.value === week.value && w?.title === week.title) break;
  }
  assert.deepEqual(
    seen.drawn.map(({ label, value }) => ({ label, value })),
    seen.want.map(({ label, value }) => ({ label, value })),
  );
  assert.equal(seen.drawn[1].title, seen.want[1].title, "the week's reset time, read as milliseconds");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
