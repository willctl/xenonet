// End to end: the network across machines in the console (src/network.js),
// on a real XenoNet Dev, the test instance with the demo's made-up data. The
// net-… simulations make up a network, with other machines and their
// agents (crates/xenonet-service/src/routes/net_sim.rs), since a real one
// needs a second machine. The Network panel on a machine on its own and in
// a network, a machine that's offline and where you are, Add a machine and
// its join line shown once and masked, another machine's agent waiting on
// you in the attention strip with its question's text, one that seems
// stuck with why, a machine that shares titles only, the other machines on
// the map from their presence with a session there found and messaged, one
// drawn from its own map's relay snapshot with a session's transcript
// opened through the window,
// where what waits sounds (here or on another machine: only where you
// are), a gateway that's down, and the message box that writes to a
// session as you, from the panel and from a session's own panel. Each
// state the page reads is the schema's (network/state/v1). No page errors.
// The window stays hidden; XENONET_E2E_SHOW=inactive shows it to watch.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { NetworkState } from "../../core/schema/network.js";
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

// Polls `fn` until it gives something truthy, and gives that (or the last
// falsy answer when time's up).
async function until(fn, ms = 8000) {
  const end = Date.now() + ms;
  let got;
  while (!(got = await fn()) && Date.now() < end) await sleep(150);
  return got;
}
// A real click where the driver says something is, once the view has
// settled: what's yours (data-yours) takes one, which the control channel
// makes only in the demo and test instances.
async function clickAt(at, what) {
  assert.ok(at, `${what} is on the page`);
  await s.driver("settled");
  return s.input({ click: at });
}
const network = () => s.driver("network", "read");
// The state the page reads, as the window reads it (the route is the
// owner's), checked against network/state/v1.
async function served() {
  const raw = await s.driver("network", "served");
  NetworkState.parse(raw);
  return raw;
}
// A simulation, and the page once it has heard of it.
async function simulate(kind, args = {}, want = () => true) {
  const done = await s.simulate(kind, args);
  assert.equal(done.ok, true, JSON.stringify(done));
  const n = await until(async () => {
    const got = await network();
    return got.simulated && want(got) ? got : null;
  });
  assert.ok(n, `the page heard of ${kind}`);
  return n;
}
let cursor = 0;

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  cursor = (await s.logs({ limit: 1 })).cursor ?? 0;
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.simulate("clear").catch(() => {});
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("a machine on its own: Network settings offers Start and Join, and nothing else of a network shows", { timeout: limit(60000) }, shoot("network-alone", async () => {
  const raw = await served();
  assert.deepEqual([raw.network, raw.self, raw.hosts], [null, null, []]);
  const st = await s.driver("state");
  assert.ok(!st.alerts.some((a) => a.kind === "remote-need"));
  const commands = await s.driver("commands");
  assert.ok(commands.some((c) => c.id === "network") && commands.some((c) => c.id === "add-machine"), "Network settings and Add a machine are in the palette");

  const opened = await s.driver("command", "network");
  assert.equal(opened.overlays.network, true);
  const n = await network();
  assert.deepEqual([n.open, n.view, n.inNetwork, n.hosts], [true, "main", false, []]);
  assert.match(n.text, /This machine is on its own\..*Start a network.*Join one/);
  await s.driver("network", "start");
  assert.match((await network()).text, /Start a network here.*xenonet net create --name <name> --host <id>/);
  await s.driver("network", "join");
  assert.match((await network()).text, /xenonet net join <address> <ticket>.*works once, for ten minutes/);
  // Add a machine starts with a network.
  await s.driver("network", "close");
  const add = await s.driver("command", "add-machine");
  assert.equal(add.overlays.network, true);
  const start = await until(async () => ((await network()).view === "start" ? network() : null));
  assert.match(start.text, /adding one starts with a network here/);
  await s.driver("network", "close");

  // net-none says the same, simulated.
  const none = await simulate("net-none");
  assert.deepEqual([none.inNetwork, none.network], [false, null]);
  await served();
}));

