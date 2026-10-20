// XenoNet's control channel: how scripts, tests and agents drive Desktop.
// Installed copies receive authenticated requests through Service. Source
// copies use /api/dev/control, relayed through their utility process.
//
//   ping        the instance, its window and whether the page is ready
//   driver      a call into the page's driver (src/driver.js): state, find,
//               select, back, step, command, settled, still and the rest
//   eval        any expression in the page
//   screenshot  the window as it looks, even when covered or hidden
//   input       real keyboard and mouse input: keys, text, click, move,
//               drag, scroll, at a point or at a node
//   window      show (inactive or in front), hide, minimize, or set the
//               page's size
//   logs        recent page console, page errors, server output
//   reload, quit, cdp (any Chrome DevTools Protocol command)
//
// Screenshots use webContents.capturePage, and input goes through the
// window's own DevTools Protocol session (webContents.debugger), so both
// work on a covered, hidden or unfocused window, with no debugging port.
// Clients: core/control.js, the console MCP server (mcp/console.js), the
// end-to-end tests. Installed copies omit eval and raw CDP access.
//
// Source copies expose additional development controls.
// Outside the demo and test instances, input aimed at what's yours alone
// (Allow, Connect, New token, Add) is refused, so an agent checking its
// work doesn't press them in passing; eval and cdp still reach anything
// the page can (docs/design/actors/interface.md, section 5).
import { app } from "electron";
import { consolePolicy } from "../core/components/console-policy.js";
import fs from "node:fs";
import path from "node:path";
import { parseKey, parseSize, modifierBits } from "./keys.js";
import { createYoursGuard } from "./trust.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const within = (p, ms, what) => Promise.race([p, sleep(ms).then(() => Promise.reject(new Error(`${what} took longer than ${ms / 1000}s`)))]);
// Mouse buttons as CDP's `buttons` bit field.
const BUTTONS = { left: 1, right: 2, middle: 4 };

