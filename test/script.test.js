// Running steps and scripts (core/script.js) against a stand-in for XenoNet
// Dev's control client, which records what it was asked.
import { test } from "node:test";
import assert from "node:assert/strict";
import { runStep, runScript } from "../core/script.js";

const STATE = { selection: null, gathered: null, overlays: { modal: false, film: false }, alerts: [], motion: { settled: true } };

function standIn({ nodes = ["project:atlas"], filmFrames = 2 } = {}) {
  const calls = [];
  let film = 0;
  const s = {
    port: 1,
    calls,
    async driver(method, ...args) {
      calls.push([method, ...args]);
      if (method === "select") {
        if (args[0] && !nodes.includes(args[0])) throw new Error(`XenoNet has no node "${args[0]}"; find lists the ids`);
        return { ...STATE, selection: args[0] ? { id: args[0] } : null };
      }
      if (method === "find") return { nodes: nodes.filter((n) => n.includes(args[0].query.split(" ")[0])).map((id) => ({ id })) };
      if (method === "film") return (film = filmFrames), { started: true };
      if (method === "state") return { ...STATE, overlays: { ...STATE.overlays, film: film-- > 0 ? "playing" : false } };
      if (method === "nonesuch") throw new Error("the driver has no nonesuch()");
      return STATE;
    },
    async screenshot(opts) {
      calls.push(["screenshot", opts]);
      return { file: `.xenonet/${opts.name}.png`, width: 10, height: 10, notes: [] };
    },
  };
  return s;
}

test("steps as strings: known ones, words for ids, and any driver method", async () => {
  const s = standIn();
  const said = [];
  await runScript(s, ["select:atlas", "shot:map", "clearOps", "palette:refunds"], (v) => said.push(v));
  assert.deepEqual(s.calls.map((c) => c[0]), ["select", "find", "select", "screenshot", "clearOps", "palette"]);
  assert.deepEqual(s.calls[2], ["select", "project:atlas"], "the best match for the words");
  assert.deepEqual(s.calls[5], ["palette", "refunds"], "a step it doesn't know goes to the driver with its argument");
  assert.equal(said[0].selection, "project:atlas");
  await assert.rejects(runStep(s, "nonesuch"), /no step "nonesuch"/);
});

test("steps as { op, args }, as the console logs them: driver calls, waits, shots and films", async () => {
  const s = standIn();
  const t0 = Date.now();
  await runScript(s, [{ op: "select", args: ["project:atlas"] }, { op: "wait", args: [30] }, { op: "shot", args: ["x"] }, { op: "film", args: [{ seconds: 3 }] }]);
  assert.ok(Date.now() - t0 >= 30);
  assert.deepEqual(s.calls.map((c) => c[0]), ["select", "screenshot", "film", "state", "state", "state"], "the film is waited out");
  assert.deepEqual(s.calls[2], ["film", { seconds: 3 }]);
});

// A stand-in for every call a step can make, which records each one.
function fullStandIn(server = null) {
  const s = standIn({ nodes: ["project:atlas", "claude:s1"] });
  const record = (name, value) => async (...args) => (s.calls.push([name, ...args]), typeof value === "function" ? value(...args) : value);
  Object.assign(s, {
    name: "XenoNet Dev (demo)",
    port: server?.address().port ?? 1,
    launch: record("launch", { page: { ready: true } }),
    quit: record("quit", { quit: true }),
    ping: record("ping", null),
    simulate: record("simulate", { ok: true }),
    input: record("input", (o) => ({ did: Object.keys(o) })),
    window: record("window", { window: {} }),
    eval: record("eval", 2),
    logs: record("logs", (o) => (o?.limit === 1 ? { cursor: 7, entries: [] } : { entries: [{ level: "error", source: "page", text: "boom" }] })),
  });
  const driver = s.driver;
  s.driver = async (method, ...args) => {
    if (method === "stats") return (s.calls.push([method, ...args]), { overlays: { stats: true, modal: false }, stats: { id: args[0] } });
    if (method === "commands") return (s.calls.push([method]), [{ id: "wallpaper", label: "Wallpaper" }]);
    if (method === "identities") return (s.calls.push([method]), { chips: [] });
    if (method === "actors") return (s.calls.push([method]), { actors: [{ id: "actor:wren", presence: "working", title: "Wren", status: "Triaging issues", beams: ["project:atlas"] }] });
    if (method === "actorRequests") return (s.calls.push([method]), { requests: [{ id: "rq1", actor: "actor:heron", name: "Heron", asker: { program: "~/bin/heron", parents: [], folder: null } }] });
    if (method === "feed") return (s.calls.push([method, ...args]), { lines: [{ kind: "spawned", actor: "actor:wren", title: "Wren", detail: "Started a Claude Code session in atlas-api", at: 1 }, { kind: "turn-end", session: "claude:s1", title: "Refunds", detail: "", at: 0 }] });
    if (method === "settled") return (s.calls.push([method]), { waitedMs: 0 });
    if (method === "still") return (s.calls.push([method, ...args]), true);
    if (method === "find" && args[0].kind === "session") return (s.calls.push([method, ...args]), { nodes: args[0].query === "none" ? [] : [{ id: "claude:s1" }] });
    if (method === "broken") throw new Error("the page fell over");
    return driver(method, ...args);
  };
  return s;
}

