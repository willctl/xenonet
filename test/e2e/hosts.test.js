// End to end: other machines on the map, on a real XenoNet Dev (the test
// instance, with the demo's made-up data, started with XENONET_HOSTS=1; see
// xenonet.test.js). A second demo XenoNet on a spare port stands in for
// another machine, shown through its relay as the local transport runs it:
// XenoNet Dev's host-add simulation adds it, since adding a host for real is
// refused from under an agent and needs SSH. Both worlds are made up
// alike, so every repo has a checkout on both machines.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet, root } from "../../core/control.js";

// The test instance takes this from the environment it starts in.
process.env.XENONET_HOSTS = "1";

const freePort = () =>
  new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
const [port, hostPort] = [await freePort(), await freePort()];
const s = xenonet("test", { port });
const HOST = "demo-b";
const HOST_NODE = `@${HOST}:core`;
const tmp = fs.realpathSync.native(os.tmpdir());
const home = fs.mkdtempSync(path.join(tmp, "xenonet-e2e-hosts-home-"));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let other = null;
let otherLog = "";

// Waits for `get` to give what `want` says, and returns what it gave last.
async function until(get, want, ms = 20000) {
  let got;
  for (const end = Date.now() + ms; !want((got = await get())) && Date.now() < end; ) await sleep(250);
  return got;
}
const hostOf = async () => (await s.driver("hosts")).hosts.find((x) => x.id === HOST) ?? null;
const findRemote = async (query, kind) => (await s.driver("find", { query, kind, limit: 50 })).nodes.find((n) => n.host === HOST) ?? null;

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    if (otherLog) err.message += `\n(the other XenoNet said: ${otherLog.slice(-1500)})`;
    throw err;
  }
};

// Back to the overview, with every machine shown.
async function home$() {
  await s.input({ keys: "Escape", move: { x: 4, y: 4 } });
  await s.driver("filterHosts", []);
  return s.driver("select", null);
}

