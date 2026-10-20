// End to end: XenoNet Dev's test instance, whose data is made up, observes
// nothing on the machine it runs on (core/actors/observers.js). However
// many of Hermes Agent's, Grok Bot's and Muse's processes run here, its actors are
// the demo's four, it has no observer among its parts, and its doctor
// leaves the actors' checks, which read this machine, alone. The window
// stays hidden; XENONET_E2E_SHOW=inactive shows it to watch.
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

// Runs a test body, and on failure saves what the window looked like.
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
  // One test instance at a time (it has a single-instance lock).
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("the test instance's actors are the demo's alone: no observer looks at this machine", { timeout: limit(60000) }, shoot("observers-off", async () => {
  const health = await s.api("/api/health");
  assert.equal(health.demo, true);
  assert.deepEqual(health.blocks.filter((b) => b.kind === "observer"), [], "no observer among its parts");
  // A few seconds, so an observer would have looked by now if there were one.
  await new Promise((r) => setTimeout(r, 6000));
  const ids = (await s.api("/api/actors")).actors.map((a) => a.id).sort();
  assert.deepEqual(ids, ["actor:kestrel", "actor:moth", "actor:wren", "actor:you"]);
  // The observers' switches still answer, for the demo's own Hermes and Grok
  // Bot, and Muse's, which no actor of the demo's is.
  assert.deepEqual((await s.api("/api/actors/observers")).observers.map((o) => [o.product, o.actors]), [["hermes-agent", ["actor:wren"]], ["grok-bot", ["actor:kestrel"]], ["muse", []]]);
  const doctor = await (await fetch(`${s.base}/api/doctor`, { method: "POST" })).json();
  assert.deepEqual(doctor.results.filter((r) => r.id.startsWith("actors.")), [], "the demo's doctor checks only itself");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