export function createControl({ root, production = false, instance, port, getWindow, showWindow, hideWindow, reveal, recorder, filmFile, onDriven = () => {} }) {
  // ---------- logs ----------

  const entries = [];
  let seq = 0;
  function log(source, level, text) {
    for (const line of String(text).split(/\r?\n/)) {
      if (!line.trim()) continue;
      entries.push({ seq: ++seq, ts: Date.now(), source, level, text: line.slice(0, 2000) });
    }
    if (entries.length > 600) entries.splice(0, entries.length - 600);
  }

  // Called for each new window.
  function watch(win) {
    const wc = win.webContents;
    wc.on("console-message", (e) => log("page", e.level, e.level === "error" || e.level === "warning" ? `${e.message} (${path.basename(e.sourceId || "page")}:${e.lineNumber})` : e.message));
    wc.on("render-process-gone", (_e, d) => log("app", "error", `the page's process ended: ${d.reason}`));
    wc.on("unresponsive", () => log("app", "warning", "the page stopped responding"));
    wc.on("did-fail-load", (_e, code, desc, url, main) => main && code !== -3 && log("app", "warning", `couldn't load ${url}: ${desc} (${code})`));
    wc.on("did-finish-load", () => log("app", "info", "page loaded"));
  }

  // ---------- the window and its page ----------

  function win() {
    const w = getWindow();
    if (!w || w.isDestroyed()) throw new Error(`${instance.name} has no window yet`);
    return w;
  }

  async function cdp(method, params = {}) {
    const d = win().webContents.debugger;
    if (!d.isAttached()) d.attach("1.3");
    return d.sendCommand(method, params);
  }

  // Runs `body` (an async function body) in the page and returns its value as
  // JSON, or throws the page's error with its own message.
  async function inPage(body, ms = 30000) {
    const js = `(async () => {
      try {
        const value = await (async () => { ${body} })();
        return { value: value === undefined ? null : JSON.parse(JSON.stringify(value)) };
      } catch (e) {
        return { error: String(e?.message ?? e) };
      }
    })()`;
    const r = await within(win().webContents.executeJavaScript(js, true), ms, "the page");
    if (r?.error) throw new Error(r.error);
    return r?.value ?? null;
  }

  // What's yours alone isn't the channel's to press, except in the demo
  // and test instances (trust.js).
  const yours = createYoursGuard({ guarded: !instance.demo, ask: (body) => inPage(body, 5000) });

  const driver = (method, args = [], ms) =>
    inPage(
      `const d = window.__xenonet?.driver;
      if (!d) throw new Error("the page has no driver: it's still loading");
      if (!Object.hasOwn(d, ${JSON.stringify(method)}) || typeof d[${JSON.stringify(method)}] !== "function") throw new Error("the driver has no " + ${JSON.stringify(method)} + "()");
      return d[${JSON.stringify(method)}](...${JSON.stringify(args)});`,
      ms,
    );

  function windowInfo() {
    const w = getWindow();
    if (!w || w.isDestroyed()) return null;
    const { width, height } = w.getContentBounds();
    return { visible: w.isVisible(), focused: w.isFocused(), minimized: w.isMinimized(), width, height };
  }

  async function ping() {
    const w = getWindow();
    const page = { url: null, loading: false, ready: false };
    if (w && !w.isDestroyed()) {
      page.url = w.webContents.getURL();
      page.loading = w.webContents.isLoading();
      try {
        Object.assign(page, await inPage(`return { ready: !!window.__xenonet?.driver, driver: window.__xenonet?.driver?.version ?? null }`, 3000));
      } catch {}
    }
    return { instance: instance.id, name: instance.name, demo: instance.demo, port, pid: process.pid, version: app.getVersion(), electron: process.versions.electron, window: windowInfo(), page };
  }

  // ---------- screenshots ----------

  async function screenshot({ name = "shot", settle = true, still = true, maxWidth = 1568, format = "jpeg", quality = 80, inline = true, timeout = 15000 } = {}) {
    const w = win();
    const notes = [];
    let held = false;
    let ready = true;
    try {
      // A page reloading (after a server restart, say) gets a moment.
      ready = (await ping()).page.ready;
      for (const until = Date.now() + 10000; !ready && Date.now() < until; ) {
        await sleep(300);
        ready = (await ping()).page.ready;
      }
      if (ready && still && !(await driver("state")).still) {
        await driver("still", [true]);
        held = true;
      }
      if (ready && settle) await driver("settled", [{ timeout }], timeout + 5000).catch((e) => notes.push(e.message));
      if (!ready) notes.push("the page isn't ready (no driver), so this is the window as it is");
      // A capture can fail while the compositor is busy (UnknownVizError on a
      // window drawing every frame); the next frame usually works.
      let img = null;
      for (let attempt = 0; ; attempt++) {
        try {
          img = await within(w.webContents.capturePage(undefined, { stayHidden: true }), 10000, "the capture");
          if (!img.isEmpty()) break;
          if (attempt >= 3) throw new Error("the capture came back empty");
        } catch (err) {
          if (attempt >= 3) throw err;
        }
        await sleep(120);
      }
      const file = path.join(root, ".xenonet", `${String(name).replace(/[^\w.-]/g, "") || "shot"}.png`);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, img.toPNG());
      const { width, height } = img.getSize();
      const out = { file: path.relative(root, file), width, height, notes };
      if (inline) {
        const small = width > maxWidth ? img.resize({ width: maxWidth, quality: "good" }) : img;
        const data = format === "png" ? small.toPNG() : small.toJPEG(quality);
        out.image = { mimeType: format === "png" ? "image/png" : "image/jpeg", data: data.toString("base64"), ...small.getSize() };
      }
      return out;
    } finally {
      if (held) await driver("still", [false]).catch(() => {});
    }
  }

  // ---------- input ----------

  // A point on the page: { x, y } in CSS pixels, or { node: id }.
  async function point(at) {
    if (Number.isFinite(at?.x) && Number.isFinite(at?.y)) return { x: at.x, y: at.y };
    if (at?.node) {
      const p = await driver("screenPos", [at.node]);
      if (!p) throw new Error(`${at.node} isn't drawn on the map`);
      if (!p.onScreen) throw new Error(`${at.node} is off screen; select it first, or reset the view`);
      return p;
    }
    throw new Error("a point is { x, y } or { node: id }");
  }
  const mouse = (type, { x, y }, extra = {}) => cdp("Input.dispatchMouseEvent", { type, x, y, ...extra });

  async function press(combo) {
    const k = parseKey(combo);
    await yours.key(k);
    const base = { key: k.key, code: k.code, windowsVirtualKeyCode: k.keyCode, modifiers: k.modifiers };
    await cdp("Input.dispatchKeyEvent", { type: k.text ? "keyDown" : "rawKeyDown", ...base, ...(k.text ? { text: k.text, unmodifiedText: k.text } : {}) });
    await cdp("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  }

  // The map works out what's under the pointer as it draws a frame, at most
  // every 50 ms, and a press takes what it found. So a press on the map
  // waits for a frame drawn after that: on a GPU the next one, and drawn in
  // software (no GPU, as on a hosted Linux runner) up to a second away. A
  // press on a panel doesn't wait for the map.
  async function hovered(p) {
    await sleep(80);
    await driver("drawn", [{ at: p }]).catch(() => {});
  }

  async function click(at, { button = "left", count = 1, modifiers } = {}) {
    const p = await point(at);
    await yours.point(p);
    const m = modifierBits(modifiers);
    await mouse("mouseMoved", p, { modifiers: m });
    await hovered(p);
    for (let i = 1; i <= count; i++) {
      await mouse("mousePressed", p, { button, buttons: BUTTONS[button], clickCount: i, modifiers: m });
      await mouse("mouseReleased", p, { button, buttons: 0, clickCount: i, modifiers: m });
    }
    return p;
  }

  async function drag(from, to, { modifiers, steps = 12 } = {}) {
    const a = await point(from), b = await point(to);
    // A press and a release on the same button is a click.
    await yours.point(a);
    await yours.point(b);
    const m = modifierBits(modifiers);
    await mouse("mouseMoved", a, { modifiers: m });
    await hovered(a);
    await mouse("mousePressed", a, { button: "left", buttons: 1, clickCount: 1, modifiers: m });
    for (let i = 1; i <= steps; i++) {
      await mouse("mouseMoved", { x: a.x + ((b.x - a.x) * i) / steps, y: a.y + ((b.y - a.y) * i) / steps }, { button: "left", buttons: 1, modifiers: m });
      await sleep(16);
    }
    await mouse("mouseReleased", b, { button: "left", buttons: 0, clickCount: 1, modifiers: m });
    return { from: a, to: b };
  }

  // One or more of these, in this order. `modifiers`, such as ["Alt"], hold
  // keys down for the pointer actions.
  async function input({ keys, text, move, click: at, drag: d, scroll, button, count, modifiers, settle = true, timeout } = {}) {
    const did = {};
    if (keys) {
      did.keys = [].concat(keys);
      for (const combo of did.keys) await press(combo);
    }
    if (text) {
      await cdp("Input.insertText", { text });
      did.text = text.length;
    }
    if (move) {
      did.move = await point(move);
      await mouse("mouseMoved", did.move, { modifiers: modifierBits(modifiers) });
    }
    if (at) did.click = await click(at, { button, count, modifiers });
    if (d) did.drag = await drag(d.from, d.to, { modifiers: d.modifiers ?? modifiers, steps: d.steps });
    if (scroll) {
      const p = await point(scroll);
      await mouse("mouseWheel", p, { deltaX: scroll.dx ?? 0, deltaY: scroll.dy ?? 0, modifiers: modifierBits(modifiers) });
      did.scroll = { ...p, dx: scroll.dx ?? 0, dy: scroll.dy ?? 0 };
    }
    if (!Object.keys(did).length) throw new Error("input takes keys, text, move, click, drag or scroll");
    // Then, like the driver's actions, wait for the view to settle and say
    // where things stand.
    if (!(await ping()).page.ready) return { did };
    if (settle) await driver("settled", [{ timeout }]).catch(() => {});
    else await sleep(50);
    return { did, state: await driver("state") };
  }

  // ---------- window ----------

  async function windowOp({ show, hide, minimize, size } = {}) {
    if (show) {
      reveal?.();
      showWindow(show === "front" ? true : "inactive");
    }
    if (hide) hideWindow();
    // Into the Dock or the taskbar, as the window's own button does; show
    // brings it back.
    if (minimize) win().minimize();
    if (size === null || size === "window") await cdp("Emulation.clearDeviceMetricsOverride");
    else if (size) await cdp("Emulation.setDeviceMetricsOverride", { ...parseSize(size), mobile: false });
    if (size !== undefined) await sleep(150);
    return { window: windowInfo(), viewport: (await ping()).page.ready ? (await driver("state")).viewport : null };
  }

  // ---------- requests ----------

  // While a request is in hand, and for a moment after, the window counts
  // as driven: the page keeps drawing even while hidden or covered, so
  // what a script waits for (the view settling, a screenshot) happens
  // (src/render-budget.js). The moment covers the frame a request leaves
  // to be drawn, and the next request of a script that runs several.
  const DRIVEN_TAIL_MS = 2000;
  let inHand = 0, tail = null;
  async function handle(body = {}) {
    inHand++;
    clearTimeout(tail);
    if (inHand === 1) onDriven(true);
    try {
      return await handleOne(body);
    } finally {
      if (--inHand === 0) tail = setTimeout(() => inHand === 0 && onDriven(false), DRIVEN_TAIL_MS);
    }
  }

  async function handleOne({ op, ...a } = {}) {
    consolePolicy({ op, ...a }, production);
    switch (op) {
      case "ping":
        return ping();
      case "driver":
        return driver(a.method, a.args ?? [], a.timeout ? a.timeout + 5000 : undefined);
      case "eval":
        return inPage(/\breturn\b/.test(a.expression) ? a.expression : `return (${a.expression});`, a.timeout);
      case "screenshot":
        return screenshot(a);
      case "input":
        return input(a);
      case "window":
        return windowOp(a);
      case "logs": {
        const since = Number(a.since) || 0;
        const levels = a.level === "error" ? ["error"] : a.level === "warning" ? ["error", "warning"] : null;
        const list = entries.filter((e) => e.seq > since && (!levels || levels.includes(e.level)) && (!a.source || e.source === a.source));
        return { entries: list.slice(-(a.limit ?? 200)), cursor: seq };
      }
      case "reload":
        win().webContents.reloadIgnoringCache();
        return { ok: true };
      case "quit":
        setTimeout(() => app.quit(), 100);
        return { ok: true };
      case "cdp":
        return cdp(a.method, a.params);
      // Records the window to an MP4 (see recorder.js): start, then stop.
      case "record":
        if (a.action === "stop") return recorder.stop();
        return recorder.start({ file: a.file ? path.resolve(root, a.file) : filmFile(), fps: a.fps, width: a.width, size: a.size, keep: a.keep === true });
      default:
        throw new Error(`unknown op "${op}"; try ping, driver, eval, screenshot, input, window, logs, reload, quit, cdp or record`);
    }
  }

  return { handle, log, watch };
}
