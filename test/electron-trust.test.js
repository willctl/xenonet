// Who the desktop app takes calls and navigation from (electron/trust.js):
// XenoNet's own page alone. Made-up windows, frames and events stand in for
// Electron's.
import { test } from "node:test";
import assert from "node:assert/strict";
import { sameOrigin, fromOwnPage, devtoolsPort, createYoursGuard } from "../electron/trust.js";

const BASE = "http://127.0.0.1:47840";

test("sameOrigin: compared as origins, so an address that only starts like XenoNet's is another site", () => {
  for (const url of [`${BASE}/`, `${BASE}/#actors`, `${BASE}/index.html?x=1`, "http://127.0.0.1:47840"]) assert.equal(sameOrigin(url, BASE), true, url);
  for (const url of [
    "http://127.0.0.1:47840@elsewhere.example/",
    "http://127.0.0.1:47840.elsewhere.example/",
    "http://127.0.0.1:478400/",
    "http://127.0.0.1:47841/",
    "http://localhost:47840/",
    "https://127.0.0.1:47840/",
    "file:///Users/nova/page.html",
    "javascript:alert(1)",
    "not a url",
    "",
    undefined,
  ])
    assert.equal(sameOrigin(url, BASE), false, String(url));
});

test("fromOwnPage: the main frame of the window, on XenoNet's origin; never a frame in it, another page, or a closed window", () => {
  const webContents = { id: 1 };
  const win = { webContents, isDestroyed: () => false };
  const main = { origin: BASE, parent: null };
  const call = (frame, sender = webContents) => ({ sender, senderFrame: frame });
  assert.equal(fromOwnPage(call(main), { win, base: BASE }), true);
  assert.equal(fromOwnPage(call({ origin: BASE, parent: main }), { win, base: BASE }), false, "a frame inside the page");
  assert.equal(fromOwnPage(call({ origin: "http://elsewhere.example", parent: null }), { win, base: BASE }), false, "the window taken to another site");
  assert.equal(fromOwnPage(call({ origin: "null", parent: null }), { win, base: BASE }), false, "an opaque origin");
  assert.equal(fromOwnPage(call(main, { id: 2 }), { win, base: BASE }), false, "another window's page");
  assert.equal(fromOwnPage(call(null), { win, base: BASE }), false, "a frame that's gone");
  assert.equal(fromOwnPage(call(main), { win: { webContents, isDestroyed: () => true }, base: BASE }), false, "a closed window");
  assert.equal(fromOwnPage(call(main), { win: null, base: BASE }), false, "no window yet");
  assert.equal(fromOwnPage(undefined, { win, base: BASE }), false);
});

test("devtoolsPort: XENONET_CDP_PORT opens the DevTools port in XenoNet Dev only, never in the installed app", () => {
  assert.equal(devtoolsPort({ fromSource: true, env: { XENONET_CDP_PORT: "9333" } }), "9333");
  assert.equal(devtoolsPort({ fromSource: false, env: { XENONET_CDP_PORT: "9333" } }), null, "the installed app, whatever a program set for it with launchctl setenv");
  assert.equal(devtoolsPort({ fromSource: true, env: {} }), null, "not asked for");
  for (const junk of ["", "9333 --inspect", "port", "-1", "123456"]) assert.equal(devtoolsPort({ fromSource: true, env: { XENONET_CDP_PORT: junk } }), null, junk);
});

// A made-up page for the control channel's guard: boxes on the screen, each
// an element with its parent, and the one with focus. The guard's questions
// run against it as they would in the page.
function madeUpPage() {
  const el = (box, attrs = {}, parent = null) => ({
    box,
    attrs,
    parent,
    closest(selector) {
      assert.equal(selector, "[data-yours]");
      for (let e = this; e; e = e.parent) if ("data-yours" in e.attrs) return e;
      return null;
    },
  });
  const body = el([0, 0, 1600, 900]);
  const card = el([1200, 40, 1580, 140], {}, body);
  const allow = el([1480, 100, 1560, 128], { "data-yours": "" }, card);
  const label = el([1490, 104, 1550, 124], {}, allow);
  const deny = el([1400, 100, 1470, 128], {}, card);
  const order = [label, allow, deny, card, body];
  const document = {
    activeElement: body,
    elementFromPoint: (x, y) => order.find((e) => x >= e.box[0] && x <= e.box[2] && y >= e.box[1] && y <= e.box[3]) ?? null,
  };
  const asked = [];
  const ask = async (code) => (asked.push(code), new Function("document", code)(document));
  return { document, ask, asked, allow, deny, body };
}

test("the control channel's guard: no click, drag or Enter lands on what's yours, except in the demo and test instances", async () => {
  const page = madeUpPage();
  const guard = createYoursGuard({ guarded: true, ask: page.ask });
  await assert.rejects(guard.point({ x: 1520, y: 114 }), /yours alone/, "Allow, under its label");
  await assert.rejects(guard.point({ x: 1482, y: 102 }), /yours alone/, "Allow, at its edge");
  await guard.point({ x: 1430, y: 114 });
  await guard.point({ x: 800, y: 450 });
  await guard.point({ x: 5000, y: 5000 });
  page.document.activeElement = page.allow;
  await assert.rejects(guard.key({ key: "Enter" }), /yours alone/, "Enter on Allow presses it");
  await assert.rejects(guard.key({ key: " " }), /yours alone/, "and so does Space");
  await guard.key({ key: "Escape" });
  await guard.key({ key: "Tab" });
  page.document.activeElement = page.deny;
  await guard.key({ key: "Enter" });

  const demo = madeUpPage();
  const open = createYoursGuard({ guarded: false, ask: demo.ask });
  demo.document.activeElement = demo.allow;
  await open.point({ x: 1520, y: 114 });
  await open.key({ key: "Enter" });
  assert.deepEqual(demo.asked, [], "the demo and test instances aren't asked about at all");
});
