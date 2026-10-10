// End to end: the actors layer (src/actors.js) on a real XenoNet Dev (the
// test instance, with the demo's made-up data and actors; see
// xenonet.test.js): the demo's actors drawn on a layer of their own with
// beams into what they act on, one selected and shown in the inspector,
// the pointer taking whatever's drawn in front, actors acting through
// simulate, one waiting on you with its card in the strip until clear, and
// every actor in its theme's form in a few themes. Then the Actors panel
// (src/actors-panel.js): every actor and how XenoNet knows it, you and an
// observer off the map and back, an actor added with its token shown once
// and its row saying how that token stands, a new token and a removal; and
// made-up registrations waiting on their cards (simulate actor-request),
// in the attention color, one let in by a real click on Allow and one
// denied, since the driver can't allow. No page errors along the way. Each
// theme's look is saved to .xenonet/e2e-actors-<id>.png on the way.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

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

// Polls the driver's actors() until `want` holds.
async function untilActors(want, ms = 10000) {
  const end = Date.now() + ms;
  let got;
  while (Date.now() < end && !want((got = await s.driver("actors")))) await sleep(200);
  return got;
}
const byId = (list, id) => list.actors.find((a) => a.id === id);

// Polls `fn` until it gives something truthy, and gives that (or the last
// falsy answer when time's up).
async function until(fn, ms = 8000) {
  const end = Date.now() + ms;
  let got;
  while (!(got = await fn()) && Date.now() < end) await sleep(150);
  return got;
}
// Where an element is on the page, in CSS pixels, for a real click, once
// it's scrolled into view as a person would; null when it isn't there.
const centerOf = (selector) =>
  s.eval(`return (() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    el?.scrollIntoView({ block: "center" });
    const r = el?.getBoundingClientRect();
    return r?.width ? { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) } : null;
  })()`);
// A real click on an element, as a person's would land.
async function clickOn(selector) {
  const at = await until(() => centerOf(selector), 4000);
  assert.ok(at, `${selector} is on the page`);
  return s.input({ click: at });
}
// The Actors panel's rows: each actor, what its row says and its buttons.
const panelRows = () =>
  s.eval(`return [...document.querySelectorAll("[data-ac-actors] > li")].map((li) => ({
    id: li.dataset.key,
    name: li.querySelector(".ac-title")?.textContent,
    via: li.querySelector(".ac-via")?.textContent,
    live: li.querySelector(".ac-live")?.textContent,
    buttons: [...li.querySelectorAll("button")].map((b) => b.textContent.trim()),
    yours: [...li.querySelectorAll("[data-yours]")].map((b) => b.textContent.trim()),
  }))`);
// The token on screen is masked: Show puts its value in the page, for ten
// seconds. This presses it, and reads it.
const tokenMasked = () => s.eval(`return document.querySelector("[data-ac-token-text]")?.textContent ?? null`);
const tokenShown = () =>
  s.eval(`return (() => {
    const box = document.querySelector('[data-secret="token"]');
    if (!box) return null;
    if (!box.classList.contains("shown")) box.querySelector("[data-secret-show]").click();
    return box.querySelector(".secret-plain")?.textContent ?? null;
  })()`);
const onPage = (text) => s.eval(`return document.documentElement.outerHTML.includes(${JSON.stringify(text)})`);
const panelClosed = async () => !(await s.driver("state")).overlays.modal;
// An actor's token tried from this test's process.
async function asActor(token) {
  const r = await fetch(`${s.base}/api/actors/me`, { headers: { authorization: `Bearer ${token}` } });
  return { status: r.status, body: await r.json() };
}