test("every kind of step reaches the instance the way it should", async () => {
  const s = fullStandIn();
  const said = [];
  const steps = [
    "launch", "launch:hidden", "launch:restart", "launch:restart:hidden", "launch:front:restart", "status", "state", "deselect", "back", "reset", "next", "prev",
    "stats:claude:s1", "stats:refunds", "command:wallpaper", "commands", "simulate:stuck", "simulate:actor-spawn,actor:wren,claude:s1", "simulate:actor-offline,actor:wren", "simulate:actor-needs-you,actor:wren", "actors", "feed", "feed:5",
    "identities", "identities:a,b", "identities:all", "mask:on", "mask:off", "filters", "filters:grok=off,archived=on",
    "key:Mod+K", "type:hello", "click:10,20", "click2:@claude:s1", "hover:@project:atlas",
    "drag:1,2,3,4", "alt-drag:1,2,3,4", "scroll:5,6,-120", "size:1600x900@2", "size:window",
    "show", "show:front", "hide", "still:on", "still:off", "settle", "eval:1+1", "logs", "listen:10",
    "sleep:5", "replay:", "quit",
  ];
  await runScript(s, steps, (v) => said.push(v));
  const calls = s.calls.map((c) => JSON.stringify(c));
  for (const expected of [
    ["launch", {}], ["launch", { show: "hidden" }], ["launch", { restart: true }], ["launch", { restart: true, show: "hidden" }], ["launch", { show: "front", restart: true }], ["ping"], ["select", null], ["back"], ["reset"], ["step", 1], ["step", -1],
    ["stats", "claude:s1", { settle: false }], ["find", { query: "refunds", kind: "session", limit: 1 }], ["command", "wallpaper"], ["simulate", "stuck"],
    ["simulate", "actor-spawn", { a: "actor:wren", b: "claude:s1" }], ["simulate", "actor-offline", { a: "actor:wren" }], ["simulate", "actor-needs-you", { a: "actor:wren" }], ["actors"], ["feed", { limit: 20 }], ["feed", { limit: 5 }],
    ["filterIdentities", ["a", "b"]], ["filterIdentities", []], ["maskIdentities", true], ["maskIdentities", false], ["filters", { grok: false, archived: true }],
    ["input", { keys: "Mod+K" }], ["input", { text: "hello" }], ["input", { click: { x: 10, y: 20 }, count: 1 }], ["input", { click: { node: "claude:s1" }, count: 2 }], ["input", { move: { node: "project:atlas" } }],
    ["input", { drag: { from: { x: 1, y: 2 }, to: { x: 3, y: 4 } }, modifiers: [] }], ["input", { drag: { from: { x: 1, y: 2 }, to: { x: 3, y: 4 } }, modifiers: ["Alt"] }], ["input", { scroll: { x: 5, y: 6, dy: -120 } }],
    ["window", { size: "1600x900@2" }], ["window", { size: null }], ["window", { show: "inactive" }], ["window", { show: "front" }], ["window", { hide: true }],
    ["still", true], ["still", false], ["settled"], ["eval", "1+1"], ["logs", { level: "warning" }], ["logs", { limit: 1 }], ["logs", { since: 7, level: "warning" }], ["replay"], ["quit"],
  ]) assert.ok(calls.includes(JSON.stringify(expected)), `${JSON.stringify(expected)} in ${calls.join(" ")}`);
  assert.ok(said.includes("XenoNet Dev (demo) isn't running"), "status says when it isn't running");
  assert.ok(said.includes("wallpaper  Wallpaper"));
  assert.ok(said.includes("actor:wren  working  Wren · Triaging issues  → 1 beam"), "the actors, a line each");
  assert.ok(said.includes("actor:heron  waiting  Heron asks to join · asked by ~/bin/heron"), "and the programs waiting for an Allow");
  assert.ok(said.includes("spawned  Wren · Started a Claude Code session in atlas-api") && said.includes("turn-end  Refunds"), "the feed, a line each");
  assert.ok(said.includes("ERROR [page] boom"));
  await assert.rejects(runStep(s, "launch:restart:hiden"), /launch takes restart, inactive, front or hidden, not "hiden"/, "a typo never shows the window");
  await assert.rejects(runStep(s, "stats:none"), /no session matches "none"/);
  await assert.rejects(runStep(s, "select:nowhere"), /nothing on the map matches "nowhere"/);
  await assert.rejects(runStep(s, "broken"), /the page fell over/, "other errors pass through");
});

