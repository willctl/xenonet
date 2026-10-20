// End to end: the Needs you panel (src/needs-you.js) on a real XenoNet Dev,
// the test instance with the demo's made-up data and actors. The demo's
// session waiting on a question, an actor waiting on you
// (actor-needs-you) and programs asking to be actors (actor-request) all
// wait in one queue, the longest wait first, opened from the palette, from
// N and from the strip's button. Show selects an actor, ✓ dismisses a
// session's wait, a demo session can't open in an app, and a real click on
// Deny keeps one program out while a real click on Allow, once it's ready,
// lets another in: the driver has no action for either. No page errors.
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
// A real click on an element, as a person's would land, once the view has
// settled, so a card folding away can't move it from under the click.
const centerOf = (selector) =>
  s.eval(`return (() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    el?.scrollIntoView({ block: "center" });
    const r = el?.getBoundingClientRect();
    return r?.width ? { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) } : null;
  })()`);
async function clickOn(selector) {
  await s.driver("settled");
  const at = await until(() => centerOf(selector), 4000);
  assert.ok(at, `${selector} is on the page`);
  return s.input({ click: at });
}
const queue = () => s.driver("needsYou");
const rows = () => s.eval(`return [...document.querySelectorAll("[data-ny-list] > li")].map((li) => ({ key: li.dataset.key, n: li.querySelector(".ny-n").textContent, text: li.textContent.replace(/\\s+/g, " ").trim(), buttons: [...li.querySelectorAll("button")].map((b) => b.textContent.trim()), yours: [...li.querySelectorAll("[data-yours]")].map((b) => b.textContent.trim()) }))`);
const listed = async (id) => (await s.api("/api/actors")).actors.some((a) => a.id === id);
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

let heron = null;
let osprey = null;

test("a question, an actor and programs asking to join wait in one queue, the longest wait first", { timeout: limit(60000) }, shoot("needs-you-queue", async () => {
  // The demo's own question, which waits from before XenoNet starts. A
  // simulated one can go again as the demo's session moves on.
  const standing = await until(async () => (await queue()).items.find((it) => it.kind === "question"));
  assert.ok(standing, "the demo has a session waiting on a question");
  const asked = { a: standing.id };
  assert.equal((await s.simulate("actor-needs-you", { a: "actor:wren" })).a, "actor:wren");
  await sleep(20);
  heron = await s.simulate("actor-request", { a: "Heron" });
  osprey = await s.simulate("actor-request", { a: "Osprey" });
  const q = await until(async () => {
    const got = await queue();
    return [`session:${asked.a}`, "actor:actor:wren", `request:${heron.request}`, `request:${osprey.request}`].every((k) => got.items.some((it) => it.key === k)) ? got : null;
  });
  assert.ok(q, "all four wait");
  const keys = q.items.map((it) => it.key);
  assert.ok(keys.indexOf(`session:${asked.a}`) < keys.indexOf("actor:actor:wren") && keys.indexOf("actor:actor:wren") < keys.indexOf(`request:${heron.request}`), `in the order they came: ${keys.join(", ")}`);
  const since = q.items.map((it) => it.since).filter((t) => t != null);
  assert.deepEqual(since, [...since].sort((a, b) => a - b), "the longest wait first");
  const wren = q.items.find((it) => it.key === "actor:actor:wren");
  assert.deepEqual([wren.kind, wren.title, wren.waitsWords, wren.actions], ["actor", "Wren", "asks", ["show", "message"]]);
  assert.match(wren.text, /refunds fix/);
  const question = q.items.find((it) => it.key === `session:${asked.a}`);
  assert.deepEqual([question.kind, question.actions], ["question", ["show", "open", "dismiss"]]);
  assert.deepEqual(q.items.find((it) => it.key === `request:${heron.request}`).actions, ["allow", "deny"]);
  assert.equal(q.open, false);
  assert.equal((await s.driver("state")).counts.needsYou, q.count);

  // The strip's button says how many, and opens the panel.
  const button = await until(() => s.eval(`return document.querySelector("#alerts [data-al-queue]")?.textContent ?? null`));
  assert.equal(button, `needs you · ${q.count}`);
  await clickOn("#alerts [data-al-queue]");
  assert.ok(await until(async () => (await s.driver("state")).overlays.needsYou), "the strip's button opens it");
  const now = await queue();
  const shown = await until(async () => ((await rows()).length === now.items.length ? rows() : null));
  assert.ok(shown, "a row each");
  assert.deepEqual(shown.map((r) => r.key), now.items.map((it) => it.key), "in the queue's order");
  assert.deepEqual(shown.map((r) => r.n), now.items.map((_, i) => String(i + 1)), "numbered");
  const heronRow = shown.find((r) => r.key === `request:${heron.request}`);
  assert.deepEqual([heronRow.buttons, heronRow.yours], [["Deny", "Allow"], ["Allow"]], "Allow is yours alone");
  assert.ok(heronRow.text.includes("Heron") && heronRow.text.includes("asks to join XenoNet") && heronRow.text.includes("run commands"), heronRow.text);
  assert.deepEqual(shown.find((r) => r.key === "actor:actor:wren").buttons, ["Show", "Message"]);
  assert.deepEqual(shown.find((r) => r.key === `session:${asked.a}`).buttons, ["Show", "Open", "✓"]);
  // Allow is ready a moment after its row shows.
  assert.ok(await until(async () => (await queue()).items.find((it) => it.key === `request:${heron.request}`)?.armed), "Allow arms");
  await s.screenshot({ name: "e2e-needs-you-panel", inline: false });

  // N closes it, and opens it again; so does the palette's command.
  await s.input({ keys: "n" });
  assert.equal(await until(async () => !(await s.driver("state")).overlays.needsYou), true, "N closes it");
  await s.input({ keys: "n" });
  assert.ok(await until(async () => (await s.driver("state")).overlays.needsYou), "N opens it");
  await s.driver("needsYou", "close");
  assert.ok((await s.driver("commands")).some((c) => c.id === "needs-you" && c.key === "N"));
  const st = await s.driver("command", "needs-you");
  assert.equal(st.overlays.needsYou, true, "the palette's command opens it");
}));