let first = null;
let cursor = 0;

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  ({ current: first } = await s.driver("themes"));
  ({ cursor } = await s.logs({ limit: 1 }));
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("the demo's actors are drawn on a layer of their own, with beams into what they act on", { timeout: 60000 }, shoot("actors-drawn", async () => {
  const list = await untilActors((l) => l.count === 4 && l.beams.length >= 2);
  assert.deepEqual(list.actors.map((a) => a.id).sort(), ["actor:kestrel", "actor:moth", "actor:wren", "actor:you"]);
  assert.equal(byId(list, "actor:you").title, "You", "never a name for you");
  for (const a of list.actors) {
    assert.match(a.color, /^#[0-9a-f]{6}$/, `${a.id}: a color`);
    assert.ok(a.form, `${a.id}: a form`);
    assert.ok(a.screen && Number.isFinite(a.screen.x), `${a.id}: somewhere on screen`);
  }
  // Agents take different colors from the palette.
  assert.equal(new Set(list.actors.filter((a) => a.actorKind === "agent").map((a) => a.color)).size, 3);
  // Beams land on nodes drawn on the map, and only there.
  const drawn = new Set(await s.eval("return window.__xenonet.scene.nodes().map((n) => n.id)"));
  for (const b of list.beams) assert.ok(drawn.has(b.node), `${b.actor}'s beam lands on ${b.node}, which is drawn`);
  assert.ok(list.beams.every((b) => !b.node.startsWith("actor:")), "beams go to places, never to actors");
  // A layer beside the graph: not nodes of it.
  const st = await s.driver("state");
  assert.equal(st.counts.actors, 4);
  assert.ok(!drawn.has("actor:wren"), "an actor isn't a node of the graph");
  const found = await s.driver("find", { query: "wren", kind: "actor" });
  assert.deepEqual(found.nodes.map((n) => n.id), ["actor:wren"]);
}));

test("selecting an actor frames it with what it acts on, and the inspector shows it", { timeout: 60000 }, shoot("actors-select", async () => {
  let st = await s.driver("select", "actor:wren");
  assert.deepEqual([st.selection.id, st.selection.kind, st.selection.title], ["actor:wren", "actor", "Wren"]);
  assert.ok(st.motion.settled);
  const inspector = await s.eval(`return (() => {
    const el = document.querySelector("#inspector");
    return {
      shown: !el.hidden,
      title: el.querySelector(".insp-title")?.textContent,
      chip: el.querySelector(".chip")?.textContent,
      sections: [...el.querySelectorAll(".panel-title.sub > span:first-child")].map((e) => e.textContent),
      presence: el.querySelector(".presence")?.textContent ?? "",
      links: [...el.querySelectorAll(".actor-targets [data-goto]")].map((e) => e.dataset.goto),
    };
  })()`);
  assert.ok(inspector.shown);
  assert.equal(inspector.title, "Wren");
  assert.match(inspector.chip, /Actor · Hermes Agent/);
  for (const section of ["Now", "How XenoNet knows it", "Working on"]) assert.ok(inspector.sections.includes(section), `the inspector has ${section}`);
  assert.match(inspector.presence, /working|idle|away|offline|not heard from|out of sight/);
  assert.ok(inspector.links.length > 0, "what it works on, each a link");
  // Its targets stay lit and on screen: the camera framed them.
  const wren = byId(await s.driver("actors"), "actor:wren");
  assert.ok(wren.selected && wren.screen.onScreen);
  for (const id of wren.beams) assert.ok((await s.driver("screenPos", id))?.onScreen, `${id} is in the frame`);
  // A target's link selects it.
  st = await s.driver("select", inspector.links[0]);
  assert.equal(st.selection.id, inspector.links[0]);
  // Esc lets go.
  await s.driver("select", "actor:wren");
  await s.input({ keys: "Escape" });
  const end = Date.now() + 5000;
  while (Date.now() < end && (await s.driver("state")).selection) await sleep(150);
  assert.equal((await s.driver("state")).selection, null);
}));

test("a click on an actor selects it, over whatever glows behind it", { timeout: 60000 }, shoot("actors-click", async () => {
  await s.driver("still", true);
  try {
    const at = await s.driver("screenPos", "actor:wren");
    assert.ok(at?.onScreen, "Wren is on screen");
    // As a person would, the pointer looks around Wren's middle for where
    // hovering shows it (a node's body may be drawn in front of it), and
    // clicks there.
    const tipAt = async (p) => (await s.input({ move: p }), s.eval(`return document.querySelector("#tooltip").hidden ? "" : document.querySelector("#tooltip").textContent`));
    let spot = null;
    for (const [dx, dy] of [[0, 0], [0, -8], [8, 0], [-8, 0], [0, 8], [6, -6], [-6, -6], [6, 6], [-6, 6], [0, -14], [14, 0], [-14, 0], [0, 14]]) {
      const p = { x: at.x + dx, y: at.y + dy };
      if (/Wren/.test(await tipAt(p))) {
        spot = p;
        break;
      }
    }
    assert.ok(spot, "hovering shows it");
    await s.input({ click: spot });
    const end = Date.now() + 5000;
    let st;
    while (Date.now() < end && (st = await s.driver("state")).selection?.id !== "actor:wren") await sleep(150);
    assert.equal(st.selection?.id, "actor:wren");
  } finally {
    await s.driver("still", false);
    await s.driver("select", null);
  }
}));

test("the pointer takes whatever is drawn in front: Wren before a node's body, and the body before Wren", { timeout: 60000 }, shoot("actors-front", async () => {
  await s.driver("still", true);
  try {
    const wren = byId(await untilActors((l) => byId(l, "actor:wren")?.beams.length > 0), "actor:wren");
    const node = wren.beams[0];
    const title = (await s.driver("find", { query: node, limit: 1 })).nodes[0]?.title;
    assert.ok(title, `${node} is on the map`);
    // The camera on the line through Wren and the node it beams into, on
    // one side and then the other, so one is drawn right in front of the
    // other at the middle of the screen.
    const look = (from) =>
      s.eval(`return (() => {
        const sc = window.__xenonet.scene, a = sc.actor("actor:wren").group.position, n = sc.getNode(${JSON.stringify(node)});
        const [near, far] = ${JSON.stringify(from)} === "wren" ? [a, n] : [n, a];
        const d = Math.hypot(near.x - far.x, near.y - far.y, near.z - far.z) || 1, k = 70 / d;
        const eye = { x: near.x + (near.x - far.x) * k, y: near.y + (near.y - far.y) * k, z: near.z + (near.z - far.z) * k };
        sc.graph.cameraPosition(eye, { x: far.x, y: far.y, z: far.z }, 0);
        return true;
      })()`);
    // What hovering shows and a click takes there. Wren's script moves it
    // now and then, so a try that finds it moved looks again.
    const takes = async (from, want) => {
      let got = null;
      for (let tries = 0; tries < 3 && got?.picked !== want; tries++) {
        await look(from);
        await sleep(250);
        const p = await s.driver("screenPos", node);
        await s.input({ move: p });
        const tip = await s.eval(`return document.querySelector("#tooltip").hidden ? "" : document.querySelector("#tooltip").textContent`);
        await s.input({ click: p });
        await sleep(300);
        got = { tip, picked: (await s.driver("state")).selection?.id ?? null };
        // Letting go waits for the camera the selection moved.
        await s.driver("select", null);
      }
      return got;
    };
    let got = await takes("wren", "actor:wren");
    assert.equal(got.picked, "actor:wren", "Wren in front takes the click");
    assert.match(got.tip, /Wren/, "and hovering shows Wren");
    got = await takes("node", node);
    assert.equal(got.picked, node, "the node's body in front takes it");
    assert.ok(got.tip.includes(title) && !/Wren/.test(got.tip), `and hovering shows the node: ${got.tip}`);
  } finally {
    await s.driver("still", false);
    await s.driver("select", null);
    await s.driver("reset");
  }
}));

test("actors act: a spawn beams to the session at once, and an actor that goes offline takes its beams away", { timeout: 60000 }, shoot("actors-act", async () => {
  const { nodes } = await s.driver("find", { query: "refunds guide", kind: "session", limit: 1 });
  const session = nodes[0].id;
  // Wren, whose script never takes it offline, so nothing else takes the
  // beam away meanwhile.
  const done = await s.simulate("actor-spawn", { a: "actor:wren", b: session });
  assert.deepEqual([done.ok, done.a, done.b], [true, "actor:wren", session]);
  let list = await untilActors((l) => byId(l, "actor:wren")?.beams.includes(session), 4000);
  assert.ok(byId(list, "actor:wren").beams.includes(session), "a beam to the session it started");
  await s.simulate("actor-offline", { a: "actor:moth" });
  list = await untilActors((l) => byId(l, "actor:moth")?.presence === "offline" && !byId(l, "actor:moth").beams.length, 5000);
  assert.deepEqual([byId(list, "actor:moth").presence, byId(list, "actor:moth").beams], ["offline", []]);
  await s.simulate("actor-online", { a: "actor:moth" });
  list = await untilActors((l) => byId(l, "actor:moth")?.presence === "working", 5000);
  assert.equal(byId(list, "actor:moth").presence, "working");
}));

test("what actors do is listed in the activity feed, each line its actor's, and a click on one selects the actor", { timeout: 60000 }, shoot("actors-feed", async () => {
  const { nodes } = await s.driver("find", { query: "refunds guide", kind: "session", limit: 1 });
  const since = Date.now() - 1000;
  await s.simulate("actor-spawn", { a: "actor:wren", b: nodes[0].id });
  const line = await until(async () => (await s.driver("feed", { limit: 60 })).lines.find((l) => l.actor === "actor:wren" && l.kind === "spawned" && l.at >= since));
  assert.ok(line, "Wren's line is in the feed");
  assert.equal(line.title, "Wren", "named as the actor, never by a session's title");
  assert.match(line.detail, /^Started a (Claude Code|Codex) session in \S+/, "in XenoNet's words");
  // The demo's other actors are listed too, as they go about their scripts.
  const others = await until(async () => {
    const lines = (await s.driver("feed", { limit: 60 })).lines.filter((l) => l.actor);
    return new Set(lines.map((l) => l.actor)).size >= 2 ? lines : null;
  }, 15000);
  assert.ok(others, "more than one actor in the feed");
  await s.input({ move: await centerOf("#feed") });
  const held = await s.driver("feed", { limit: 60 });
  assert.equal(held.paused, true);
  await s.simulate("actor-edit", { a: "actor:wren", b: nodes[0].id });
  assert.ok(await until(async () => (await s.driver("feed", { limit: 60 })).pending > 0), "new activity waits while the pointer is in the feed");
  assert.deepEqual((await s.driver("feed", { limit: 60 })).lines, held.lines, "rows stay under the pointer");
  await clickOn(`#feed li[data-actor="actor:wren"]`);
  assert.equal((await until(async () => ((await s.driver("state")).selection?.id === "actor:wren" ? "yes" : null))) ?? "no", "yes", "a click on its line selects it");
  await s.input({ move: { x: 800, y: 600 } });
  assert.equal((await s.driver("feed")).pending, 0, "leaving brings in the queued activity");
  await s.driver("select", null);
}));

test("an actor's message to agents is drawn from the actor, light running down a beam to each session it reached", { timeout: 60000 }, shoot("actors-message", async () => {
  // An actor's real message needs its token, which the process check
  // refuses from under an agent, as these tests may run; so the layer is
  // handed the message as the page's agent-message listener hands it.
  const sessions = (await s.driver("find", { kind: "session", limit: 40 })).nodes.filter((n) => n.drawn).map((n) => n.id);
  const reached = sessions.slice(0, 2);
  assert.equal(reached.length, 2, "two sessions on the map to message");
  const before = byId(await s.driver("actors"), "actor:moth");
  const drawn = await s.eval(`return (() => {
    const sc = window.__xenonet.scene;
    sc.actorMessage("actor:moth", ${JSON.stringify([...reached, "actor:wren"])}, Date.now());
    return sc.actorLayer.stats();
  })()`);
  assert.ok(drawn.pulses >= 2, `light runs to each session: ${JSON.stringify(drawn)}`);
  const list = await untilActors((l) => reached.every((id) => byId(l, "actor:moth")?.beams.includes(id)), 4000);
  assert.ok(reached.every((id) => byId(list, "actor:moth").beams.includes(id)), `a beam to each: ${JSON.stringify(byId(list, "actor:moth").beams)} (was ${JSON.stringify(before.beams)})`);
  assert.ok(!byId(list, "actor:moth").beams.includes("actor:wren"), "never a beam to an actor");
  assert.deepEqual(await s.driver("errors"), []);
}));

test("an actor waiting on you has its card in the strip, and clear takes it away", { timeout: 60000 }, shoot("actors-needs-you", async () => {
  const done = await s.simulate("actor-needs-you", { a: "actor:wren" });
  assert.deepEqual([done.ok, done.a], [true, "actor:wren"]);
  const needOf = async () => (await s.driver("state")).alerts.find((a) => a.kind === "actor-need" && a.actor === "actor:wren");
  const card = await until(needOf, 6000);
  assert.ok(card, "Wren's card is in the strip");
  assert.equal(card.waits, "asks");
  assert.match(card.text, /refunds fix/);
  const shown = await s.eval(`return (() => {
    const el = document.querySelector('#alerts [data-al-goto="actor:wren"]');
    return el && { text: el.textContent.replace(/\\s+/g, " ").trim(), need: el.matches(".need.actor-need"), title: document.title };
  })()`);
  assert.ok(shown?.need, "drawn as something waiting on you");
  assert.match(shown.text, /Wren.*asks.*refunds fix/);
  assert.match(shown.title, /^\(\d+\) XenoNet$/, "and counted in the window's title");
  // Framing the whole map leaves room for the strip, full now, and the
  // panels: no actor floats behind them.
  await s.driver("reset");
  const room = await s.eval(`return (() => {
    const box = (sel) => document.querySelector(sel).getBoundingClientRect();
    return { top: Math.round(box("#alerts").bottom), left: Math.round(box(".hud-left").right), right: Math.round(box(".hud-right").left) };
  })()`);
  assert.ok(room.top > 150, `the strip holds several cards: ${JSON.stringify(room)}`);
  for (const a of (await s.driver("actors")).actors) assert.ok(a.screen.y > room.top && a.screen.x > room.left && a.screen.x < room.right, `${a.id} at ${a.screen.x},${a.screen.y} is clear of the strip and the panels (${JSON.stringify(room)})`);
  // Its card selects it.
  await clickOn('#alerts [data-al-goto="actor:wren"]');
  assert.equal((await until(async () => ((await s.driver("state")).selection?.id === "actor:wren" ? "yes" : null))) ?? "no", "yes", "its card selects it");
  await s.driver("select", null);
  // clear undoes every simulation, this one too.
  await s.simulate("clear");
  assert.equal((await until(async () => ((await needOf()) ? null : "gone"), 6000)) ?? "still there", "gone", "clear takes the card away");
}));

test("an actor that reaches XenoNet through a tunnel has a gate, open with an adapter connected and shut without; a read flares through it", { timeout: 60000 }, shoot("actors-tunnel", async () => {
  const gateOf = (l) => (l.tunnels ?? []).find((g) => g.actor === "actor:wren");
  assert.equal(gateOf(await s.driver("actors")), undefined, "no gate before it has a tunnel");
  assert.deepEqual((await s.simulate("actor-tunnel", { a: "actor:wren" })).a, "actor:wren");
  let list = await untilActors((l) => gateOf(l)?.open > 0.9, 8000);
  assert.ok(gateOf(list)?.open > 0.9, `its gate opens: ${JSON.stringify(list.tunnels)}`);
  const wren = byId(list, "actor:wren");
  assert.ok(gateOf(list).at.every(Number.isFinite), "and has a place on the map");
  // A read: light down the conduit, then out along beams to what it read.
  const target = wren.beams[0];
  assert.ok(target, "Wren beams into something to read");
  const read = await s.simulate("actor-read", { a: "actor:wren", b: target });
  assert.deepEqual(read.reach, [target]);
  const lit = await until(async () => ((await s.driver("actors")).layer.pulses > 0 ? "lit" : null), 3000);
  assert.equal(lit, "lit", "light runs from the gate to what it read");
  await s.simulate("actor-tunnel-down", { a: "actor:wren" });
  list = await untilActors((l) => gateOf(l)?.open < 0.3, 8000);
  assert.ok(gateOf(list)?.open < 0.3 && gateOf(list).shown > 0.5, `shut, but still there: ${JSON.stringify(gateOf(list))}`);
  // The inspector says which route it takes, and whether it's open.
  await s.driver("select", "actor:wren");
  const through = await until(async () => s.eval(`return (() => { const k = [...document.querySelectorAll(".kv .k")].find((x) => x.textContent === "through"); return k ? k.nextElementSibling.textContent : null; })()`), 4000);
  assert.match(through ?? "", /OpenAI Secure MCP Tunnel · shut/);
  await s.driver("select", null);
  await s.simulate("clear");
  list = await untilActors((l) => !gateOf(l), 8000);
  assert.equal(gateOf(list), undefined, "clear takes the gate away, once it's faded");
  assert.deepEqual(await s.driver("errors"), []);
}));

test("you write to an actor from its inspector: it reads it in its inbox as yours, from XenoNet's window", { timeout: 60000 }, shoot("actors-message-you", async () => {
  await s.driver("select", "actor:moth");
  const button = await until(() => s.eval(`return !!document.querySelector('#inspector [data-message-actor="actor:moth"]')`));
  assert.ok(button, "Message… in a registered actor's inspector");
  await s.eval(`return document.querySelector('#inspector [data-message-actor="actor:moth"]').click()`);
  assert.ok(await until(() => s.eval(`return !!document.querySelector("[data-msg-text]")`)), "the box to write in");
  assert.equal(await s.eval(`return document.querySelector("[data-msg-text]").hasAttribute("data-yours") && document.querySelector("[data-msg-send]").hasAttribute("data-yours")`), true, "yours alone to write and send");
  await s.eval(`return (() => { document.querySelector("[data-msg-text]").value = "Hold the geofence job until morning."; document.querySelector("[data-msg-send]").click(); })()`);
  const sent = await until(async () => (await s.api("/api/engine/state")).messages.find((m) => m.from === "actor:you" && m.text === "Hold the geofence job until morning."), 6000);
  assert.ok(sent, "sent as yours");
  assert.deepEqual(sent.to.map((t) => t.id), ["actor:moth"]);
  // Not from outside the window, and not to an agent's session.
  const outside = await fetch(`${s.base}/api/actors/you/message`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ to: "actor:moth", text: "x" }) });
  assert.equal(outside.status, 403);
  await s.driver("select", null);
  assert.deepEqual(await s.driver("errors"), []);
}));