before(async () => {
  await xenonet("test").quit();
  // The other machine: a plain demo XenoNet without the flag, as any host
  // runs, with a home of its own.
  other = spawn(process.execPath, ["server/index.js", "--demo"], {
    cwd: root,
    env: { ...process.env, HOME: home, USERPROFILE: home, XENONET_PORT: String(hostPort), XENONET_HOSTS: "", XENONET_INSTANCE: "", XENONET_CONTROL: "", XENONET_EXEC: "", XENONET_HOT: "0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  other.stdout.on("data", (d) => (otherLog += d));
  other.stderr.on("data", (d) => (otherLog += d));
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  const ready = await until(async () => (await fetch(`http://127.0.0.1:${hostPort}/api/graph`).then((r) => r.json()).catch(() => null))?.nodes?.some((n) => n.kind === "session"), Boolean, 60000);
  assert.ok(ready, `the other XenoNet didn't start: ${otherLog.slice(-1500)}`);
  const added = await s.simulate("host-add", { a: HOST, b: String(hostPort) });
  assert.equal(added.ok, true, JSON.stringify(added));
  const live = await until(hostOf, (h) => h?.state === "live" && h.drawn, 60000);
  assert.equal(live?.state, "live", `the host didn't come up live: ${JSON.stringify(live)}`);
  await s.driver("settled", { timeout: 30000 });
}, { timeout: 240000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP !== "1") await s.quit().catch(() => {});
  if (other && other.exitCode === null) {
    other.kill("SIGTERM");
    await new Promise((r) => other.once("exit", r));
  }
  if (process.env.XENONET_E2E_KEEP === "1") return;
  for (const p of [port, hostPort]) fs.rmSync(path.join(tmp, `xenonet-demo-${p}`), { recursive: true, force: true, maxRetries: 10 });
  fs.rmSync(home, { recursive: true, force: true, maxRetries: 10 });
});

test("the other machine is a constellation of its own beside this one's, with bridges between checkouts of one repo", { timeout: 60000 }, shoot("hosts-map", async () => {
  await home$();
  const { hosts, bridges, chips } = await s.driver("hosts");
  const h = hosts.find((x) => x.id === HOST);
  assert.deepEqual([h.state, h.node, h.drawn, h.demo, h.stale], ["live", HOST_NODE, true, true, false]);
  assert.ok(h.sessions > 0 && h.constellation?.nodes > 5, JSON.stringify(h));
  assert.ok(bridges.total > 0 && bridges.drawn > 0, `bridges: ${JSON.stringify(bridges)}`);
  assert.deepEqual(chips.map((c) => c.key), ["@here", HOST]);
  const st = await s.driver("state");
  assert.deepEqual(st.hosts, [{ id: HOST, state: "live" }]);
  // Beside the map, not on its rings: the host node is further out than
  // any of this machine's nodes, and its own nodes are nearer it than to
  // this machine's core.
  const where = await s.eval(`return (() => {
    const nodes = window.__xenonet.scene.nodes();
    const r = (n) => Math.hypot(n.x, n.y, n.z);
    const host = nodes.find((n) => n.id === ${JSON.stringify(HOST_NODE)});
    const mine = nodes.filter((n) => !n.host);
    const theirs = nodes.filter((n) => n.host && n.kind !== "host");
    const near = theirs.filter((n) => Math.hypot(n.x - host.x, n.y - host.y, n.z - host.z) < r(n)).length;
    return { host: r(host), widest: Math.max(...mine.map(r)), theirs: theirs.length, near };
  })()`);
  assert.ok(where.host > where.widest, JSON.stringify(where));
  assert.equal(where.near, where.theirs, JSON.stringify(where));
  const found = await s.driver("find", { query: HOST, kind: "host" });
  assert.equal(found.nodes[0]?.id, HOST_NODE);
  assert.equal(st.errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("a host you're not looking at is drawn only in part, and looking at it draws all of it", { timeout: 60000 }, shoot("hosts-cap", async () => {
  await home$();
  const chipOf = async () => (await s.driver("hosts")).chips.find((c) => c.key === HOST);
  // The other machine's nodes drawn now: its host node, and what the cap
  // lets through.
  const drawnNow = () => s.eval(`return window.__xenonet.scene.nodes().filter((n) => n.id.startsWith(${JSON.stringify(`@${HOST}:`)}) && window.__xenonet.driver.screenPos(n.id) !== null).length`);
  const chip = await until(chipOf, (c) => !!c?.capped);
  assert.ok(chip?.capped, `its chip says it's capped: ${JSON.stringify(chip)}`);
  const { drawn, of } = chip.capped;
  assert.ok(drawn <= 24 && drawn < of, `at most 24 of its nodes at the overview: ${JSON.stringify(chip.capped)}`);
  assert.equal(await drawnNow(), drawn + 1, "what the chip says is drawn, and the host node");
  // Looking at it, by selecting its host node: all of it, where it was.
  await s.driver("select", HOST_NODE);
  const looked = await chipOf();
  assert.equal(looked.capped, undefined, `not capped while it's looked at: ${JSON.stringify(looked)}`);
  assert.equal(await drawnNow(), of + 1, "every node of it drawn");
  await home$();
  assert.ok((await until(chipOf, (c) => !!c?.capped))?.capped, "capped again at the overview");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("a remote project gathers its sessions, and they say where they live and stay read-only", { timeout: 90000 }, shoot("hosts-gather", async () => {
  await home$();
  const project = await findRemote("atlas-api", "project");
  assert.ok(project, "the other machine has atlas-api");
  let st = await s.driver("select", project.id);
  assert.equal(st.gathered?.root, project.id);
  assert.ok(st.gathered.sessions >= 2, `gathered: ${JSON.stringify(st.gathered)}`);
  assert.deepEqual([st.selection.host, st.selection.kind], [HOST, "project"]);
  const look = () => s.eval(`return ({ where: document.querySelector("#inspector .insp-where")?.textContent ?? "", actions: [...document.querySelectorAll("#inspector [data-action]")].map((b) => b.dataset.action), answer: !!document.querySelector("#inspector [data-need-open], #inspector [data-need-ack], #inspector [data-fresh]"), gather: document.querySelector("[data-gather-text]")?.textContent ?? "" })`);
  let seen = await look();
  assert.match(seen.where, new RegExp(`on ${HOST} · live`));
  assert.match(seen.where, /read-only/);
  assert.deepEqual(seen.actions, [], "nothing opens a remote project here");
  assert.match(seen.gather, new RegExp(`on ${HOST}$`));

  // One of its sessions, selected with a real click: Preview and Stats only.
  const session = st.gathered.members.find((id) => /^@demo-b:(claude|codex):/.test(id));
  const clicked = await s.input({ click: { node: session } });
  st = clicked.state;
  assert.equal(st.selection?.id, session);
  assert.equal(st.selection.host, HOST);
  seen = await look();
  assert.deepEqual(seen.actions, ["preview", "stats"]);
  assert.equal(seen.answer, false, "no answering, dismissing or handing off from here");
  assert.match(seen.where, new RegExp(`on ${HOST}`));
}));

test("a remote session's stats come through the other machine's relay", { timeout: 60000 }, shoot("hosts-stats", async () => {
  // The demo's first session, which has tool calls, on the other machine.
  const session = `@${HOST}:claude:0190d3e0-0000-7000-8000-000000000001`;
  const st = await s.driver("stats", session);
  assert.equal(st.overlays.modal, true);
  assert.ok(st.stats && st.stats.calls > 0, `stats: ${JSON.stringify(st.stats)}`);
  await s.input({ keys: "Escape" });
}));

test("the alert strip names the host of a remote alert and offers nothing to do there", { timeout: 60000 }, shoot("hosts-alerts", async () => {
  await home$();
  const cards = await until(
    () => s.eval(`return ([...document.querySelectorAll("#alerts .al.need")].map((el) => ({ text: el.textContent.replace(/\\s+/g, " "), buttons: el.querySelectorAll("[data-al-open], [data-al-ack]").length })))`),
    (list) => list.some((c) => c.text.includes(`on ${HOST}`)),
  );
  const remote = cards.find((c) => c.text.includes(`on ${HOST}`));
  assert.ok(remote, JSON.stringify(cards));
  assert.equal(remote.buttons, 0);
  assert.match(remote.text, new RegExp(`The question is on ${HOST}`), "its text stays on its machine");
  assert.ok(cards.some((c) => !c.text.includes(`on ${HOST}`) && c.buttons === 2), "this machine's own still opens and dismisses");
}));

test("going offline dims the host and holds its constellation still, and coming back restores it", { timeout: 90000 }, shoot("hosts-offline", async () => {
  await home$();
  const project = await findRemote("atlas-api", "project");
  await s.simulate("host-offline", { a: HOST });
  const off = await until(hostOf, (h) => h?.state === "offline" && h.stale, 20000);
  assert.deepEqual([off?.state, off?.stale], ["offline", true]);
  const dim = (id) => s.eval(`return (window.__xenonet.scene.getNode(${JSON.stringify(id)})?.__obs?.userData.dim ?? 1)`);
  const dimmed = await until(() => dim(project.id), (d) => d < 0.6, 10000);
  assert.ok(dimmed < 0.6, `its projects dim: ${dimmed}`);
  const brief = (await s.driver("find", { query: "atlas-api", kind: "project", limit: 50 })).nodes.find((n) => n.id === project.id);
  assert.equal(brief.stale, true);
  const asOf = await until(() => s.eval(`return ([...document.querySelectorAll("#alerts .al-host.stale")].map((el) => el.textContent))`), (list) => list.length > 0, 10000);
  assert.match(asOf[0], new RegExp(`as of .*, ${HOST} not live`));
  const st = await s.driver("state");
  assert.deepEqual(st.hosts, [{ id: HOST, state: "offline" }]);

  await s.simulate("host-online", { a: HOST });
  const back = await until(hostOf, (h) => h?.state === "live" && !h.stale, 30000);
  assert.equal(back?.state, "live");
  assert.ok((await until(() => dim(project.id), (d) => d > 0.6, 10000)) > 0.6, "lit again");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("a machine's chip shows only its work, and show all brings the rest back", { timeout: 60000 }, shoot("hosts-chips", async () => {
  await home$();
  const local = (await s.driver("find", { query: "atlas-api", kind: "project", limit: 50 })).nodes.find((n) => !n.host);
  const remote = await findRemote("atlas-api", "project");
  const drawn = (id) => s.eval(`return (window.__xenonet.driver.screenPos(${JSON.stringify(id)}) !== null)`);
  assert.deepEqual([await drawn(local.id), await drawn(remote.id)], [true, true]);
  await s.driver("filterHosts", [HOST]);
  assert.deepEqual((await s.driver("hosts")).filter, [HOST]);
  assert.deepEqual([await drawn(local.id), await drawn(remote.id), await drawn("core")], [false, true, true], "this machine's core stays as the center");
  // A real click on this machine's chip adds it back.
  const chip = await s.eval(`return (() => { const r = document.querySelector('[data-host-key="@here"]').getBoundingClientRect(); return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`);
  await s.input({ click: chip });
  assert.deepEqual((await s.driver("hosts")).filter.sort(), ["@here", HOST].sort());
  assert.equal(await drawn(local.id), true);
  await s.driver("filterHosts", []);
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