test("net-founder: this machine, where you are, the gateway and the other machines, one offline, with their agents", { timeout: limit(60000) }, shoot("network-founder", async () => {
  await simulate("net-founder", {}, (n) => n.inNetwork);
  const raw = await served();
  assert.equal(raw.self.id, "studio");
  const { network: n } = await s.driver("network", "open");
  assert.deepEqual([n.network, n.self, n.gateway.link, n.gateway.words, n.gateway.address], ["home", "studio", "live", "Linked", "gw.example.net"]);
  const byId = Object.fromEntries(n.hosts.map((x) => [x.id, x]));
  assert.deepEqual(Object.keys(byId).sort(), ["laptop", "rack", "studio"]);
  assert.deepEqual([byId.studio.here, byId.studio.you, byId.studio.founder, byId.studio.gateway], [true, true, true, true], "you're here, at the founder and gateway");
  assert.deepEqual([byId.laptop.online, byId.laptop.you, byId.laptop.remote.length], [true, false, 2]);
  assert.equal(byId.rack.online, false);
  assert.match(byId.rack.row, /^offline since .+ · 1 agent when last seen$/, "the offline one says since when");
  assert.match(n.text, /studio\s*this machine\s*you're here\s*founder\s*gateway\s*window/i);
  // Its other machines' agents, each with Message; the founder has no Remove.
  assert.equal(await s.eval(`return document.querySelectorAll("#modal [data-net-message]").length`), 3);
  assert.equal(await s.eval(`return !!document.querySelector('#modal [data-net-host="studio"] [data-net-remove]')`), false);
  assert.equal(await s.eval(`return !!document.querySelector('#modal [data-net-host="rack"] [data-net-remove]')`), true);
  await s.screenshot({ name: "e2e-network-founder", inline: false });
}));

test("Add a machine shows the join line once, masked until Show, and a taken id gets Replace", { timeout: limit(60000) }, shoot("network-add", async () => {
  await simulate("net-founder", {}, (n) => n.inNetwork);
  // A machine that's there already.
  let st = await s.driver("network", "add", { host: "laptop", window: false });
  await clickAt(st.network.at.make, "Make the join line");
  const taken = await until(async () => (await network()).error);
  assert.match(taken, /There's a machine called laptop .*Replace/);

  st = await s.driver("network", "add", { host: "desk", window: true });
  assert.equal(st.network.view, "add");
  await clickAt(st.network.at.make, "Make the join line");
  const line = await until(async () => (await network()).join);
  assert.ok(line, "the join line shows");
  assert.deepEqual([line.host, line.window, line.shown, line.masked, line.joined], ["desk", true, false, true, false], "masked until Show");
  assert.ok(line.expires > Date.now() + 9 * 60e3 && line.expires <= Date.now() + 10 * 60e3, "for ten minutes");
  const shownText = (await network()).text;
  assert.match(shownText, /xenonet net join gw\.example\.net •{12}/);
  assert.match(shownText, /It works once, until .+: ten minutes\./);
  // Waiting now, and the ticket itself is in no answer.
  const raw = await served();
  assert.deepEqual(raw.tickets.map((t) => [t.host, t.window]), [["desk", true]]);
  assert.ok(!/[A-Z2-7]{4}-[A-Z2-7]{4}-[A-Z2-7]{2}/.test(JSON.stringify(raw)), "the state has no ticket");

  // Show shows it, for a while; a screenshot holds the page still, which
  // masks it again.
  await clickAt((await network()).at.show, "Show");
  const shown = await until(async () => ((await network()).join.shown ? network() : null));
  assert.equal(shown.join.masked, false, "Show shows it");
  assert.ok(!/[A-Z2-7]{4}-[A-Z2-7]{4}-[A-Z2-7]{2}/.test(JSON.stringify(shown)), "the driver never says the ticket");
  const held = await s.eval(`return (async () => { await window.__xenonet.driver.still(true); const t = document.querySelector('#modal [data-secret="net-join"] .secret-text').innerText; await window.__xenonet.driver.still(false); return t; })()`);
  assert.match(held, /•{12}/, "held still for a screenshot, it's masked");
  await s.screenshot({ name: "e2e-network-join-line", inline: false });
  await s.driver("network", "close");
  assert.equal((await network()).join, null, "it goes with the panel");
}));

test("net-remote-waiting: another machine's agent waiting on you shows in the strip with its machine, and sounds only where you are", { timeout: limit(60000) }, shoot("network-remote-waiting", async () => {
  await simulate("net-remote-waiting", {}, (n) => n.waits.length > 0);
  const card = await until(async () => (await s.driver("state")).alerts.find((a) => a.kind === "remote-need"));
  assert.ok(card, "a card in the strip");
  assert.deepEqual([card.host, card.waits, card.chime], ["laptop", "is waiting for you", true]);
  assert.match(card.agent, /^@laptop:claude:/);
  assert.match(card.text, /query string/);
  const html = await s.eval(`return document.querySelector('#alerts [data-al-remote]')?.textContent.replace(/\\s+/g, " ").trim() ?? null`);
  assert.match(html, /Fix the login redirect is waiting for you\s*on laptop/);
  await s.screenshot({ name: "e2e-network-strip", inline: false });
  // A click on it opens the Network panel at its machine.
  const at = await s.eval(`return (() => { const r = document.querySelector('#alerts [data-al-remote]').getBoundingClientRect(); return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`);
  await clickAt(at, "the card");
  assert.ok(await until(async () => (await network()).open), "it opens the Network panel");
  assert.match((await network()).text, /waits on you Should the redirect in <LoginForm> keep the old query string/i);
  await s.driver("network", "close");

  // You're at laptop: it sounds there, not here.
  await simulate("net-remote-waiting", { a: "laptop" }, (n) => n.hosts.find((x) => x.id === "laptop")?.you);
  const quiet = await until(async () => (await s.driver("state")).alerts.find((a) => a.kind === "remote-need" && a.chime === false));
  assert.ok(quiet, "no chime here while you're at laptop");
}));

test("net-founder: another machine's question and stuck reason in the strip and the Network panel, escaped, and none of it with titles only", { timeout: limit(60000) }, shoot("network-detail", async () => {
  await simulate("net-founder", {}, (n) => n.waits.length > 0 && n.stuck.length > 0);
  const alerts = await until(async () => {
    const got = (await s.driver("state")).alerts;
    return got.some((a) => a.kind === "remote-need") && got.some((a) => a.kind === "remote-stuck") ? got : null;
  });
  assert.ok(alerts, "a card for the question and one for the stuck session");
  const need = alerts.find((a) => a.kind === "remote-need");
  const stuck = alerts.find((a) => a.kind === "remote-stuck");
  assert.deepEqual([need.host, need.text], ["laptop", "Should the redirect in <LoginForm> keep the old query string, or drop it?"]);
  assert.deepEqual([stuck.host, stuck.text], ["laptop", "ran cargo test 3 times in a row"]);
  const strip = await s.eval(`return document.querySelector('#alerts')?.textContent.replace(/\\s+/g, " ").trim() ?? ""`);
  assert.match(strip, /redirect in <LoginForm> keep the old query string/);
  // Escaped as transcripts are: its text is text, never markup.
  assert.match(await s.eval(`return document.querySelector('#alerts [data-al-remote]').innerHTML`), /in &lt;LoginForm&gt; keep/);
  assert.match(strip, /Tidy the billing tests may be stuck\s*on laptop\s*ran cargo test 3 times in a row/);
  const { network: n } = await s.driver("network", "open");
  assert.match(n.text, /waits on you Should the redirect in <LoginForm> keep the old query string/i);
  assert.match(n.text, /may be stuck ran cargo test 3 times in a row/i);
  const laptop = n.hosts.find((x) => x.id === "laptop");
  assert.ok(laptop.remote.some((a) => a.lastMessage === "The redirect now waits for the session check."), "what it said last");
  await s.screenshot({ name: "e2e-network-detail", inline: false });
  await s.driver("network", "close");

  // laptop shares titles only: the wait still shows, its text doesn't.
  await simulate("net-founder", { b: "titles-only" }, (n) => n.waits.length > 0 && n.stuck.length === 0);
  const card = await until(async () => (await s.driver("state")).alerts.find((a) => a.kind === "remote-need" && !/query string/.test(a.text)));
  assert.ok(card, "the card stays");
  assert.equal(card.text, "The question shows in laptop's console.");
  const st = await s.driver("state");
  assert.ok(!st.alerts.some((a) => a.kind === "remote-stuck"), "no stuck reason crosses");
  // Once the stuck card has folded away, none of laptop's detail is in the strip.
  const quietStrip = await until(async () => {
    const text = await s.eval(`return document.querySelector('#alerts')?.textContent ?? ""`);
    return /query string|cargo test/.test(text) ? null : { text };
  });
  assert.ok(quietStrip, "none of laptop's detail on the page");
  const { network: m } = await s.driver("network", "open");
  assert.ok(!/query string|cargo test|session check/.test(m.text), m.text);
  assert.match(m.text, /The question shows in laptop's console\./);
  await s.driver("network", "close");
}));

test("net-founder: the other machines on the map from their presence, one dimmed while it's away, and a session there found, shown and messaged as you", { timeout: limit(60000) }, shoot("network-map", async () => {
  await simulate("net-founder", {}, (n) => n.inNetwork);
  const drawn = await until(async () => {
    const h = await s.driver("hosts");
    return h.hosts.some((x) => x.id === "laptop" && x.drawn && x.constellation) && h.hosts.some((x) => x.id === "rack" && x.drawn) ? h : null;
  });
  assert.ok(drawn, "laptop and rack are constellations on the map");
  const laptop = drawn.hosts.find((x) => x.id === "laptop");
  const rack = drawn.hosts.find((x) => x.id === "rack");
  assert.deepEqual([laptop.state, laptop.network, rack.state, rack.stale], ["live", true, "offline", true], "the one that's away dims");
  assert.equal(drawn.self, "studio");
  assert.ok(!drawn.hosts.some((x) => x.id === "studio"), "this machine isn't one of them");
  // Projects by repo, and a folder that isn't one.
  const projects = await s.driver("find", { kind: "project", query: "@laptop:" });
  assert.deepEqual(projects.nodes.map((p) => p.id).sort(), ["@laptop:project:repo:github.com/acme/atlas-web", "@laptop:project:repo:github.com/acme/ledger"]);
  assert.equal((await s.driver("find", { kind: "project", query: "importer" })).nodes[0]?.id, "@rack:project:folder:~/jobs/importer");
  // A session there, found by its title.
  const found = await s.driver("find", { kind: "session", query: "login redirect" });
  const far = found.nodes.find((n) => n.id.startsWith("@laptop:"));
  assert.ok(far, JSON.stringify(found));
  assert.deepEqual([far.host, far.network, far.waits], ["laptop", true, true]);
  // Selected, it shows what its presence says, escaped, with Message.
  await s.driver("select", far.id);
  const panel = await until(async () => {
    const t = await s.eval(`return document.querySelector('#inspector')?.innerText.replace(/\\s+/g, " ") ?? ""`);
    return /Presence/i.test(t) ? t : null;
  });
  assert.ok(panel, "the inspector shows it");
  assert.match(panel, /on laptop · live · in your network/);
  assert.match(panel, /Should the redirect in <LoginForm> keep the old query string, or drop it\?/);
  assert.match(panel, /said last The redirect now waits for the session check\./i);
  assert.match(panel, /From laptop's presence/);
  await s.screenshot({ name: "e2e-network-map", inline: false });
  const button = await s.eval(`return (() => { const b = document.querySelector('#inspector [data-action="message"]'); const r = b?.getBoundingClientRect(); return r?.width ? { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2), label: b.textContent.trim() } : null; })()`);
  assert.equal(button?.label, "✉ Message");
  await clickAt(button, "Message");
  const box = await until(async () => ((await s.driver("message")).open ? s.driver("message") : null));
  assert.deepEqual([box?.to, box?.name], [far.id, "Fix the login redirect"]);
  assert.match(await s.eval(`return document.querySelector("#modal .m-sub").textContent`), /A session on laptop\. It gets it as from you, at studio's console/);
  await clickAt(await s.eval(`return (() => { const r = document.querySelector("#modal [data-mclose]").getBoundingClientRect(); return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`), "Close");
  await s.driver("select", null);
  // On its own again, the other machines leave the map.
  await simulate("net-none", {}, (n) => !n.inNetwork);
  assert.ok(await until(async () => !(await s.driver("hosts")).hosts.some((x) => x.network)), "they go");
}));

test("net-founder with laptop's relay: laptop drawn from its own map's snapshot, and a session's transcript opened from its panel, through the window", { timeout: limit(90000) }, shoot("network-relay", async () => {
  // laptop's map is this machine's own demo world, served as laptop's by a
  // relay over a network inside the service (the hub's net-relay).
  const done = await s.simulate("net-founder", { b: "relay" });
  assert.equal(done.relay?.ok, true, JSON.stringify(done));
  const session = "@laptop:claude:0190d3e0-0000-7000-8000-000000000001";
  const drawn = await until(async () => {
    const h = await s.driver("hosts");
    const laptop = h.hosts.find((x) => x.id === "laptop");
    return laptop?.network === "relay" && laptop.drawn && laptop.state === "live" && (await s.eval(`return !!window.__xenonet.scene.getNode(${JSON.stringify(session)})`)) ? h : null;
  }, 45000);
  assert.ok(drawn, "laptop is drawn from its relay snapshot");
  const laptop = drawn.hosts.find((x) => x.id === "laptop");
  assert.ok(laptop.sessions > 0 && laptop.constellation?.nodes > 5, JSON.stringify(laptop));
  // rack, offline with no snapshot, is still drawn from its presence.
  assert.equal(drawn.hosts.find((x) => x.id === "rack")?.network, true);
  // Its project with its sessions, as an SSH host's is: Preview, Stats,
  // and Message, since it's in your network; nothing that opens it here.
  await s.driver("select", session);
  const look = () => s.eval(`return ({ where: document.querySelector("#inspector .insp-where")?.textContent ?? "", actions: [...document.querySelectorAll("#inspector [data-action]")].map((b) => b.dataset.action), live: document.querySelector("#inspector .panel-title.sub span")?.textContent ?? "", answer: !!document.querySelector("#inspector [data-need-open], #inspector [data-need-ack], #inspector [data-fresh]") })`);
  const seen = await until(async () => {
    const l = await look();
    return /on laptop/.test(l.where) ? l : null;
  });
  assert.ok(seen, "the inspector shows it");
  assert.match(seen.where, /in your network/);
  assert.deepEqual(seen.actions, ["preview", "stats", "message"]);
  assert.equal(seen.live, "Live", "its live state, not its presence");
  assert.equal(seen.answer, false, "nothing answered, dismissed or handed off from here");
  // Preview, from the panel: its transcript comes through the window.
  await s.eval(`return (document.querySelector('#inspector [data-action="preview"]').click(), true)`);
  const shown = await until(async () => {
    const t = await s.eval(`return document.querySelector("#preview .p-count")?.textContent ?? ""`);
    return /^\d+ entries/.test(t) && !/^0 entries/.test(t) ? t : null;
  }, 20000);
  assert.ok(shown, `the transcript loaded: ${await s.eval(`return document.querySelector("#preview")?.innerText.slice(0, 300) ?? ""`)}`);
  assert.equal(await s.eval(`return !!document.querySelector("#preview .origin.bad")`), false, "no error in its place");
  await s.screenshot({ name: "e2e-network-relay", inline: false });
  await s.input({ keys: "Escape" });
  await s.driver("select", null);
  // Without the relay, laptop is presence's again.
  await simulate("net-founder", {}, (n) => n.inNetwork);
  assert.ok(await until(async () => (await s.driver("hosts")).hosts.find((x) => x.id === "laptop")?.network === true), "drawn from presence again");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("network mode: the load panel is Machines, the network's sums with each machine's part, this machine's sessions labelled here, and the Machines view's chips are the Filters chips", { timeout: limit(90000) }, shoot("network-machines", async () => {
  const done = await s.simulate("net-founder", { b: "relay" });
  assert.equal(done.relay?.ok, true, JSON.stringify(done));
  // The made-up network: studio (here), laptop online with a load of its
  // own, rack offline. The panel turns into the network's.
  const sys = await until(async () => {
    const v = await s.driver("system");
    return v?.network?.counted === 2 && v.hosts?.length === 3 ? v : null;
  }, 20000);
  assert.ok(sys, JSON.stringify(await s.driver("system")));
  assert.equal(sys.title, "Machines");
  assert.match(await s.eval(`return document.querySelector("#sysmon .panel-title").textContent`), /Machines/);
  const [studio, laptop, rack] = sys.hosts;
  assert.deepEqual([studio.id, studio.here, studio.counted, laptop.id, laptop.counted, rack.id, rack.online, rack.counted], ["studio", true, true, "laptop", true, "rack", false, false]);
  assert.notEqual(laptop.threads, studio.threads, "laptop is its own machine, not this one counted twice");
  // The sums: every thread's busy share, all the memory, the mean of the GPUs.
  const n = sys.network;
  const near = (a, b) => Math.abs(a - b) < 1e-9;
  assert.ok(near(n.cpu, (studio.cpu * studio.threads + laptop.cpu * laptop.threads) / (studio.threads + laptop.threads)), JSON.stringify(n));
  assert.ok(near(n.ram.share, (studio.memory.used + laptop.memory.used) / (studio.memory.total + laptop.memory.total)));
  assert.equal(n.gpu.count, 2);
  assert.ok(near(n.gpu.mean, (studio.gpu + laptop.gpu) / 2));
  assert.ok(near(n.gpu.vram.total, 16 * 1024 ** 3), "laptop's Apple GPU shares memory: not VRAM");
  // The stacked bars: a segment each for studio and laptop, adding up to the ring.
  for (const k of ["cpu", "gpu", "ram"]) {
    assert.deepEqual(n.bars[k].map((b) => b.id), ["studio", "laptop"], k);
    const sum = n.parts[k].reduce((a, p) => a + p.v, 0);
    assert.ok(near(sum, k === "cpu" ? n.cpu : k === "gpu" ? n.gpu.mean : n.ram.share), k);
  }
  assert.match(sys.text, /rack since/, "rack dimmed, with since when");
  // Each core and the trace stay this machine's, under its id.
  assert.equal(sys.cpu.cores, 16);
  assert.match(await s.eval(`return document.querySelector("#sysmon [data-local]").textContent`), /studio/);
  await s.screenshot({ name: "e2e-network-machines-panel", inline: false });

  // This machine's sessions carry its id with here, as laptop's carry laptop.
  const local = (await s.driver("find", { kind: "session", query: "claude:", limit: 80 })).nodes.find((x) => !x.host);
  assert.deepEqual(local?.machine, { id: "studio", here: true });
  const tags = await until(async () => {
    const t = await s.eval(`return [...document.querySelectorAll("#agents .host-tag")].map((e) => e.textContent)`);
    return t.some((x) => /^studio\s*here$/.test(x)) && t.includes("laptop") ? t : null;
  });
  assert.ok(tags, "the active sessions say which machine each is on, this one's too");

  // The Machines view, from the panel's title: its machine chips are the
  // Filters chips.
  await s.eval(`return (document.querySelector("#sysmon [data-sm-title]").click(), true)`);
  const view = await until(async () => ((await s.driver("machines", {})).machines.kpis ? s.driver("machines", {}) : null));
  assert.ok(view, "the view opened from the title");
  const v = view.machines;
  assert.deepEqual([v.kpis.machines, v.kpis.online, v.kpis.counted, v.kpis.gpus], [3, 2, 2, 2]);
  assert.deepEqual(v.cards.map((c) => c.id).sort(), ["laptop", "rack", "studio"]);
  assert.equal(v.cards.find((c) => c.id === "rack").hardware.gpus.length, 2, "an offline machine's parts, from its last reading");
  const picked = await s.driver("machines", { filter: { machines: ["laptop"] } });
  assert.deepEqual([picked.machines.filter.machines, picked.machines.shown], [["laptop"], ["laptop"]]);
  assert.deepEqual((await s.driver("hosts")).filter, ["laptop"], "the same chip under Filters");
  const linux = await s.driver("machines", { filter: { machines: [], platform: "linux", online: true }, sort: "cpu", view: "table" });
  assert.deepEqual([linux.machines.shown, linux.machines.view], [["studio"], "table"]);
  await s.screenshot({ name: "e2e-network-machines-view", inline: false });
  await s.driver("machines", { filter: { platform: "", online: false }, view: "cards" });
  await s.driver("machines", { close: true });
  assert.deepEqual((await s.driver("hosts")).filter, []);

  // Share load with the network, in this machine's Network panel.
  const { network: panel } = await s.driver("network", "open");
  assert.equal(panel.shareLoad, true);
  await clickAt(panel.at.load, "Share load with the network");
  assert.ok(await until(async () => (await served()).shareLoad === false), "turned off");
  await clickAt((await network()).at.load, "Share load with the network");
  assert.ok(await until(async () => (await served()).shareLoad === true), "and on again");
  await s.driver("network", "close");

  // On its own again: This computer, as before.
  await simulate("net-none", {}, (x) => !x.inNetwork);
  const alone = await until(async () => {
    const x = await s.driver("system");
    return x?.network === undefined && x.shown ? x : null;
  }, 10000);
  assert.ok(alone, "the panel is this computer's again");
  assert.match(await s.eval(`return document.querySelector("#sysmon .panel-title").textContent`), /This computer/);
  assert.equal((await s.driver("find", { kind: "session", query: "claude:", limit: 1 })).nodes[0]?.machine, undefined);
}));

test("what waits here sounds only where you are too, and everywhere is the rule's: one rule for this machine's waits and the others'", { timeout: limit(60000) }, shoot("network-chime", async () => {
  // A session here waits on you, and you're at laptop: neither it nor
  // laptop's sounds here.
  await s.simulate("needs-you");
  await simulate("net-remote-waiting", { a: "laptop" }, (n) => n.hosts.find((x) => x.id === "laptop")?.you);
  const away = await until(async () => {
    const got = (await s.driver("state")).alerts;
    const here = got.find((a) => a.kind === "need");
    return here && here.chime === false && got.some((a) => a.kind === "remote-need" && a.chime === false) ? got : null;
  });
  assert.ok(away, "no chime here, local or remote, while you're at laptop");
  // You're back here: both sound here.
  await simulate("net-remote-waiting", {}, (n) => n.hosts.find((x) => x.id === "studio")?.you);
  const back = await until(async () => {
    const got = (await s.driver("state")).alerts;
    return got.find((a) => a.kind === "need")?.chime === true && got.some((a) => a.kind === "remote-need" && a.chime === true) ? got : null;
  });
  assert.ok(back, "both chime here while you're here");
  // On its own again, a machine always chimes.
  await simulate("net-none", {}, (n) => !n.inNetwork);
  assert.ok(await until(async () => (await s.driver("state")).alerts.find((a) => a.kind === "need")?.chime === true));
  await s.simulate("clear");
}));

test("net-approval: a window host another one signed in waits for an approval here, and Approve adds it", { timeout: limit(60000) }, shoot("network-approval", async () => {
  await simulate("net-approval", {}, (n) => n.proposals.length === 1);
  await served();
  const { network: n } = await s.driver("network", "open");
  assert.deepEqual([n.proposals[0].op, n.proposals[0].host, n.proposals[0].window, n.proposals[0].signers], ["add", "den", true, ["laptop"]]);
  // The panel's titles show in capitals.
  assert.match(n.text, /waiting for approval.*add den as a window host, asked by laptop\. It needs the founder or a second window host\./i);
  await clickAt(n.at.approve, "Approve");
  const done = await until(async () => {
    const got = await network();
    return got.proposals.length === 0 && got.hosts.some((x) => x.id === "den") ? got : null;
  });
  assert.ok(done, "den is in, and nothing waits");
  await s.driver("network", "close");
}));

test("net-gateway-down says so: since when, why, and what waits in the outbox", { timeout: limit(60000) }, shoot("network-gateway-down", async () => {
  await simulate("net-gateway-down", {}, (n) => n.gateway?.link === "offline");
  await served();
  const { network: n } = await s.driver("network", "open");
  assert.deepEqual([n.self, n.gateway.words, n.gateway.reason, n.outbox.frames], ["laptop", "Can't reach the gateway", "timed out", 3]);
  assert.match(n.text, /Can't reach the gateway\s*gw\.example\.net.*No answer since .+: timed out\. This machine keeps working/);
  assert.match(n.text, /3 items wait in the outbox/);
  assert.equal(n.hosts.find((x) => x.id === "studio").online, false, "the other machines are away with it");
  await s.screenshot({ name: "e2e-network-gateway-down", inline: false });
  await s.driver("network", "close");
}));

test("the message box writes to a session as you, from the Network panel and from a session's own panel", { timeout: limit(90000) }, shoot("network-message", async () => {
  // Another machine's session, through a gateway that's down: it waits.
  const down = await simulate("net-gateway-down", {}, (n) => n.gateway?.link === "offline");
  const to = down.hosts.find((x) => x.id === "studio").remote[0].id;
  let st = await s.driver("messageAs", to, "Hold the release until the tests pass.");
  assert.deepEqual([st.message.open, st.message.to, st.message.name], [true, to, "Write the release notes"]);
  assert.match(await s.eval(`return document.querySelector("#modal .m-sub").textContent`), /A session on studio\. It gets it as from you, at laptop's console/);
  await clickAt(st.message.sendAt, "Send");
  let last = await until(async () => (await s.driver("message")).last);
  assert.deepEqual([last.to, last.ok, last.state], [to, true, "queued"]);
  assert.ok(await until(async () => (await network()).outbox.frames === 4), "into the outbox");

  // One on a machine that's there.
  const waiting = await simulate("net-remote-waiting", {}, (n) => n.waits.length > 0);
  st = await s.driver("messageAs", waiting.waits[0].agent, "Keep the query string, please.");
  await clickAt(st.message.sendAt, "Send");
  last = await until(async () => ((await s.driver("message")).last?.to === waiting.waits[0].agent ? (await s.driver("message")).last : null));
  assert.deepEqual([last.ok, last.state], [true, "stored"]);
  // The box closes once the answer is in, a moment after the driver sees it.
  assert.ok(await until(async () => (await s.driver("state")).overlays.modal === false), "the box closes once it's sent");

  // A session of this machine's has Message in its own panel.
  const found = await s.driver("find", { kind: "session", limit: 1 });
  const id = found.nodes[0].id;
  await s.driver("select", id);
  const button = await s.eval(`return (() => { const b = document.querySelector('#inspector [data-action="message"]'); const r = b?.getBoundingClientRect(); return r?.width ? { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2), label: b.textContent.trim() } : null; })()`);
  assert.equal(button?.label, "✉ Message");
  await clickAt(button, "Message");
  const box = await until(async () => ((await s.driver("message")).open ? s.driver("message") : null));
  assert.equal(box?.to, id, "the box is to that session");
  assert.match(await s.eval(`return document.querySelector("#modal .m-sub").textContent`), /A session on this machine\. It gets it as from you, at studio's console/);
  await s.input({ text: "Look at the failing test first." });
  await clickAt(box.sendAt, "Send");
  // It goes to POST /api/actors/you/message, which says how it went.
  last = await until(async () => ((await s.driver("message")).last?.to === id ? (await s.driver("message")).last : null));
  assert.ok(last && last.status > 0, `the route answered: ${JSON.stringify(last)}`);
  // Refused (until the engine takes messages to sessions from you), it stays
  // open, saying why; closed either way.
  if ((await s.driver("message")).open) await clickAt(await s.eval(`return (() => { const r = document.querySelector("#modal [data-mclose]").getBoundingClientRect(); return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }; })()`), "Close");
  await s.driver("select", null);
}));

test("clear ends the made-up network, and no page errors along the way", { timeout: limit(30000) }, async () => {
  await s.simulate("clear");
  assert.ok(await until(async () => !(await network()).inNetwork && !(await network()).simulated), "on its own again");
  assert.ok(!(await s.driver("state")).alerts.some((a) => a.kind === "remote-need"), "the strip's card goes");
  const { entries } = await s.logs({ since: cursor, level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
