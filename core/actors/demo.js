// The demo world's actors (contracts, section 3): you, and three made-up
// agents at work on the demo's own projects and sessions (core/demo.js):
//
//   Wren     a local agent (Hermes Agent), observed and attributed: it
//            works on atlas-api and ledger and starts Claude Code sessions
//            in them;
//   Kestrel  a cloud agent (Grok Bot) that runs commands in pathfinder
//            through its desktop app on this machine, seen only while the
//            app runs;
//   Moth     a custom actor that registered itself: it watches geofence
//            and says things to the agents there.
//
// Each plays a script a step at a time, like the demo's sessions: a few
// hours of it before now, so there's history, and then a step every few
// seconds while the server runs. The scripts go through the registry's own
// calls, the ones observers, registered actors and the window use (an
// observer's look, a heartbeat, facts XenoNet writes the details from, the
// window's reports), so the demo shows what real actors would. Names and
// versions are made up; the products are ones XenoNet means to observe.
import path from "node:path";
import { world, demoSessionList, demoClaudeAccount } from "../demo.js";
import { pathKey } from "../paths.js";
import { claudeId } from "../identities.js";
import { YOU } from "../schema/ids.js";

const WREN = "actor:wren";
const KESTREL = "actor:kestrel";
const MOTH = "actor:moth";
const MIN = 60e3;

export const STEP_MS = 3000;
// Whose turn each tick is: the agents in turn, and you now and then.
const TURNS = [WREN, KESTREL, MOTH, WREN, KESTREL, MOTH, YOU];
// The instance each agent reports as: Hermes's gateway, Grok Bot's app, and
// the program Moth runs as.
const INSTANCE = { [WREN]: "gateway", [KESTREL]: "app", [MOTH]: "main" };

const project = (rel) => `project:${pathKey(path.join(world.projectsDir, ...rel.split("/")))}`;
const session = (title) => demoSessionList().find((s) => s.title === title)?.id ?? null;
const file = (rel) => path.join(world.projectsDir, ...rel.split("/"));

export const DEMO_ACTORS = () => [
  {
    id: WREN,
    name: "Wren",
    product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" },
    via: ["demo", "observed", "attributed"],
    heartbeatMs: 60000,
    capabilities: ["spawn-sessions", "edit-repos", "run-commands", "message-people", "schedule"],
    identity: claudeId(demoClaudeAccount("ledger").account, demoClaudeAccount("ledger").org),
  },
  {
    id: KESTREL,
    name: "Kestrel",
    product: { id: "grok-bot", vendor: "SpaceXAI", version: null },
    via: ["demo", "observed"],
    // A cloud agent sends no heartbeat, so it never turns away.
    heartbeatMs: null,
    capabilities: ["run-commands", "browse", "message-people"],
  },
  {
    id: MOTH,
    name: "Moth",
    via: ["demo", "registered"],
    heartbeatMs: 30000,
    capabilities: ["message-agents", "message-people"],
  },
];

