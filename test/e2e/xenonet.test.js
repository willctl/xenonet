// End to end: drives a real XenoNet Dev, the test instance with the demo's
// made-up data, through the same control channel agents use (see
// core/control.js). The window stays hidden; XENONET_E2E_SHOW=inactive
// shows it to watch, and XENONET_E2E_KEEP=1 leaves it running afterwards. A
// failing test saves a screenshot to .xenonet/e2e-<test>.png.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet, root } from "../../core/control.js";

// The test instance, on a spare port only this run knows, so nothing else
// drives it meanwhile.
const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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

// Back to the overview: nothing selected, no panel open, the pointer off the
// map (or whatever is under it shows its hover card).
async function home() {
  await s.input({ keys: "Escape", move: { x: 4, y: 4 } });
  let st = await s.driver("state");
  if (st.overlays.wallpaper) await s.driver("command", "wallpaper");
  if (st.selection) st = await s.driver("select", null);
  return st;
}

const findOne = async (query, kind) => {
  const { nodes } = await s.driver("find", { query, kind, limit: 1 });
  assert.ok(nodes.length, `the demo has a ${kind ?? "node"} matching "${query}"`);
  return nodes[0];
};

before(async () => {
  // One test instance at a time (it has a single-instance lock): any other
  // copy quits first.
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("boots on the demo world with its alerts and no page errors", { timeout: 60000 }, shoot("boot", async () => {
  const st = await s.driver("state");
  assert.ok(st.counts.projects >= 8, `projects: ${st.counts.projects}`);
  assert.ok(st.counts.sessions >= 10, `sessions: ${st.counts.sessions}`);
  // The demo has a session waiting on a question, a collision and a stuck loop.
  for (const kind of ["need", "collision", "stuck"]) assert.ok(st.alerts.some((a) => a.kind === kind), `a ${kind} alert`);
  assert.equal(st.errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("selecting a project gathers its sessions; arrows step through, Esc puts them back", { timeout: 60000 }, shoot("gather", async () => {
  await home();
  const project = await findOne("atlas-api", "project");
  let st = await s.driver("select", project.id);
  assert.equal(st.selection.id, project.id);
  assert.equal(st.gathered.root, project.id);
  assert.ok(st.gathered.sessions >= 2, `gathered sessions: ${st.gathered.sessions}`);
  assert.ok(st.motion.settled);

  // ↓ with real keys: the first gathered node, then the next.
  st = (await s.input({ keys: "ArrowDown" })).state;
  assert.equal(st.selection.id, st.gathered.members[0]);
  st = (await s.input({ keys: "ArrowDown" })).state;
  assert.equal(st.selection.id, st.gathered.members[1]);

  // Esc: back to the project, then out, and the gathering goes.
  st = (await s.input({ keys: "Escape" })).state;
  assert.equal(st.selection.id, project.id);
  st = (await s.input({ keys: "Escape" })).state;
  assert.equal(st.selection, null);
  assert.equal(st.gathered, null);
}));

test("a real click on a gathered session selects it", { timeout: 60000 }, shoot("click", async () => {
  await home();
  const project = await findOne("atlas-api", "project");
  const { gathered } = await s.driver("select", project.id);
  const session = gathered.members.find((id) => /^(claude|codex):/.test(id));
  const { did, state } = await s.input({ click: { node: session } });
  assert.ok(did.click, "clicked");
  assert.equal(state.selection?.id, session);
  // Still gathered: picking one of its sessions keeps the gathering.
  assert.equal(state.gathered?.root, project.id);
}));

test("something selected, the right column is its inspector alone; nothing selected, the rest come back", { timeout: 60000 }, shoot("context", async () => {
  await home();
  try {
    await s.window({ size: "1600x1000" });
    let st = await s.driver("state");
    assert.deepEqual(st.panels.right, ["link", "load", "tokens", "legend"]);
    const project = await findOne("atlas-api", "project");
    st = await s.driver("select", project.id);
    assert.deepEqual(st.panels.right, ["selected"], "a project: its inspector alone");
    assert.deepEqual(st.panels.left, ["filters", "sessions", "activity"], "the left column stays");
    st = await s.driver("select", "actor:wren");
    assert.deepEqual(st.panels.right, ["selected"], "an actor too");
    st = await s.driver("select", null);
    assert.deepEqual(st.panels.right, ["link", "load", "tokens", "legend"]);
  } finally {
    await s.window({ size: "window" });
  }
}));

test("the palette opens with Mod+K and runs a command with Enter", { timeout: 60000 }, shoot("palette", async () => {
  await home();
  let st = (await s.input({ keys: "Mod+K" })).state;
  assert.ok(st.overlays.palette, "palette open");
  st = (await s.input({ text: "> wallpaper" })).state;
  st = (await s.input({ keys: "Enter" })).state;
  assert.equal(st.overlays.palette, false);
  assert.equal(st.overlays.wallpaper, true);
  st = (await s.input({ keys: "W" })).state;
  assert.equal(st.overlays.wallpaper, false);
}));

test("simulated states reach the alert strip, and clear undoes them", { timeout: 60000 }, shoot("simulate", async () => {
  await home();
  const before = await s.driver("state");
  const stuck = new Set(before.alerts.filter((a) => a.kind === "stuck").map((a) => a.session));
  const { nodes } = await s.driver("find", { kind: "session", limit: 50 });
  const calm = nodes.find((n) => n.drawn && !n.stuck && !stuck.has(n.id) && !n.needsYou);
  assert.ok(calm, "a session that isn't stuck");
  const { a } = await s.simulate("stuck", { a: calm.id });
  assert.equal(a, calm.id);
  const until = Date.now() + 5000;
  let st;
  while (Date.now() < until && !(st = await s.driver("state")).alerts.some((x) => x.kind === "stuck" && x.session === calm.id)) await sleep(200);
  assert.ok(st.alerts.some((x) => x.kind === "stuck" && x.session === calm.id), "a stuck alert for it");
  await s.simulate("clear");
  await sleep(600);
  st = await s.driver("state");
  assert.ok(!st.alerts.some((x) => x.kind === "stuck" && x.session === calm.id), "cleared");
}));

test("a screenshot of the hidden window is the page at full size, and lets go of still", { timeout: 60000 }, shoot("screenshot", async () => {
  await home();
  const st = await s.driver("state");
  const shot = await s.screenshot({ name: "e2e-overview", maxWidth: 400 });
  assert.equal(shot.width, Math.round(st.viewport.width * st.viewport.dpr));
  assert.equal(shot.height, Math.round(st.viewport.height * st.viewport.dpr));
  const png = fs.readFileSync(path.join(root, shot.file));
  assert.equal(png.subarray(1, 4).toString(), "PNG");
  // A map, not a blank frame: a flat colour compresses to almost nothing.
  assert.ok(png.length > 100_000, `${png.length} bytes`);
  assert.equal(shot.image.mimeType, "image/jpeg");
  assert.equal(shot.image.width, 400);
  assert.equal((await s.driver("state")).still, false);
}));

test("still holds the view: two shots in a row match", { timeout: 60000 }, shoot("still", async () => {
  await home();
  await s.driver("still", true);
  try {
    const a = await s.screenshot({ name: "e2e-still-a", inline: false });
    const b = await s.screenshot({ name: "e2e-still-b", inline: false });
    const [pa, pb] = [a, b].map((x) => fs.readFileSync(path.join(root, x.file)));
    // Live demo activity can still change a panel's text between the two,
    // so the map is compared, not every byte.
    assert.ok(Math.abs(pa.length - pb.length) / pa.length < 0.02, `${pa.length} vs ${pb.length} bytes`);
  } finally {
    await s.driver("still", false);
  }
}));

test("identity chips filter the map to one identity's work, and hiding identities masks them", { timeout: 60000 }, shoot("identities", async () => {
  await home();
  let ids = await s.driver("identities");
  const label = (c) => c.label;
  // The demo names two identities: work (Atlas) and orbit (a side project).
  const orbit = ids.chips.find((c) => c.label === "orbit");
  assert.ok(orbit && ids.chips.some((c) => c.label === "work"), `named chips: ${ids.chips.map(label).join(", ")}`);
  assert.ok(orbit.sessions >= 2, `orbit's sessions: ${orbit.sessions}`);
  let st = await s.driver("filterIdentities", [orbit.key]);
  assert.deepEqual(st.identities.filter, [orbit.key]);
  ids = await s.driver("identities");
  assert.equal(ids.shownSessions, orbit.sessions);
  st = await s.driver("filterIdentities", []);
  assert.deepEqual(st.identities.filter, []);

  // A Claude desktop session names its account; masked, it's a letter.
  const solver = await findOne("route solver", "session");
  st = await s.driver("select", solver.id);
  assert.equal(st.selection.ranAs, "nova@example.com");
  st = await s.driver("maskIdentities", true);
  assert.equal(st.identities.masked, true);
  assert.match(st.selection.ranAs, /^Claude [A-Z]$/);
  ids = await s.driver("identities");
  for (const c of ids.chips) assert.match(c.label, /^(Claude|ChatGPT|OpenAI|GitHub|git|Identity) [A-Z]+\d*$/);
  await s.driver("maskIdentities", false);
  await home();
}));

test("a session's stats: Mod+I in the palette opens them, with a cost and a table for each chart", { timeout: 60000 }, shoot("stats", async () => {
  await home();
  const n = await findOne("partial refunds", "session");
  const opened = await s.driver("stats", n.id);
  assert.equal(opened.overlays.modal, true);
  assert.ok(opened.stats.calls > 0 && opened.stats.cost > 0, JSON.stringify(opened.stats));
  assert.deepEqual(opened.stats.models, ["claude-sonnet-4-5"]);
  const panel = await s.eval(`return { hero: document.querySelector(".st .st-kpi.hero .st-v")?.textContent, charts: document.querySelectorAll(".st [data-chart] svg").length, tables: document.querySelectorAll(".st .st-table").length }`);
  assert.match(panel.hero, /^\$\d+\.\d\d$/);
  assert.ok(panel.charts >= 3 && panel.tables >= panel.charts, JSON.stringify(panel));
  // From the palette: the session, then Mod+I.
  await home();
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "partial refunds to the orders" });
  const st = (await s.input({ keys: "Mod+I" })).state;
  assert.equal(st.overlays.palette, false);
  assert.equal(st.overlays.modal, true);
  assert.match(await s.eval(`return document.querySelector(".st h2")?.textContent`), /partial refunds/i);
  await home();
}));

test("a theme change redraws every window in the new look, and back", { timeout: 90000 }, shoot("theme", async () => {
  // Choosing the theme is yours (verify.person), and these tests run under
  // an agent, so the change is sent from the page, as XenoNet's own window
  // sends it, but straight to the server rather than through the page's
  // theme code: the new look reaches the page over the event stream, as it
  // reaches every window.
  const set = (id) => s.eval(`return fetch("/api/theme", { method: "POST", headers: { "content-type": "application/json" }, body: ${JSON.stringify(JSON.stringify({ id }))} }).then((r) => r.json())`);
  const look = () => s.eval(`return { theme: document.documentElement.dataset.theme, accent: getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() }`).catch(() => null);
  const until = async (id) => {
    for (let i = 0; i < 60; i++) {
      const l = await look();
      if (l?.theme === id) return l;
      await sleep(500);
    }
    throw new Error(`the page didn't change to ${id}`);
  };
  const t = await set("helios");
  assert.equal(t.id, "helios");
  assert.equal((await until("helios")).accent, t.colors.accent);
  assert.equal((await s.api("/api/themes")).current, "helios");
  await set("xenonet");
  assert.equal((await until("xenonet")).accent, "#39ff9f");
  await s.driver("settled", { timeout: 30000 });
}));

test("what you do is logged as steps, which play back to the same state", { timeout: 60000 }, shoot("ops", async () => {
  await home();
  await s.driver("clearOps");
  const project = await findOne("atlas-api", "project");
  await s.driver("select", project.id);
  await s.driver("step", 1);
  const there = (await s.driver("state")).selection.id;
  const steps = (await s.driver("ops")).map(({ op, args }) => ({ op, args }));
  assert.deepEqual(steps.map((x) => x.op), ["select", "select"]);
  await home();
  const { runScript } = await import("../../core/script.js");
  await runScript(s, steps);
  assert.equal((await s.driver("state")).selection.id, there);
  await home();
}));

test("a film of the day plays to its end card and records an MP4", { timeout: 90000, skip: !(await import("../../core/dirs.js")).findFfmpeg() && "no ffmpeg" }, shoot("film", async () => {
  await home();
  const file = path.join(root, ".xenonet", "e2e-film.mp4");
  fs.rmSync(file, { force: true });
  await s.record({ action: "start", file, size: [960, 540], width: 960 });
  await s.driver("film", { seconds: 3 });
  let seen = new Set();
  for (let i = 0; i < 80; i++) {
    const f = (await s.driver("state")).overlays.film;
    if (f) seen.add(f);
    else if (seen.size) break;
    await sleep(250);
  }
  const done = await s.record({ action: "stop" });
  assert.deepEqual([...seen], ["title", "playing", "end"]);
  assert.equal(done.file, file);
  assert.ok(done.bytes > 50_000 && done.frames > 200, JSON.stringify(done));
  assert.equal(fs.readFileSync(file).subarray(4, 8).toString(), "ftyp", "an MP4");
}));

test("the doctor lists what went wrong lately, copies diagnostics safe to share, and offers an agent to look at them", { timeout: 60000 }, shoot("doctor", async () => {
  await home();
  const diag = await s.api("/api/diagnose");
  assert.equal(diag.bundle.schema, "diagnostics/bundle/v1");
  assert.match(diag.markdown, /^# XenoNet diagnostics/);
  await s.driver("command", "doctor");
  const until = async (fn, ms = 8000) => {
    const end = Date.now() + ms;
    let got;
    while (!(got = await fn()) && Date.now() < end) await sleep(150);
    return got;
  };
  const lately = await until(() => s.eval(`return (() => { const l = document.querySelector("[data-dr-errors]"); return l && !/Reading/.test(l.textContent) ? l.textContent.replace(/\\s+/g, " ").trim() : null; })()`));
  assert.match(lately ?? "", /Nothing went wrong in the last day/, "the demo keeps no log");
  assert.ok(await s.eval(`return !!document.querySelector("[data-dr-copy]") && !!document.querySelector("[data-dr-ask]")`));
  await s.eval(`return document.querySelector("[data-dr-ask]").click()`);
  const view = await until(() => s.eval(`return document.querySelector("[data-dr-start]") ? document.querySelector(".dr .m-body").textContent.replace(/\\s+/g, " ") : null`));
  assert.match(view ?? "", /never files an issue, opens a pull request or sends anything/);
  await s.eval(`return document.querySelector('[data-dr-app="claude"]').click()`);
  assert.equal(await s.eval(`return document.querySelector('[data-dr-app="claude"]').getAttribute("aria-pressed")`), "true");
  // The demo's XenoNet isn't yours to ask about.
  const refused = await fetch(`${s.base}/api/diagnose/ask`, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  assert.equal(refused.status, 409);
  await s.eval(`return document.querySelector("[data-dr-back]").click()`);
  assert.ok(await until(() => s.eval(`return !!document.querySelector("[data-dr-run]")`)), "back to the checks");
  await home();
}));

test("sounds come from where things happen: an actor's act is placed on its side of the screen, and flat places nothing", { timeout: 60000 }, shoot("sound", async () => {
  await home();
  await s.driver("command", "sound-speakers");
  assert.equal((await s.driver("sound")).mode, "speakers");
  const viewport = (await s.driver("state")).viewport;
  // An actor clearly off to one side of the middle, so its side is plain.
  const { actors } = await s.driver("actors");
  const side = actors.filter((a) => a.id !== "actor:you" && a.screen && Math.abs(a.screen.x - viewport.width / 2) > 120).sort((a, b) => Math.abs(b.screen.x - viewport.width / 2) - Math.abs(a.screen.x - viewport.width / 2))[0];
  assert.ok(side, `an actor off to one side: ${JSON.stringify(actors.map((a) => [a.id, a.screen]))}`);
  const { nodes } = await s.driver("find", { kind: "session", limit: 1 });
  const since = Date.now() - 1;
  await s.simulate("actor-edit", { a: side.id, b: nodes[0].id });
  let heard = null;
  for (let i = 0; i < 40 && !heard; i++) {
    heard = (await s.driver("sound")).heard.find((h) => h.kind === "actor" && h.id === side.id && h.at >= since);
    if (!heard) await sleep(150);
  }
  assert.ok(heard, "its act was placed");
  assert.equal(Math.sign(heard.pan), Math.sign(side.screen.x - viewport.width / 2), `from its side: pan ${heard.pan}, at x ${side.screen.x} of ${viewport.width}`);
  assert.ok(heard.gain > 0 && heard.gain <= 1.25);
  await s.driver("command", "sound-flat");
  const flatSince = Date.now() - 1;
  await sleep(300);
  await s.simulate("actor-edit", { a: side.id, b: nodes[0].id });
  let flat = null;
  for (let i = 0; i < 40 && !flat; i++) {
    flat = (await s.driver("sound")).heard.find((h) => h.kind === "actor" && h.at >= flatSince);
    if (!flat) await sleep(150);
  }
  assert.deepEqual([flat?.mode, flat?.gain, flat?.pan], ["flat", 1, 0], "flat: as before");
  await s.driver("command", "sound-speakers");
}));

test("on a narrow screen the panels are one sheet the tabs pick, and a selection opens in it", { timeout: 90000 }, shoot("narrow", async () => {
  await home();
  try {
    await s.window({ size: "390x844" });
    let st = await s.driver("state");
    assert.deepEqual(st.sheet, { current: "", narrow: true });
    assert.equal(await s.eval(`return getComputedStyle(document.getElementById("sheet-tabs")).display`), "flex", "the tabs show");
    const shown = () => s.eval(`return [...document.querySelectorAll(".dock > .panel")].filter((p) => getComputedStyle(p).display !== "none").map((p) => p.dataset.sheet)`);
    assert.deepEqual(await shown(), [], "the map alone");
    st = await s.driver("sheet", "load");
    assert.equal(st.sheet.current, "load");
    assert.deepEqual(await shown(), ["load"], "only the sheet picked");
    const project = await findOne("atlas-api", "project");
    st = await s.driver("select", project.id);
    assert.equal(st.sheet.current, "selected", "a selection opens in the sheet");
    st = await s.driver("select", null);
    assert.equal(st.sheet.current, "load", "and closing it puts back what was open");
    await s.driver("sheet", null);
  } finally {
    await s.window({ size: "window" });
  }
  assert.equal((await s.driver("state")).sheet.narrow, false, "wide again, every panel in its column");
}));

test("each panel's icon sits before its title, the same as its tab's; the Link panel keeps its signal light", { timeout: 60000 }, shoot("panel-icons", async () => {
  await home();
  try {
    // Wide, where every panel is in its column and the tabs don't show.
    await s.window({ size: "1600x1000" });
    assert.equal((await s.driver("state")).sheet.narrow, false);
    const icons = await s.eval(`return [...document.querySelectorAll(".dock > .panel[data-sheet]")].map((p) => {
      const title = p.querySelector(":scope > .panel-title > span:first-child");
      const icon = title?.querySelector(".panel-icon");
      const tab = document.querySelector('[data-sheet-tab="' + p.dataset.sheet + '"] svg');
      return icon ? [p.dataset.sheet, title.firstElementChild === icon, icon.innerHTML === tab?.innerHTML, icon.getBoundingClientRect().width > 0] : [p.dataset.sheet];
    })`);
    const has = (sheet) => [sheet, true, true, true];
    assert.deepEqual(icons, [has("filters"), has("sessions"), has("activity"), ["link"], has("load"), has("tokens"), ["selected"], has("legend")], "first in its title, its tab's, and shown");
    assert.ok(await s.eval(`return !!document.querySelector(".dock > .status #link-led svg")`), "the Link panel's own light");
  } finally {
    await s.window({ size: "window" });
  }
}));

test("This computer's panel shows the demo's made-up load, each core and the system's mark, and the palette hides and shows it", { timeout: 60000 }, shoot("system", async () => {
  await home();
  const until = async (fn, ms = 8000) => {
    const end = Date.now() + ms;
    let got;
    while (!(got = await fn()) && Date.now() < end) await sleep(150);
    return got;
  };
  const sys = await until(async () => {
    const v = await s.driver("system");
    return v?.shown && v.gpu && v.history > 1 ? v : null;
  });
  assert.ok(sys, "the panel shows a reading");
  assert.deepEqual([sys.source, sys.platform, sys.cpu.cores, sys.gpu.name], ["demo", "linux", 16, "NVIDIA GeForce RTX 4080"]);
  assert.match(sys.text, /\d+% CPU/);
  assert.equal(await s.eval(`return document.querySelectorAll('#sysmon [data-cores] i').length`), 16, "a bar for each core");
  assert.equal(await s.eval(`return document.querySelector('#sysmon-tick svg')?.getAttribute('aria-label')`), "Linux", "the system's mark, not its name");
  await s.driver("command", "system-panel");
  assert.equal((await s.driver("system")).shown, false, "hidden from the palette");
  await s.driver("command", "system-panel");
  assert.equal((await s.driver("system")).shown, true, "and back");
}));

test("Token use says about what today's work would cost, and what the demo's made-up OpenRouter key has spent", { timeout: 60000 }, shoot("spend", async () => {
  await home();
  const until = async (fn, ms = 8000) => {
    const end = Date.now() + ms;
    let got;
    while (!(got = await fn()) && Date.now() < end) await sleep(150);
    return got;
  };
  const sp = await until(async () => {
    const v = await s.driver("spend");
    return v?.shown && v.costs?.today > 0 && v.gauge ? v : null;
  });
  assert.ok(sp, "the estimate and the OpenRouter row show");
  assert.deepEqual(sp.costs.unpriced, [], "the demo's models have prices");
  assert.match(sp.text, /about \$\d+\.\d\d today/i);
  assert.match(sp.gauge, /openrouter.*\$21\.40/i);
  assert.equal(sp.openrouter.limit, 50);
}));

test("Spotify's chip shows what's playing in the demo's made-up player, pauses it, and XenoNet's music steps aside while it plays", { timeout: 60000 }, shoot("spotify", async () => {
  await home();
  const until = async (fn, ms = 8000) => {
    const end = Date.now() + ms;
    let got;
    while (!(got = await fn()) && Date.now() < end) await sleep(150);
    return got;
  };
  const shown = await until(async () => {
    const sp = await s.driver("spotify");
    return sp?.shown ? sp : null;
  });
  assert.ok(shown, "the chip shows");
  assert.match(shown.text, /Night Drive/);
  assert.equal(shown.yielding, false, "paused, so XenoNet's music plays");
  await s.eval(`return document.querySelector('#spotify [data-sp="play"]').click()`);
  const playing = await until(async () => {
    const sp = await s.driver("spotify");
    return sp?.now?.playing ? sp : null;
  });
  assert.ok(playing, "playing");
  assert.equal(playing.yielding, true, "XenoNet's music steps aside while Spotify plays");
  await s.eval(`return document.querySelector('#spotify [data-sp="pause"]').click()`);
  const paused = await until(async () => {
    const sp = await s.driver("spotify");
    return sp && !sp.now?.playing ? sp : null;
  });
  assert.equal(paused?.yielding, false, "and comes back when it stops");
  const outside = await fetch(`${s.base}/api/spotify/player`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ op: "pause" }) });
  assert.equal(outside.status, 403, "only the window plays and pauses");
}));

test("no page errors after all that", { timeout: 30000 }, async () => {
  const { entries } = await s.logs({ level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
});