test("Show selects an actor, ✓ dismisses a session's wait, and a demo session can't open in an app", { timeout: limit(60000) }, shoot("needs-you-answer", async () => {
  const q = await queue();
  const question = q.items.find((it) => it.kind === "question" && it.actions.includes("dismiss"));
  assert.ok(question, "a question to dismiss");
  await assert.rejects(s.driver("needsYou", "open-app", question.key), /can't open in an app/, "the demo's sessions exist only in XenoNet");
  await assert.rejects(s.driver("needsYou", "dismiss", "session:nobody"), /Nothing waits on you by the key/);
  await assert.rejects(s.driver("needsYou", "allow", `request:${heron.request}`), /Use read, open, close/, "no action for Allow");

  const after = await s.driver("needsYou", "dismiss", question.key);
  assert.ok(!after.needsYou.items.some((it) => it.key === question.key), "gone from the queue at once");
  assert.ok(await until(async () => !(await s.api("/api/summary")).needsYou.some((n) => n.id === question.id)), "and the server drops it");
  assert.ok(await until(async () => !(await queue()).items.some((it) => it.key === question.key)), "it stays gone");

  const shown = await s.driver("needsYou", "show", "actor:actor:wren");
  assert.equal(shown.selection?.id, "actor:wren", "Show selects it");
  assert.equal(shown.overlays.needsYou, false, "and puts the panel away");
  await s.driver("select", null);
}));

test("a real click on Deny keeps one program out, and on Allow, once ready, lets another in", { timeout: limit(60000) }, shoot("needs-you-decide", async () => {
  await s.driver("needsYou", "open");
  await clickOn(`[data-ny-deny="request:${heron.request}"]`);
  assert.ok(await until(async () => !(await queue()).items.some((it) => it.key === `request:${heron.request}`)), "Heron no longer waits");
  assert.equal(await listed(heron.a), false, "Deny keeps it out");
  const ready = await until(async () => (await queue()).items.find((it) => it.key === `request:${osprey.request}`)?.armed);
  assert.ok(ready, "Osprey's Allow is ready");
  await clickOn(`[data-ny-allow="request:${osprey.request}"]`);
  assert.ok(await until(() => listed(osprey.a)), "Allow lets it in");
  assert.ok(await until(async () => !(await queue()).items.some((it) => it.kind === "request")), "no program waits now");
  // What's left: Wren, until clear.
  await s.simulate("clear");
  assert.ok(await until(async () => !(await queue()).items.some((it) => it.kind === "actor")), "clear takes Wren's wait away");
  const empty = await until(async () => {
    const q = await queue();
    return q.items.length ? null : q;
  }, 4000);
  if (empty) assert.equal(await s.eval(`return document.querySelector("[data-ny-empty]")?.hidden`), false, "an empty queue says nothing waits");
  await s.driver("needsYou", "close");
  // Osprey goes again, so the instance is as it was.
  await s.eval(`return (async () => { const key = await window.xenonetDesktop.windowKey(); return (await fetch("/api/actors/${osprey.a.slice(6)}", { method: "DELETE", headers: { "x-xenonet-window": key } })).status; })()`);
}));

test("no page errors along the way", { timeout: limit(30000) }, async () => {
  const { entries } = await s.logs({ since: cursor, level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