test("in each of a few themes, every actor takes the theme's form, with no page errors", { timeout: 180000 }, shoot("actors-themes", async () => {
  for (const id of ["phosphor", "helios", "rime", "trench", "eclipse", "ochre", first]) {
    await s.driver("theme", id);
    const theme = await s.api(`/api/themes/${id}`);
    const list = await untilActors((l) => l.actors.every((a) => a.form === (a.actorKind === "human" ? theme.actors.human.form : theme.actors.form)), 5000);
    for (const a of list.actors) {
      assert.equal(a.form, a.actorKind === "human" ? theme.actors.human.form : theme.actors.form, `${id}: ${a.id}'s form`);
      assert.ok(a.actorKind === "human" ? a.color === theme.colors.human : theme.actors.palette.includes(a.color), `${id}: ${a.id}'s color ${a.color} is the theme's`);
    }
    await s.driver("select", "actor:wren");
    await s.screenshot({ name: `e2e-actors-${id}`, inline: false });
    await s.driver("select", null);
  }
  const { entries } = await s.logs({ since: cursor, level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
}));

test("the Actors panel lists every actor and how XenoNet knows it, and takes you and an observer off the map and back", { timeout: 60000 }, shoot("actors-panel", async () => {
  const st = await s.driver("command", "actors");
  assert.ok(st.overlays.modal, "the panel is open");
  const rows = await until(async () => {
    const r = await panelRows();
    return r.length >= 4 ? r : null;
  });
  const row = (id) => rows.find((r) => r.id === id);
  assert.deepEqual(rows.map((r) => r.id), ["actor:you", "actor:kestrel", "actor:moth", "actor:wren"], "you first, then by name");
  assert.equal(row("actor:you").name, "You", "never a name for you");
  assert.match(row("actor:wren").via, /XenoNet reads its state · XenoNet traces work here back to it · made up, for the demo/);
  assert.match(row("actor:kestrel").via, /^XenoNet reads its state/);
  assert.match(row("actor:moth").via, /^it reports itself/);
  // Its name shows it on the map; Wake…, New token and Remove for the one
  // that reports itself, an observer's switch for the observed ones, and
  // Show me on the map for you.
  assert.deepEqual(row("actor:moth").buttons, ["Moth", "Wake…", "New token", "Remove"]);
  assert.deepEqual(row("actor:moth").yours, ["New token"], "New token is yours alone");
  assert.deepEqual(row("actor:wren").buttons, ["Wren"]);
  assert.deepEqual(row("actor:you").buttons, ["Show me on the map"]);
  assert.deepEqual(await s.eval(`return [...document.querySelectorAll("[data-ac-observe]")].map((b) => [b.dataset.acObserve, b.textContent])`), [["hermes-agent", "On"], ["grok-bot", "On"], ["muse", "On"]]);

  // You off the map: not drawn, and this window reports nothing; then back.
  // The switch is kept for the window, so it's put back however this goes.
  let list;
  try {
    await clickOn("[data-ac-you]");
    list = await untilActors((l) => !byId(l, "actor:you"), 4000);
    assert.equal(byId(list, "actor:you"), undefined, "off the map");
    assert.equal(await s.eval(`return localStorage.getItem("xenonet.you")`), "off");
    assert.match((await panelRows())[0].live, /off the map/);
    await clickOn("[data-ac-you]");
    list = await untilActors((l) => !!byId(l, "actor:you"), 4000);
    assert.ok(byId(list, "actor:you"), "back on the map");
  } finally {
    await s.eval(`return localStorage.setItem("xenonet.you", "on")`);
  }

  // Grok Bot's observer off: Kestrel leaves the map and the panel, and
  // comes back with it on.
  await clickOn('[data-ac-observe="grok-bot"]');
  list = await untilActors((l) => !byId(l, "actor:kestrel"), 5000);
  assert.equal(byId(list, "actor:kestrel"), undefined);
  assert.equal((await s.api("/api/actors/observers")).observers.find((o) => o.product === "grok-bot").on, false);
  assert.ok(await until(async () => !(await panelRows()).some((r) => r.id === "actor:kestrel")), "and the panel");
  await clickOn('[data-ac-observe="grok-bot"]');
  list = await untilActors((l) => !!byId(l, "actor:kestrel"), 5000);
  assert.ok(byId(list, "actor:kestrel"));

  // Show selects one on the map, and the panel steps aside.
  await clickOn('[data-ac-show="actor:wren"]');
  assert.ok(
    await until(async () => {
      const now = await s.driver("state");
      return now.selection?.id === "actor:wren" && !now.overlays.modal;
    }),
    "Wren is selected, and the panel closed",
  );
  await s.driver("select", null);
}));

test("an actor added in the panel shows its token once; New token replaces it, and Remove takes it away", { timeout: 60000 }, shoot("actors-add", async () => {
  await s.driver("command", "actors");
  await clickOn("[data-ac-new]:not([disabled])");
  await until(() => centerOf("[data-ac-name]"));
  assert.equal(await s.eval(`return document.activeElement?.matches("[data-ac-name]") ?? false`), true, "the name field has focus");
  await s.input({ text: "Kite" });
  assert.equal(await s.eval(`return document.querySelector("[data-ac-add]").hasAttribute("data-yours")`), true, "adding hands out a token: yours alone");
  await clickOn("[data-ac-add]");
  // Masked, and not in the page at all, until Show.
  assert.match((await until(tokenMasked)) ?? "", /^xenonet_actor_•+$/);
  assert.equal(await s.eval(`return /xenonet_actor_[0-9a-f]{64}/.test(document.documentElement.outerHTML)`), false, "the token isn't in the page before Show");
  const first = await until(tokenShown);
  assert.match(first ?? "", /^xenonet_actor_[0-9a-f]{64}$/, "its token, once");
  // XenoNet knows it; from under an agent, which may be running these tests,
  // the process check still refuses it on macOS.
  assert.notEqual((await asActor(first)).status, 401, "XenoNet knows the token");

  // Done: the token leaves the page, and Kite is on the list.
  await clickOn("[data-ac-done]");
  assert.ok(await until(async () => (await panelRows()).some((r) => r.id === "actor:kite")), "Kite in the panel");
  assert.equal(await onPage(first), false, "the page let the token go");

  // Its row says its token works, for 30 days unless it's used, as the
  // driver reads it with the window's key.
  const tokenOf = async (id) => (await s.driver("actorTokens")).tokens.find((t) => t.actor === id) ?? null;
  const kite = await until(async () => {
    const t = await tokenOf("actor:kite");
    return t?.row ? t : null;
  });
  assert.ok(kite, "Kite's row says how its token stands");
  assert.equal(kite.state, "works");
  assert.ok(Math.abs(kite.expiresAt - (Date.now() + 30 * 86400e3)) < 5 * 60e3, `it expires 30 days on: ${new Date(kite.expiresAt).toISOString()}`);
  assert.match(kite.words, /^token works · expires in 30 days · (never used|last used just now)$/);
  assert.equal(kite.row, kite.words, "the row says what the driver does");
  const all = await s.driver("actorTokens");
  assert.equal(all.canSee, true, "this window holds the key");
  assert.deepEqual([(await tokenOf("actor:moth"))?.state, (await tokenOf("actor:moth"))?.row], ["none", "no token"], "the demo's made-up actor that reports itself has none");
  assert.ok(!all.tokens.some((t) => t.actor === "actor:wren"), "an actor XenoNet only observes has no token");
  assert.equal((await fetch(`${s.base}/api/actors/tokens`)).status, 403, "a program without the key can't see them");

  // New token, asked first: a new one, once, and the old one stops.
  await clickOn('[data-ac-token="actor:kite"]');
  assert.equal(await s.eval(`return document.querySelector("[data-ac-rotate]").hasAttribute("data-yours")`), true);
  await clickOn("[data-ac-rotate]");
  const second = await until(async () => {
    const t = await tokenShown();
    return t && t !== first ? t : null;
  });
  assert.match(second ?? "", /^xenonet_actor_[0-9a-f]{64}$/);
  assert.deepEqual([(await asActor(first)).status, (await asActor(first)).body.reason], [401, "rotated"]);
  assert.equal((await tokenOf("actor:kite"))?.state, "works", "its new token works");
  // Closing the panel lets that one go too.
  await s.input({ keys: "Escape" });
  assert.ok(await until(async () => (await panelClosed()) && !(await onPage(second))), "closed, and the token gone with it");

  // Remove, asked first: its token stops, and it leaves the list.
  await s.driver("command", "actors");
  await clickOn('[data-ac-remove="actor:kite"]');
  await clickOn("[data-ac-remove-yes]");
  assert.ok(await until(async () => !(await s.api("/api/actors")).actors.some((a) => a.id === "actor:kite")), "Kite is gone");
  assert.deepEqual([(await asActor(second)).status, (await asActor(second)).body.reason], [401, "removed"]);
  assert.ok(!(await panelRows()).some((r) => r.id === "actor:kite"));
  assert.equal(await tokenOf("actor:kite"), null, "and its token's line with it");
  await s.input({ keys: "Escape" });
  await until(panelClosed);
}));

test("programs asking to be actors wait on cards; a real click on Allow lets one in, and Deny keeps the other out", { timeout: 60000 }, shoot("actors-request", async () => {
  const heron = await s.simulate("actor-request", { a: "Heron" });
  const osprey = await s.simulate("actor-request", { a: "Osprey" });
  assert.deepEqual([heron.ok, osprey.ok], [true, true]);
  assert.match(heron.a, /^actor:heron/);
  // Their cards, first in the strip, each Allow ready a moment after it shows.
  const ours = [heron.request, osprey.request];
  const waiting = await until(async () => {
    const r = await s.driver("actorRequests");
    return ours.every((id) => r.requests.some((q) => q.id === id && q.card && q.armed)) ? r : null;
  });
  assert.ok(waiting, "both cards up, and ready");
  assert.equal(waiting.canDecide, true, "this window holds the key");
  const card = waiting.requests.find((r) => r.id === heron.request);
  assert.deepEqual([card.name, card.actor, card.asker.program, card.asker.folder], ["Heron", heron.a, `~/bin/${heron.a.slice(6)}`, `~/Projects/${heron.a.slice(6)}`]);
  assert.equal((await s.driver("state")).counts.requests, waiting.requests.length);
  const text = await s.eval(`return document.querySelector('#alerts [data-al-request="${heron.request}"]').textContent`);
  for (const words of ["Heron", "asks to join XenoNet", "run commands, edit repos, message SI", card.asker.program, card.asker.folder]) assert.ok(text.includes(words), `the card says ${words}`);
  assert.equal(await s.eval(`return [...document.querySelectorAll("#alerts .al")].slice(0, ${waiting.requests.length}).every((el) => el.matches(".request"))`), true, "first in the strip");
  assert.equal(await s.eval(`return document.querySelector('#alerts [data-al-allow="${heron.request}"]').hasAttribute("data-yours")`), true, "Allow is yours alone");
  assert.equal(byId(await s.driver("actors"), heron.a), undefined, "not on the map while it waits");
  // In the attention color, in every theme: Eclipse's actors are gray.
  const cardColor = () => s.eval(`return (() => {
    const card = document.querySelector('#alerts [data-al-request="${heron.request}"]');
    const css = (el, name) => getComputedStyle(el).getPropertyValue(name).trim().toLowerCase();
    return { card: css(card, "--c"), attention: css(document.documentElement, "--attention"), actor: css(document.documentElement, "--actor") };
  })()`);
  for (const id of ["eclipse", first]) {
    await s.driver("theme", id);
    const c = await cardColor();
    assert.equal(c.card, c.attention, `${id}: the card is in the attention color, not the actors' ${c.actor}`);
  }

  // A real click on Heron's Allow: there's no driver method for it.
  await clickOn(`#alerts [data-al-allow="${heron.request}"]`);
  const list = await untilActors((l) => !!byId(l, heron.a), 6000);
  assert.equal(byId(list, heron.a)?.presence, "unknown", "allowed, and not heard from yet");
  assert.ok(await until(() => s.eval(`return !document.querySelector('#alerts [data-al-request="${heron.request}"]')`)), "its card is gone");

  // The panel shows Osprey in full, its parents' paths and all, and its
  // Deny keeps it out.
  await s.driver("command", "actors");
  const asked = await until(() => s.eval(`return document.querySelector('[data-ac-requests] > li[data-key="${osprey.request}"]')?.textContent ?? null`));
  assert.ok(asked?.includes("Osprey") && asked.includes("/bin/zsh") && asked.includes("Terminal.app"), asked);
  // Its row waits in the attention color, as its card did.
  const rowColor = await s.eval(`return (() => {
    const row = document.querySelector('[data-ac-requests] > li[data-key="${osprey.request}"]');
    const css = (el, name) => getComputedStyle(el).getPropertyValue(name).trim().toLowerCase();
    return { row: css(row, "--c"), attention: css(document.documentElement, "--attention") };
  })()`);
  assert.equal(rowColor.row, rowColor.attention, "the panel's request row is in the attention color");
  await clickOn(`[data-ac-requests] [data-ac-deny="${osprey.request}"]`);
  assert.ok(await until(async () => !(await s.driver("actorRequests")).requests.some((q) => ours.includes(q.id))), "neither waits now");
  assert.equal(byId(await s.driver("actors"), osprey.a), undefined);
  assert.deepEqual([(await asActor(osprey.token)).status, (await asActor(osprey.token)).body.reason], [401, "denied"]);

  // Heron goes again, from the panel.
  await clickOn(`[data-ac-remove="${heron.a}"]`);
  await clickOn("[data-ac-remove-yes]");
  assert.ok(await until(async () => !(await s.api("/api/actors")).actors.some((a) => a.id === heron.a)));
  await s.input({ keys: "Escape" });
  await until(panelClosed);
}));

test("no page errors along the way", { timeout: 30000 }, async () => {
  const { entries } = await s.logs({ since: cursor, level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