// Each actor's script, looped. An agent's step is what its source would
// say: `presence` and `status` (a look or a heartbeat), `targets` it acts
// on, or an `event` as facts. Yours is what the window reports.
export const DEMO_SCRIPTS = () => ({
  [WREN]: [
    { presence: "working" },
    { status: "Triaging issues in atlas-api", targets: [[project("atlas-api"), "works-on"]] },
    { event: { kind: "spawned", target: session("Add partial refunds to the orders API"), via: "attributed" } },
    { event: { kind: "edited", target: project("atlas-api"), paths: [file("atlas-api/src/orders/refunds.ts")], via: "attributed" } },
    { event: { kind: "ran", target: project("atlas-api"), count: 3, via: "attributed" } },
    { event: { kind: "finished", target: project("atlas-api"), subject: "session" } },
    { status: "Closing the month in ledger", targets: [[project("ledger"), "works-on"]] },
    { event: { kind: "spawned", target: session("Draft the Q4 close checklist"), via: "attributed" } },
    { event: { kind: "messaged", platform: "telegram" } },
    { presence: "idle", status: "" },
  ],
  [KESTREL]: [
    { presence: "idle" },
    { presence: "working", status: "Benchmarking the route solver", targets: [[project("pathfinder"), "works-on"]] },
    { event: { kind: "ran", target: project("pathfinder"), count: 1 } },
    { event: { kind: "ran", target: project("pathfinder"), count: 4 } },
    { event: { kind: "failed", target: project("pathfinder"), subject: "command" } },
    { event: { kind: "ran", target: project("pathfinder"), count: 2 } },
    { event: { kind: "finished", target: project("pathfinder"), subject: "task" } },
    // Its app closed: nothing here can say whether it stopped.
    { presence: "unknown", status: "" },
  ],
  [MOTH]: [
    { presence: "working" },
    { status: "Watching geofence", targets: [[project("labs/geofence"), "watches"]] },
    { event: { kind: "said", target: session("Draw delivery zones from postcodes"), count: 1 } },
    { event: { kind: "messaged", platform: "email" } },
    { presence: "idle" },
  ],
  [YOU]: [
    { console: { presence: "working", did: [{ what: "looked", node: project("atlas-api") }] } },
    { console: { did: [{ what: "read", node: session("Review the partial refunds change") }] } },
    { console: { did: [{ what: "started", node: project("atlas-api"), app: "claude" }] } },
    { console: { did: [{ what: "terminal", node: project("ledger") }] } },
    { console: { presence: "idle" } },
  ],
});

// Adds the demo's actors to `actors`, with the last few hours of their
// scripts behind them, and drives the scripts on while it runs. Returns
// step(), which plays the next turn (what the driver does every STEP_MS).
export function addDemoActors(actors, { history = 3 * 3600e3 } = {}) {
  const now = actors.now();
  const scripts = DEMO_SCRIPTS();
  const at = new Map(Object.keys(scripts).map((id) => [id, 0]));
  // An actor you removed, or whose product's observer you turned off, is
  // left out until the demo starts again.
  const playing = (id) => actors.has(id) && actors.enabled(id) && !actors.simulationHeld(id);
  const play = (id, when) => {
    const script = scripts[id];
    const step = script[at.get(id) % script.length];
    at.set(id, at.get(id) + 1);
    if (!playing(id)) return null;
    // You turned yourself off the map: the made-up you keeps still until
    // the page reports again.
    if (id === YOU && actors.you.off()) return null;
    if (step.console) return actors.you.report(step.console, { at: when });
    if (step.presence !== undefined || step.status !== undefined) actors.report(id, { instance: INSTANCE[id], presence: step.presence, status: step.status, at: when });
    for (const [node, relation] of step.targets ?? []) actors.target(id, node, relation, when);
    if (step.event) actors.record(id, { ...step.event, at: when });
  };
  // Those that are there say so between their steps: an observer looking
  // again, a heartbeat, the window's minute.
  const beat = (when) => {
    for (const id of actors.ids()) {
      if (actors.simulationHeld(id)) continue;
      const p = actors.presence(id);
      if ((p !== "working" && p !== "idle" && p !== "away") || !actors.enabled(id)) continue;
      if (id === YOU) actors.you.report({}, { at: when });
      else if (INSTANCE[id]) actors.report(id, { instance: INSTANCE[id], at: when });
    }
  };
  // Twice through each script before now, the turns spread evenly and the
  // last a minute ago, written in one go.
  actors.store.transaction(() => {
    for (const def of DEMO_ACTORS()) actors.upsert({ ...def, at: now - history });
    const turns = Math.ceil((Math.max(...Object.values(scripts).map((s) => s.length)) * 2 * TURNS.length) / 3);
    for (let k = 0; k < turns; k++) {
      const when = Math.round(now - history + ((history - MIN) * k) / (turns - 1));
      beat(when);
      play(TURNS[k % TURNS.length], when);
    }
    beat(now);
  });
  let turn = 0;
  const step = () => {
    const when = actors.now();
    beat(when);
    return play(TURNS[turn++ % TURNS.length], when);
  };
  actors.drive(STEP_MS, step);
  return step;
}