test("a made-up registration's token stays out of what simulate prints", async () => {
  const s = fullStandIn();
  s.simulate = async (kind, named) => (s.calls.push(["simulate", kind, named]), { ok: true, kind, a: "actor:heron", request: "rq1", token: `xenonet_actor_${"0".repeat(64)}`, expiresAt: 1 });
  const said = [];
  await runScript(s, ["simulate:actor-request,Heron"], (v) => said.push(v));
  assert.deepEqual(s.calls.at(-1), ["simulate", "actor-request", { a: "Heron", b: undefined }]);
  assert.deepEqual(said, [{ ok: true, kind: "actor-request", a: "actor:heron", request: "rq1", expiresAt: 1 }]);
});

test("a theme step switches through the page and waits until it's drawn, and says when it can't", async () => {
  const s = fullStandIn();
  const driver = s.driver;
  s.driver = async (method, ...args) => {
    if (method === "theme" && args[0] !== "noir") throw new Error(`there's no theme called ${args[0]}`);
    return driver(method, ...args);
  };
  const said = [];
  await runStep(s, { op: "theme", args: ["noir"] }, (v) => said.push(v));
  assert.deepEqual(said, ["theme noir"]);
  assert.deepEqual(s.calls.at(-1), ["theme", "noir"]);
  await assert.rejects(runStep(s, "theme:mars"), /there's no theme called mars/);
  await runStep(s, "previewTheme:noir", (v) => said.push(v));
  assert.deepEqual(s.calls.at(-1), ["previewTheme", "noir"], "previews go to the driver like any method");
  await runStep(s, "film:2", (v) => said.push(v));
  assert.ok(s.calls.some((c) => c[0] === "film" && c[1].seconds === 2));
});

test("a root step says which projects folder XenoNet reads, or reads another", async () => {
  const s = fullStandIn();
  const driver = s.driver;
  s.driver = async (method, ...args) => {
    if (method === "root") return (s.calls.push([method]), { path: "/home/nova/Projects", source: "default" });
    if (method === "setRoot") return (s.calls.push([method, ...args]), { ...STATE, root: { path: args[0] ?? "/home/nova/Projects" } });
    return driver(method, ...args);
  };
  const said = [];
  await runScript(s, ["root", "root:~/Clients", "root:reset", { op: "setRoot", args: ["~/Clients"] }], (v) => said.push(v));
  assert.deepEqual(s.calls, [["root"], ["setRoot", "~/Clients"], ["setRoot", null], ["setRoot", "~/Clients"]]);
  assert.equal(said[0], "/home/nova/Projects  default");
  assert.deepEqual([said[1].root, said[2].root, said[1].settled], ["~/Clients", "/home/nova/Projects", true]);
});

test("a hosts step lists the other machines on the map, or shows only some of them", async () => {
  const s = fullStandIn();
  const driver = s.driver;
  s.driver = async (method, ...args) => {
    if (method === "hosts") return (s.calls.push([method]), { hosts: [{ id: "vm1", state: "live", sessions: 3, drawn: true }, { id: "studio", state: "offline", stale: true, sessions: 2, drawn: true }] });
    if (method === "filterHosts") return (s.calls.push([method, ...args]), STATE);
    return driver(method, ...args);
  };
  const said = [];
  await runScript(s, ["hosts", "hosts:vm1,@here", "hosts:all"], (v) => said.push(v));
  assert.deepEqual(s.calls, [["hosts"], ["filterHosts", ["vm1", "@here"]], ["hosts"], ["filterHosts", []], ["hosts"]]);
  assert.deepEqual(said.slice(0, 2), ["vm1  live  3 sessions", "studio  offline  (not live)  2 sessions"]);
});
