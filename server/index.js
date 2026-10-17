import http from "node:http";
import { componentService } from "./component-service.js";
import { refusalRoutes } from "./routes/refusals.js";
import { integrationRoutes } from "./routes/integrations.js";
import { createIntegrations } from "../core/integrations/index.js";
import { demoRefusals } from "../core/refusals/demo.js";
import { SERVICE_API, CONSOLE_API, CAPABILITIES } from "../core/components/protocol.js";
let components = null;
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { config } from "../core/config.js";
import { XenoNetState } from "../core/state.js";
import { urlFor, launch, handoffUrl, MAX_BRIEF, NEEDS_INPUT_URL } from "../core/open.js";
import { codexClient } from "../core/codex.js";
import * as store from "../core/store.js";
import { Engine, isActorAgent } from "../core/engine/engine.js";
import { attest } from "../core/engine/attest.js";
import { bearerOf as bearerHeader } from "../core/actors/tokens.js";
import { YOU } from "../core/schema/ids.js";
import { EngineStore } from "../core/engine/store.js";
import { createVerifier } from "../core/engine/verify.js";
import { createProcs } from "../core/procs.js";
import { engineRoutes } from "./routes/engine.js";
import { deliveryRoutes } from "./routes/delivery.js";
import { DeliveryPolicy } from "../core/delivery/policy.js";
import { DeliveryRouter } from "../core/delivery/router.js";
import { Channels } from "../core/delivery/channel.js";
import { createOpenCode } from "../core/delivery/opencode.js";
import { createTmux } from "../core/delivery/tmux.js";
import { createResume } from "../core/delivery/resume.js";
import { planUsage } from "../core/delivery/usage.js";
import { writeListenSettings } from "../core/delivery/setup.js";
import { actorRoutes, createActorCaller } from "./routes/actors.js";
import { createActors, mapOf, CLOCK_MS } from "../core/actors/index.js";
import { createRegistration } from "../core/actors/registration.js";
import { createWindowKey } from "../core/actors/key.js";
import { workspaceRoutes } from "./routes/workspace.js";
import { themeRoutes } from "./routes/themes.js";
import { mcpRoutes } from "./routes/mcp.js";
import { environmentRoutes } from "./routes/environments.js";
import { hostRoutes } from "./routes/hosts.js";
import { HostsConfig } from "../core/hosts/config.js";
import { aliasReader } from "../core/hosts/aliases.js";
import { staticFile } from "./static.js";
import { createConnect } from "../core/engine/connect.js";
import { doctorContext } from "../core/doctor/context.js";
import { createDoctorService } from "../core/doctor/service.js";
import { summarize, machine } from "../core/doctor/index.js";
import { createTokenScan } from "../core/actors/token-file.js";
import { commandState, currentRuntime, installCommand } from "../core/command.js";
import { readThemes, copyTheme } from "../core/themefiles.js";
import { DEFAULT_THEME } from "../core/themes.js";
import { drift, DRIFT_FILE } from "../core/drift.js";
import { driftRoutes } from "./routes/drift.js";
import { createConnections, fingerprintOf } from "../core/actors/connections.js";
import { createRemotePresence } from "../core/actors/remote-presence.js";
import { assistantDir } from "../core/assistant/credentials.js";
import { stateSource } from "../core/assistant/source.js";
import { createAudit } from "../core/assistant/audit.js";
import { createAssistantService } from "../core/assistant/service.js";
import { assistantEndpoint } from "./assistant-endpoint.js";
import { assistantRoutes } from "./routes/assistant.js";
import { createTunnels } from "../core/assistant/tunnels.js";
import { createSecretStore, createMemorySecrets } from "../core/secret-store.js";
import { createLog, logDir, readLogs, summarize as summarizeLogs, LEVELS } from "../core/log.js";
import { makeBundle, bundleMarkdown, writeLook, askCommand } from "../core/diagnostics.js";
import { tilde } from "../core/paths.js";
import { createRefusals, contextOn } from "../core/refusals/store.js";
import { createDecider, DecisionError } from "../core/decide.js";
import { createSpotify, createDemoSpotify, duckPlan, SpotifyError } from "../core/spotify.js";
import { createSystemStats, createDemoSystemStats } from "../core/system-stats.js";
import { createOpenRouterUsage, createDemoOpenRouterUsage } from "../core/openrouter-usage.js";
import { createUpstream, upstreamOf } from "./upstream.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VERSION = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
// The app's name, which the desktop app passes on (XenoNet, XenoNet Dev…).
const NAME = process.env.XENONET_NAME || (config.demo ? "XenoNet (demo)" : "XenoNet");
const dev = process.argv.includes("--dev");
// XenoNet Dev reading another XenoNet, read-only (upstream.js): development
// only, never the installed service.
const upstream = dev && process.env.XENONET_UPSTREAM ? createUpstream(upstreamOf(process.env.XENONET_UPSTREAM)) : null;
// Which XenoNet this is: installed, or a development instance by name.
const INSTANCE = process.env.XENONET_INSTANCE || (process.env.XENONET_STANDALONE === "1" || process.env.XENONET_LAUNCHER_EXEC ? "installed" : config.demo ? "demo" : dev ? "dev" : null);
// The service's error log (core/log.js), in the state folder; the demo's
// errors stay in its console. Anything thrown that nothing caught goes in
// it before the process ends as it would have.
const log = createLog({ dir: config.demo ? null : logDir(config.stateDir), component: "service", instance: INSTANCE, version: VERSION, home: config.homeDir });
process.on("uncaughtExceptionMonitor", (err, origin) => log.error(origin === "unhandledRejection" ? "service.unhandled-rejection" : "service.uncaught", err?.message ?? String(err), { err }));
// What the lenient readers find in other apps' data (core/vendor.js), kept
// in the cache folder for npm run contracts and the doctor.
drift.open(path.join(config.cacheDir, DRIFT_FILE));
const state = new XenoNetState();
const clients = new Set();
// The engine: what the agents on this machine are doing, and the means for
// them to work together (see engine/engine.js). Inside the desktop app,
// XENONET_EXEC is the app itself, which agents' tools run on as Node.
const engineStore = new EngineStore(path.join(config.stateDir, "engine.sqlite"));
state.blocks.add(engineStore.block);
// Actors beside the sessions (core/actors), kept in engine.sqlite: you, the
// demo's made-up ones, and registered and observed actors, whom agents see
// among them in the engine. XENONET_ACTORS=off turns the layer off.
const actors = config.actors ? createActors({ store: engineStore, demo: config.demo, map: mapOf(state) }) : null;
// Delivery: scopes and guards on messages, and the router that puts them
// into live sessions (core/delivery).
const deliveryPolicy = new DeliveryPolicy({ file: path.join(config.stateDir, "delivery.json"), auditFile: path.join(config.stateDir, "delivery-audit.jsonl") });
const engine = new Engine({ state, store: engineStore, actors, policy: deliveryPolicy });
const channels = new Channels();
// Requests in an agent's name, or with an actor's token, are checked
// against the process they come from (see core/engine/verify.js). Under
// the desktop app, the server's parent is XenoNet itself, whose window is
// yours. The observers read the same process table, shared for a moment.
const procs = createProcs();
const verify = createVerifier({ procs, store: engineStore, home: process.parentPort ? process.ppid : null });
// Registering actors, which wait for your Allow in the window, and the
// tokens they report with (core/actors/registration.js).
const registration = actors ? createRegistration({ actors }) : null;
const actorCaller = registration ? createActorCaller({ registration, verify }) : null;
actors?.drive(CLOCK_MS, () => registration.tick());
// The observers of Hermes Agent, Grok Bot and Muse (core/actors/observers.js),
// each on unless you turned it off; never in the demo, whose actors are
// made up and whose machine isn't read. Loaded only where they run.
const observers = actors && !config.demo ? (await import("../core/actors/observers.js")).createObservers({ actors, registration, procs, home: config.homeDir }) : null;
for (const o of observers?.list ?? []) state.blocks.add(o);
const connect = createConnect({ port: config.port, root: process.env.XENONET_LAUNCHER_ROOT || root, demo: config.demo, exec: process.env.XENONET_LAUNCHER_EXEC || process.env.XENONET_EXEC || process.execPath, runAsNode: !process.env.XENONET_LAUNCHER_EXEC && !!process.env.XENONET_EXEC });

const ALLOWED_HOSTS = new Set([`127.0.0.1:${config.port}`, `localhost:${config.port}`]);
const ALLOWED_ORIGINS = new Set([...ALLOWED_HOSTS].map((h) => `http://${h}`));

// XenoNet Dev's control channel (electron/control.js) is answered by the
// desktop app's main process; this server relays requests to it over the
// utility process's port, so the channel shares the API's address and its
// Host and Origin checks. Only XenoNet Dev sets XENONET_CONTROL.
const controlled = process.env.XENONET_CONTROL === "1" && !!process.parentPort;
const relays = new Map();
let relaySeq = 0;
process.parentPort?.on("message", ({ data }) => {
  if (data?.type !== "control-result") return;
  relays.get(data.id)?.(data);
  relays.delete(data.id);
});
// The desktop app says whether its window shows, which is part of your
// presence (core/actors/you.js): hidden is away, closed is offline.
process.parentPort?.on("message", ({ data }) => {
  if (data?.type === "you") actors?.you.window(data.window);
});
function relay(body) {
  const ms = 30e3 + Math.min(Number(body?.timeout) || 0, 120e3);
  return new Promise((resolve) => {
    const id = ++relaySeq;
    const timer = setTimeout(() => (relays.delete(id), resolve({ error: `XenoNet Dev didn't answer in ${ms / 1000}s`, late: true })), ms);
    relays.set(id, (data) => (clearTimeout(timer), resolve(data)));
    process.parentPort.postMessage({ type: "control", id, body });
  });
}

function send(res, status, body, headers = {}) {
  const data = typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", ...headers });
  res.end(data);
}

// A request's JSON body. A body that's too big or isn't JSON is the
// client's mistake, and says so (`status`).
async function readJson(req, limit = 1 << 20) {
  let size = 0;
  const chunks = [];
  for await (const c of req) {
    size += c.length;
    if (size > limit) throw Object.assign(new Error("the body is too big"), { status: 413 });
    chunks.push(c);
  }
  try {
    return chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : {};
  } catch {
    throw Object.assign(new Error("the body isn't JSON"), { status: 400 });
  }
}

// An event for every page. With hosts on, the stream a relay follows
// (?hosts=0) gets this machine's own view, `local`, which is null for what
// came from another host: nothing passes from one machine to the next.
function broadcast(event, data, local = data) {
  const msg = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
  const own = local === data ? msg : local == null ? null : `event: ${event}\ndata: ${JSON.stringify(local)}\n\n`;
  for (const res of clients) {
    const m = res.ownOnly ? own : msg;
    if (m) res.write(m);
  }
}
state.on("pulse", (p) => broadcast("pulse", p));
state.on("activity", (a) => broadcast("activity", a));
// Decisions XenoNet hands to a decision model on OpenRouter, when you ask
// (core/decide.js): its key in your credential store; the demo's in memory.
const decisionSecrets = config.demo ? createMemorySecrets() : createSecretStore();
const decider = createDecider({ secrets: decisionSecrets, log });
const integrations = createIntegrations({ dir: config.demo ? null : path.join(config.stateDir, "integrations"), home: config.homeDir, decider, nodes: () => state.graph?.nodes ?? [], demo: config.demo });
// What OpenRouter's key has spent, for Token use (core/openrouter-usage.js):
// read only while a window asks, through GET /api/spend.
const openrouter = config.demo ? createDemoOpenRouterUsage() : createOpenRouterUsage({ secrets: decisionSecrets, log });

// Spotify, as a remote for the app you use (core/spotify.js): what's
// playing goes to the page (spotify), a few seconds apart while a window
// is open; and with the dip on, your volume dips on this computer while a
// session waits on you. The demo's player is made up.
const spotify = config.demo ? createDemoSpotify() : createSpotify({ file: path.join(config.stateDir, "spotify.json"), secrets: createSecretStore(), port: config.port, log });
// What this computer is doing, for the window's system panel
// (core/system-stats.js): read only while a window asks.
const system = config.demo ? createDemoSystemStats() : createSystemStats();
const music = { last: "", held: null, quietUntil: 0, timer: null };
async function pollSpotify() {
  clearTimeout(music.timer);
  let wait = clients.size ? 5000 : 30000;
  try {
    const st = await spotify.status();
    if (st.connected) {
      const now = await spotify.player();
      const json = JSON.stringify(now);
      if (json !== music.last) (music.last = json), broadcast("spotify", now);
      if (st.duck) {
        const waiting = [...(state.live?.sessions?.values() ?? [])].some((s) => s.needsYou?.(Date.now()));
        const plan = duckPlan({ waiting, player: now, device: st.device, held: music.held });
        music.held = plan.held;
        if (plan.op === "volume") await spotify.command("volume", plan.volume);
      }
    }
  } catch (err) {
    // Spotify's asked XenoNet to wait, needs Premium, or isn't reachable:
    // say so once in a while, and look again later.
    if (Date.now() > music.quietUntil) log.warn(`spotify.${err.code ?? "error"}`, err.message), (music.quietUntil = Date.now() + 10 * 60e3);
    wait = 60000;
  }
  music.timer = setTimeout(pollSpotify, wait);
  music.timer.unref?.();
}

// Every provider refusal XenoNet sees, kept in the state folder for a
// dataset (core/refusals): with its context when you've turned that on. A
// new one seen live is a line in the log and, a few an hour at most, a
// notification; the activity feed has it from the session already.
const refusals = createRefusals({ dir: path.join(config.stateDir, "refusals"), home: config.homeDir, context: () => contextOn(path.join(config.stateDir, "refusals")) });
const demoRefusalRecords = config.demo ? demoRefusals() : null;
const refusalNotes = { at: [], bySession: new Map() };
state.on("refusal", (ev) => {
  let rec = null;
  try {
    rec = refusals.record(ev.signal, ev);
  } catch (err) {
    return log.warn("refusals.record", `couldn't keep a refusal: ${err.message}`, { err });
  }
  if (!rec || !ev.live) return;
  log.info("refusals.seen", `${rec.provider} ${rec.kind}${rec.signal.category ? ` (${rec.signal.category})` : ""} in ${ev.sessionId}`);
  broadcast("refusal", { id: rec.id, at: rec.at, provider: rec.provider, kind: rec.kind, category: rec.signal.category, session: ev.sessionId, context: !!rec.context });
  const t = Date.now();
  refusalNotes.at = refusalNotes.at.filter((x) => t - x < 3600e3);
  if (config.demo || rec.kind !== "refusal" || refusalNotes.at.length >= 6 || t - (refusalNotes.bySession.get(ev.sessionId) ?? 0) < 10 * 60e3) return;
  refusalNotes.at.push(t);
  refusalNotes.bySession.set(ev.sessionId, t);
  const node = state.index.get(ev.sessionId);
  notifyDesktop({ type: "notify", key: `refusal:${ev.sessionId}`, id: ev.sessionId, title: `${rec.provider === "openai" ? "OpenAI" : "Anthropic"} refused a turn`, subtitle: (node?.title ?? ev.sessionId).slice(0, 80), body: (rec.signal.explanation ?? rec.signal.category ?? "XenoNet kept a record of it.").slice(0, 160) });
});
state.on("tokens", (t) => broadcast("tokens", t));
engine.on("message", (m) => broadcast("agent-message", m));
// A message an actor sent becomes its said event, never its text.
engine.on("message", (m) => actors?.said(m));
const engineRoute = engineRoutes({ engine, connect, verify, readJson, send, actorCaller, actors, channels });
// The settings a Claude Code session starts with to listen after its turns
// (xenonet doctor --live), which a resumed turn gets too.
let listenFile = null;
if (!config.demo) {
  try {
    listenFile = writeListenSettings({ dir: config.stateDir, port: config.port });
  } catch (err) {
    console.error(`[xenonet] listen settings: ${err.message}`);
  }
}
const deliveryRouter = new DeliveryRouter({
  engine,
  policy: deliveryPolicy,
  channels,
  opencode: createOpenCode(),
  tmux: createTmux(),
  // Made-up sessions are never resumed.
  resume: config.demo ? null : createResume({ settings: listenFile }),
  // A provider at 90% of its plan takes no wakes: from the live readings.
  usage: (app) => planUsage({ ...state.live?.limits, claude: state.live?.claudeLimits?.() ?? null }, app),
  notify: ({ agent, title, from }) => {
    if (config.demo) return;
    notifyDesktop({ type: "notify", key: `delivery:${agent}`, id: agent, title: "A message wasn't picked up", subtitle: title, body: `${from}'s message to ${title} is still waiting: the session hasn't started a turn in a minute.` });
  },
});
engine.delivery = deliveryRouter;
deliveryRouter.on("delivery", (d) => broadcast("delivery", d));
deliveryRouter.on("error", (err) => console.error(`[xenonet] delivery: ${err.message}`));
const deliveryRoute = deliveryRoutes({ policy: deliveryPolicy, router: deliveryRouter, verify, readJson, send });
// The actors' list goes out when it changes, and so do the map's changes
// to it; each event as it happens; and how many registrations wait for
// you, which the window then reads with its key.
actors?.on("actors", (list) => broadcast("actors", list));
actors?.on("event", (e) => broadcast("actor-event", e));
registration?.on("requests", (r) => broadcast("actor-requests", r));
registration?.on("asked", (r) => notifyRequest(r));
registration?.on("asked-gateway", (r) => notifyGatewayAsk(r));
state.on("graph", () => actors?.refresh());
// The key for the routes only XenoNet's own window may use (see
// core/actors/key.js): made now, and handed to the desktop app once the
// server is up.
const windowKey = createWindowKey();
// The actor gateway (server/actor-gateway.js), for actors outside this
// machine: off until you turn it on, on a port of its own beside this one,
// put on the internet by the tunnel you pick (server/gateway-control.js).
// XENONET_GATEWAY_PROBE=local checks it without the internet, for tests.
const gatewayControl =
  actors && registration
    ? (await import("./gateway-control.js")).createGatewayControl({
        engine,
        registration,
        actors,
        // Runs an actor reports from elsewhere (a sandbox): sessions on the
        // map, each with a beam from its actor.
        runs: state.reportedRuns,
        onRun: ({ actor, id }) => state.refreshAfterChange().then(() => actors.target(actor, id, "works-on")).catch(() => {}),
        store: engineStore,
        port: Number(process.env.XENONET_GATEWAY_PORT) || config.port + 10,
        version: VERSION,
        dir: config.demo ? null : config.stateDir,
        probe: process.env.XENONET_GATEWAY_PROBE === "local" ? "local" : null,
        log: (code, detail) => (code === "gateway.error" ? log.warn(code, detail) : log.info(code, detail)),
      })
    : null;
// Waking actors when a message arrives (core/actors/wake.js): a stream for
// a program of their own, a webhook you give, or a notification to you. A
// webhook's URL and key go to the credential store; the demo's to memory.
const wakes =
  actors && registration
    ? (await import("../core/actors/wake.js")).createWakes({
        engine,
        actors,
        store: engineStore,
        secrets: config.demo ? createMemorySecrets() : createSecretStore(),
        version: VERSION,
        notify: notifyMail,
        log: (code, detail) => log.warn(code, detail),
      })
    : null;
const actorRoute = actorRoutes({ actors, registration, caller: actorCaller, windowKey, verify, gateway: gatewayControl, wakes, readJson, send });

// Remote actors (docs/design/cloud-assistants.md): connections you pair in
// the window let an actor such as a dot read what's going on here through
// xenonet mcp assistant, on a private endpoint of its own that listens only
// while a grant could be used. They read this machine's state only, never
// other hosts'. XENONET_ASSISTANT=off leaves all of it out.
const connections = actors && process.env.XENONET_ASSISTANT !== "off"
  ? createConnections({
      actors,
      store: engineStore,
      dir: assistantDir(config.stateDir),
      fingerprint: fingerprintOf({ stateDir: config.stateDir }),
      describe: (place) => state.index.get(place)?.name ?? null,
      taken: (slug) => engineStore.waitingActorRequests(Date.now()).some((r) => r.slug === slug),
      port: config.port,
    })
  : null;
const assistantSource = connections ? stateSource({ state, actors }) : null;
const assistantAudit = connections ? createAudit({ store: engineStore }) : null;
// Each read, at most one an actor a second, goes to the page as light
// through the actor's tunnel to what it read (remote-read); nothing of it
// is kept, and the feed hears nothing of it.
const remoteRead = ({ actor, connection, reach, at }) => broadcast("remote-read", { actor, transport: engineStore.connection(connection)?.transport_kind ?? null, reach, at });
const assistantService = connections ? createAssistantService({ connections, source: assistantSource, remote: createRemotePresence({ actors, onRead: remoteRead }), audit: assistantAudit, version: VERSION }) : null;
const assistant = connections ? assistantEndpoint({ connections, service: assistantService, audit: assistantAudit, env: { ...process.env, XENONET_PORT: String(config.port) }, log: (line) => console.log(`[xenonet] ${line}`) }) : null;
if (connections) actors.drive(60e3, () => connections.tick());
// Tunnels XenoNet runs for those connections (core/assistant/tunnels.js):
// tunnel-client as a child of this service, with the tunnel's key from
// your credential store. The adapter it runs is XenoNet's launcher, as the
// hooks' is. In the demo, whose connections are made up, a key stays in
// memory and tunnel-client is only what XENONET_TUNNEL_CLIENT names.
const tunnels = connections
  ? createTunnels({
      connections,
      store: engineStore,
      dir: assistantDir(config.stateDir),
      secrets: config.demo ? createMemorySecrets() : createSecretStore(),
      ...(config.demo ? { find: () => process.env.XENONET_TUNNEL_CLIENT || null, profiles: () => [] } : {}),
      home: config.homeDir,
      launch: { exec: process.env.XENONET_LAUNCHER_EXEC || process.env.XENONET_EXEC || process.execPath, entry: path.join(process.env.XENONET_LAUNCHER_ROOT || root, "cli", "xenonet.js"), runAsNode: !process.env.XENONET_LAUNCHER_EXEC && !!process.env.XENONET_EXEC, port: config.port },
      log: (line) => console.log(`[xenonet] ${line}`),
    })
  : null;
// The window reads how they stand with its key; this says only that it
// changed.
tunnels?.on("changed", () => broadcast("assistant", { at: Date.now() }));
const assistantRoute = assistantRoutes({ connections, tunnels, audit: assistantAudit, source: assistantSource, state, endpoint: assistant, windowKey, verify, readJson, send, version: VERSION });

// Each building block's health (core/blocks), as /api/health and the
// doctor have it: name, kind, state, since, lastOk, lastError, counters.
const blocks = () => state.blocks.health().map(({ budget, ...health }) => health);
const workspaceRoute = workspaceRoutes({ state, verify, readJson, send });

// Your standing settings, the ones that change what XenoNet reads or shows
// (the projects folder, your rules for it, Codex's picks for it, which
// asking for spends your Codex usage, identities' names, hidden flags and
// pins, the theme), are yours, as connecting agents and adding hosts are:
// from XenoNet's window or your own terminal, never from under an agent or
// an actor product (verify.person). Answers 403 and is false otherwise.
async function yours(req, res) {
  const you = await verify.person(req.socket);
  if (!you.ok) send(res, 403, { error: you.reason, code: "yours" });
  return you.ok;
}

// Hosts (docs/design/hosts.md): this XenoNet as a host, for xenonet relay,
// which needs no flag (hosts.json is read the first time it's asked for),
// and with XENONET_HOSTS=1, the hub that shows other machines' XenoNets on this
// map, read-only. Without the flag, nothing here starts and the map is
// this machine's alone.
const hostsConfig = new HostsConfig({ stateDir: () => config.stateDir });
const hub =
  process.env.XENONET_HOSTS === "1"
    ? (await import("../core/hosts/hub.js")).createHub({
        hosts: hostsConfig,
        cacheDir: config.cacheDir,
        root,
        home: config.homeDir,
        demo: config.demo,
        windows: config,
        local: {
          graph: () => state.graph,
          live: () => state.liveSnapshot(),
          layout: () => store.getLayout(),
          identities: () => store.getIdentities(),
          aliases: aliasReader(config.homeDir),
          forgetLayout(prefix) {
            const gone = Object.keys(store.getLayout()).filter((id) => id.startsWith(prefix));
            store.setLayout(Object.fromEntries(gone.map((id) => [id, null])));
            return gone.length;
          },
        },
        linkOptions: { exec: process.env.XENONET_EXEC || process.execPath, runAsNode: !!process.env.XENONET_EXEC },
      })
    : null;
if (hub) state.blocks.add(hostsConfig.store);
const hostRoute = hostRoutes({ hosts: hostsConfig, hub, verify, readJson, send, home: config.homeDir });
const mcpRoute = mcpRoutes({ state, demo: config.demo, hub, send });
// The map and the live state go out with other hosts' in them, with hosts
// on; the hub sends them again when a host's map or live state changes.
state.on("graph", (g) => broadcast("graph", hub ? hub.graph(g) : g, g));
state.on("live", (s) => broadcast("live", hub ? hub.live(s) : s, s));
if (hub) for (const event of ["graph", "live", "activity", "tokens", "pulse", "agent-message", "hosts"]) hub.on(event, (data) => broadcast(event, data, null));

// The doctor's look for actors' tokens left in the repos XenoNet knows, one
// for the server's life: the half-hourly runs take the day's look, and a
// run you ask for yourself looks again (the decision at merge point B).
const tokenScan = actors ? createTokenScan({ repos: () => [...state.index.values()].filter((n) => n.kind === "project" && n.git && n.path).map((n) => n.path), run: machine().run }) : null;

// The doctor (see core/doctor): every half hour, and on request. New problems
// show in the console and, from the desktop app, as a notification. The demo
// checks only itself, never your machine, and stays quiet.
const doctor = createDoctorService({
  only: config.demo ? ["xenonet.server", "xenonet.sources", "xenonet.blocks", "contracts.drift", "claude.transcripts", "codex.sessions", "system.projects", "system.runtime", "hosts"] : null,
  file: config.demo ? null : path.join(config.stateDir, "doctor.json"),
  context: async ({ byHand = false } = {}) =>
    doctorContext({
      self: { port: config.port, name: NAME },
      health: { sources: state.sources, blocks: blocks() },
      live: state.liveSnapshot(),
      connect: config.demo ? null : await connect.preview().catch(() => null),
      stateLoad: store.loadReport(),
      drift: () => drift.view(),
      // What the process check refused because it couldn't see who sent it.
      verify: () => verify.state,
      // Whether you asked for this run yourself (xenonet doctor, Check again).
      byHand,
      // Actors, for their checks: tokens that expired, what the observers
      // read, and the look for token files in repos.
      actors: actors
        ? {
            expired: () => engineStore.expiredActorTokens(),
            observers: () => observers?.status() ?? [],
            tokenFiles: (opts) => tokenScan.look(opts),
            gateway: gatewayControl ? () => gatewayControl.status() : null,
            wakes: wakes ? () => wakes.list() : null,
          }
        : null,
      hosts: hub ? () => hub.list() : null,
    }),
  onResult: (r) => broadcast("doctor", r),
  onNewProblems: (fresh) => {
    notifyDoctor(fresh);
    // Each new problem goes in the log too, so a failed check is still
    // there to read after the next run passes.
    for (const p of fresh) log[p.status === "fail" ? "error" : "warn"](`doctor.${p.id}`, `${p.title}: ${p.detail}`, { fix: p.fix });
  },
});

// Drift in the apps' data (routes/drift.js), for xenonet drift and the
// console; once you've looked, the doctor runs again without its warning:
// a fresh run, since one already under way looked before you did.
const driftRoute = driftRoutes({ log: drift, home: config.homeDir, readJson, send, onSeen: () => doctor.check({ fresh: true }).catch(() => {}) });

// The theme in use (see core/themes.js); yours live in the state folder's
// themes/. A change there reaches every page at once: the theme in use,
// which a page draws again only if it changed, and that the list changed.
const themesDir = path.join(config.stateDir, "themes");
const currentTheme = () => {
  const { themes } = readThemes(themesDir);
  return themes[store.getTheme()] ?? themes[DEFAULT_THEME];
};
const themeRoute = themeRoutes({ dir: themesDir, send, readJson, quiet: config.demo });
const environmentRoute = environmentRoutes({ store, yours, send, readJson, broadcast });
try {
  fs.mkdirSync(themesDir, { recursive: true });
  let pending = null;
  fs.watch(themesDir, () => {
    clearTimeout(pending);
    pending = setTimeout(() => (broadcast("theme", currentTheme()), broadcast("themes", {})), 150);
  });
} catch {}

// An app's link, a folder or a page, opened by the desktop app the window
// is in when there is one. Windows brings an app to the front only for the
// app you're using, so one opened from this background process opens behind
// your windows, and looks as if nothing happened. Without a desktop app, or
// with one too old to open things, the server opens it itself.
async function open(target) {
  if (await components?.open(target)) return;
  if (process.parentPort) return void process.parentPort.postMessage({ type: "open", target });
  launch(target);
}

// A notification from the desktop app about new problems; clicking it opens
// the doctor.
function notifyDesktop(note) {
  process.parentPort?.postMessage(note);
  components?.notify(note);
}

function notifyDoctor(fresh) {
  if (config.demo) return;
  const worst = fresh.find((r) => r.status === "fail") ?? fresh[0];
  const title = fresh.length === 1 ? "XenoNet found a problem" : `XenoNet found ${fresh.length} problems`;
  const detail = worst.detail.charAt(0).toUpperCase() + worst.detail.slice(1);
  notifyDesktop({ type: "notify", key: "doctor", command: "doctor", title, subtitle: `${worst.group}: ${detail}`.slice(0, 120), body: worst.fix ?? "Open XenoNet's doctor for the details." });
}

// A registration waiting for you: a notification, which the desktop app
// shows when the window isn't in front, and which opens the Actors panel
// (the page's "actors" command). The demo stays quiet.
function notifyRequest({ name }) {
  if (config.demo) return;
  notifyDesktop({ type: "notify", key: "actor-request", command: "actors", title: "An actor asks to join XenoNet", subtitle: `"${name}" waits for you to allow it`, body: "Allow or deny it in XenoNet's Actors panel." });
}

// A session asked for an actor's gateway token. The notification says whose,
// never a token: none exists until you allow the ask in the window.
function notifyGatewayAsk({ actor, name }) {
  if (config.demo) return;
  notifyDesktop({ type: "notify", key: `gateway-ask:${actor}`, command: "actors", title: "A session asks for a gateway token", subtitle: `For ${name}'s app`, body: "Allow it in XenoNet's Actors panel. The token is made there and shown masked." });
}

// Messages waiting for an actor nothing can wake, such as Muse: a
// notification, which opens the Actors panel. It says how many, never who
// sent them or what they say. The demo stays quiet.
function notifyMail({ actor, name, unread }) {
  if (config.demo) return;
  notifyDesktop({ type: "notify", key: `actor-mail:${actor}`, command: "actors", title: `${name} has ${unread === 1 ? "a message" : `${unread} messages`} waiting`, subtitle: `Nothing can wake ${name}, so it reads them when it next checks`, body: "Ask it to check its XenoNet inbox, or set up a wake in the Actors panel." });
}

// Dev only: puts real sessions into states that are otherwise rare (a
// pending question, an edit collision, a stuck loop, a wiki write, a problem
// the doctor found) so the visuals can be checked. In memory only; "clear"
// undoes it. The actor-… kinds have an actor act instead (see
// core/actors), and actor-request makes up a registration that waits, in
// the demo and test instances only. The host-… kinds go to the hub (other
// hosts, with XENONET_HOSTS=1). What it did, or null for an unknown
// simulation or session.
function simulate(kind, { a, b, ...more }) {
  if (kind === "actor-request") return registration?.simulate(kind, { a }) ?? null;
  // A read through an actor's made-up tunnel (actor-tunnel), as a remote
  // actor's read reaches the page: of node b, or the places it acts on.
  if (kind === "actor-read") {
    if (!actors?.demo) return null;
    a ??= actors.ids().find((id) => actors.remoteOf(id)?.active);
    if (!a || !actors.remoteOf(a)) return null;
    const reach = b ? [b] : [...new Set(actors.targetsOf(a).map((t) => t.id))].slice(0, 4);
    broadcast("remote-read", { actor: a, transport: actors.remoteOf(a).transports?.[0] ?? null, reach, at: Date.now() });
    return { ok: true, kind, a, reach };
  }
  if (String(kind).startsWith("actor-")) return actors?.simulate(kind, { a, b }) ?? null;
  if (String(kind).startsWith("host-")) return hub?.simulate(kind, { a, b, ...more }) ?? null;
  const live = state.live;
  const now = Date.now();
  // Without sessions named, the most recently active ones on the map.
  if (kind !== "clear" && (!a || (kind === "collision" && !b))) {
    const recent = [...live.sessions.values()].filter((s) => state.index.has(s.id)).sort((x, y) => (y.lastEventAt ?? 0) - (x.lastEventAt ?? 0));
    a ??= recent[0]?.id;
    b ??= recent.find((s) => s.id !== a)?.id;
  }
  const sa = live.sessions.get(a), sb = live.sessions.get(b);
  if (kind === "needs-you" && sa) {
    sa.question = { callId: "sim-q", text: "Should I run the migration against the shared staging database, or only the local fixture?", ts: now };
    sa.inflight.set("sim-q", { name: "AskUserQuestion", ts: now });
    sa.lastEventAt = now;
    live.activity(sa, { kind: "needs-you", detail: sa.question.text, ts: now }, true);
  } else if (kind === "collision" && sa && sb) {
    const file = path.join(config.projectsDir, "ledger", "scripts", "local-dev", "app.ps1");
    sa.addEdits([{ path: file, kind: "edit", diff: "- $port = 3000\n+ $port = 3030\n  Start-Process node" }], now - 90e3);
    sb.addEdits([{ path: file, kind: "update", diff: "@@ -12,3 +12,4 @@\n-$port = 3000\n+$port = 3100\n+$env:NODE_ENV = 'development'" }], now - 20e3);
    sa.files.set(file, now - 90e3);
    sb.files.set(file, now - 20e3);
  } else if (kind === "stuck" && sa) {
    sa.resetStall();
    sa.question = sa.hookNeed = null;
    sa.inflight.clear();
    sa.turn = { active: true, startedAt: now - 120e3 };
    sa.errorStreak = 4;
    sa.lastErrorAt = now;
    sa.errorSince = now - 20e3;
    sa.recentTools.push(...[0, 1, 2, 3].map((i) => ({ key: "shell|npm run db:push", label: "npm run db:push", loopable: true, ok: false, ts: now - (4 - i) * 5e3 })));
    sa.lastEventAt = now;
  } else if (kind === "turn-end" && sa) {
    live.activity(sa, { kind: "turn-end", duration: 42000, ts: now }, true);
  } else if (kind === "wiki-write" && sa) {
    live.activity(sa, { kind: "tool", tool: "mcp__agent-wiki__wiki_log", category: "mcp", detail: "agent-wiki.wiki_log", server: "agent-wiki", pages: ["xenonet", "ledger"], files: [], ts: now }, true);
  } else if (kind === "doctor") {
    // The doctor finding a problem, reported as a real run would be; the
    // next real run (or clear) puts back what's true.
    const problem = { id: "codex.auth", group: "Codex", title: "Codex is signed in", status: "fail", detail: "not signed in", fix: "Run codex login.", ms: 1 };
    const results = [...(doctor.last?.results ?? []).filter((r) => r.id !== problem.id), problem];
    doctor.report({ at: now, ms: 1, results, summary: summarize(results) });
    notifyDoctor([problem]);
    return { ok: true, kind };
  } else if (kind === "clear") {
    doctor.check({ fresh: true }).catch(() => {});
    // An actor waiting on you (actor-needs-you) stops waiting too.
    actors?.simulate("clear");
    for (const s of live.sessions.values()) {
      if (s.question?.callId === "sim-q") (s.question = null), s.inflight.delete("sim-q");
      s.errorStreak = 0;
      s.recentTools = s.recentTools.filter((t) => t.key !== "shell|npm run db:push");
      for (const [k, list] of s.edits) if (k.endsWith("local-dev/app.ps1")) s.edits.delete(k);
    }
  } else return null;
  broadcast("live", state.liveSnapshot());
  return { ok: true, kind, ...(kind === "clear" ? {} : { a }), ...(kind === "collision" ? { b } : {}) };
}

async function api(req, res, url) {
  const route = url.pathname;
  const requested = req.headers["x-xenonet-api-version"];
  if (requested && requested !== "1") return send(res, 426, { error: "This XenoNet Service does not support the requested API version.", api: SERVICE_API });

  // You, writing to an actor from XenoNet's own window (the actor's
  // inspector): only with the window's key, never with a token, and only
  // to an actor that reads its inbox. Its envelope says it came from the
  // window (core/engine/attest.js).
  if (req.method === "POST" && route === "/api/actors/you/message") {
    if (bearerHeader(req.headers) !== undefined || !windowKey.matches(req.headers["x-xenonet-window"])) return send(res, 403, { error: "this is for XenoNet's own window", code: "yours" });
    const w = verify.window ? await verify.window(req.socket) : { ok: true };
    if (!w.ok) return send(res, 403, { error: w.reason, code: "yours" });
    const body = await readJson(req, 16 << 10).catch(() => null);
    const to = typeof body?.to === "string" ? body.to : "";
    if (!isActorAgent(to)) return send(res, 400, { error: "write to an actor here (actor:<slug>); sessions get messages from other sessions and actors", code: "bad-request" });
    try {
      return send(res, 200, await engine.send(YOU, { to, text: body.text }, { envelope: attest({ sender: { id: YOU, kind: "you", name: "you" }, method: "window", home: config.homeDir }) }));
    } catch (err) {
      return send(res, 400, { error: String(err?.message ?? err), code: "bad-request" });
    }
  }
  if (route.startsWith("/api/engine/") || route === "/api/hook/codex") return engineRoute(req, res, url);
  if (route === "/api/delivery" || route.startsWith("/api/delivery/")) return deliveryRoute(req, res, url);
  if (route === "/api/actors" || route.startsWith("/api/actors/")) return actorRoute(req, res, url);
  if (route === "/api/assistant" || route.startsWith("/api/assistant/")) return assistantRoute(req, res, url);
  if (await hostRoute(req, res, url)) return;

  if (req.method === "GET" && route === "/api/health") return send(res, 200, { ok: true, name: NAME, version: VERSION, api: SERVICE_API, consoleApi: CONSOLE_API, capabilities: components ? CAPABILITIES : [], demo: config.demo, instance: INSTANCE, pid: process.pid, sources: state.sources, blocks: blocks() });
  // The error log (core/log.js), scrubbed as it was written: ?since= (ms),
  // ?level=error|warn|info, ?component=, ?code= (a prefix), ?summary=1 for
  // each code once with how often.
  // The page's uncaught errors, which it sends here (src/main.js): in the
  // log as page.uncaught, scrubbed and held to ten seconds apart like any.
  if (req.method === "POST" && route === "/api/logs/page") {
    const body = await readJson(req, 8192).catch(() => null);
    if (body && typeof body.message === "string") log.error("page.uncaught", body.message.slice(0, 1000), { err: typeof body.stack === "string" && body.stack ? { message: body.message, stack: body.stack.slice(0, 3000) } : null });
    return send(res, 200, { ok: true });
  }
  if (req.method === "GET" && route === "/api/logs") {
    const q = url.searchParams;
    const entries = config.demo ? [] : readLogs(logDir(config.stateDir), { since: Number(q.get("since")) || Date.now() - 24 * 3600e3, level: LEVELS.includes(q.get("level")) ? q.get("level") : "warn", component: q.get("component") || null, code: q.get("code") || null, limit: Math.min(1000, Number(q.get("limit")) || 200) });
    return send(res, 200, q.get("summary") === "1" ? { at: Date.now(), summary: summarizeLogs(entries) } : { at: Date.now(), entries });
  }

  // What OpenRouter's key has spent, with the key XenoNet keeps for decision
  // models; at most once a minute.
  if (req.method === "GET" && route === "/api/spend") return send(res, 200, { openrouter: await openrouter.status() });

  // This computer's CPU, memory and GPU, for the system panel
  // (core/system-stats.js), with the last few minutes of each.
  if (req.method === "GET" && route === "/api/system") return send(res, 200, await system.read());

  // Themes (core/themes.js, and yours in the state folder's themes/): the
  // list, the one in use, a switch, and a copy of one to make your own; one
  // by id is in routes/themes.js.
  if (await themeRoute(req, res, url)) return;
  if (await mcpRoute(req, res, url)) return;
  if (await environmentRoute(req, res, url)) return;
  if (route === "/api/themes" && req.method === "GET") {
    const { themes, problems } = readThemes(themesDir);
    const current = currentTheme().id;
    return send(res, 200, { current, folder: themesDir, problems, themes: Object.values(themes).map((t) => ({ id: t.id, name: t.name, about: t.about ?? "", builtIn: t.builtIn, ...(t.replacesBuiltIn ? { replacesBuiltIn: true } : {}), swatch: [t.colors.bg, t.colors.accent, t.colors.claude, t.colors.codex, t.colors.attention] })) });
  }
  if (route === "/api/theme" && req.method === "GET") return send(res, 200, currentTheme());
  if (route === "/api/theme" && req.method === "POST") {
    if (!(await yours(req, res))) return;
    const { id } = await readJson(req);
    if (!readThemes(themesDir).themes[id]) return send(res, 404, { error: `there's no theme called ${id}` });
    store.setTheme(id);
    broadcast("theme", currentTheme());
    return send(res, 200, currentTheme());
  }
  if (route === "/api/themes/copy" && req.method === "POST") {
    const { from, id, name } = await readJson(req);
    try {
      return send(res, 200, { file: copyTheme(themesDir, from ?? currentTheme().id, String(id ?? ""), name) });
    } catch (err) {
      return send(res, 409, { error: err.message });
    }
  }

  // The xenonet command on your PATH (core/command.js), run by this app's
  // runtime so it works without Node.
  if (route === "/api/command") {
    if (req.method === "GET") return send(res, 200, commandState());
    if (req.method === "POST") {
      try {
        return send(res, 200, installCommand({ entry: path.join(process.env.XENONET_LAUNCHER_ROOT || root, "cli", "xenonet.js"), ...(process.env.XENONET_LAUNCHER_EXEC ? { runtime: process.env.XENONET_LAUNCHER_EXEC } : currentRuntime()) }));
      } catch (err) {
        return send(res, 409, { error: err.message });
      }
    }
  }

  if (route === "/api/drift" || route.startsWith("/api/drift/")) return driftRoute(req, res, url);

  // The doctor's last results (running it first if it hasn't run), or a new
  // run (xenonet doctor, the console's Check again, an agent's doctor tool).
  // Any program may ask for one, but only yours counts as by hand and looks
  // through every repo for tokens afresh; an agent's gets the day's look.
  if (route === "/api/doctor") {
    if (req.method === "GET") return send(res, 200, doctor.last ?? (await doctor.check()));
    if (req.method === "POST") return send(res, 200, await doctor.check({ byHand: (await verify.person(req.socket)).ok }));
  }

  // Spotify (core/spotify.js). GET /api/spotify: how it stands and what's
  // playing. The window's, with its key: POST /app {clientId}, /connect
  // (where to sign in), /player {op, value}, /settings {device, duck}.
  // Yours: POST /disconnect. GET /callback is Spotify sending you back.
  if (route === "/api/spotify" || route.startsWith("/api/spotify/")) {
    const fromWindow = () => bearerHeader(req.headers) === undefined && windowKey.matches(req.headers["x-xenonet-window"]);
    try {
      if (req.method === "GET" && route === "/api/spotify") {
        const st = await spotify.status();
        return send(res, 200, { ...st, now: st.connected ? await spotify.player().catch(() => null) : null });
      }
      if (req.method === "GET" && route === "/api/spotify/callback") {
        const q = url.searchParams;
        const page = (title, text) => send(res, 200, `<!doctype html><meta charset="utf-8"><title>XenoNet</title><body style="font:15px system-ui;background:#05070d;color:#e8f4ff;display:grid;place-items:center;height:100vh;margin:0"><div><h2>${title}</h2><p>${text}</p></div></body>`, { "content-type": "text/html; charset=utf-8" });
        try {
          await spotify.callback({ code: q.get("code"), state: q.get("state"), error: q.get("error") });
          pollSpotify();
          return page("Spotify is connected", "You can close this tab and go back to XenoNet.");
        } catch (err) {
          return page("Spotify isn't connected", String(err.message).replace(/[<>&]/g, ""));
        }
      }
      if (req.method === "POST" && route === "/api/spotify/disconnect") return (await yours(req, res)) ? send(res, 200, await spotify.disconnect()) : undefined;
      if (req.method === "POST") {
        if (!fromWindow()) return send(res, 403, { error: "this is for XenoNet's own window", code: "yours" });
        const body = (await readJson(req, 2048).catch(() => null)) ?? {};
        if (route === "/api/spotify/app") return send(res, 200, spotify.setApp(body.clientId));
        if (route === "/api/spotify/connect") return send(res, 200, spotify.begin());
        if (route === "/api/spotify/settings") {
          const device = body.device === "current" ? ((await spotify.player().catch(() => null))?.device?.id ?? null) : body.device;
          return send(res, 200, spotify.set({ device, duck: body.duck }));
        }
        if (route === "/api/spotify/player") {
          await spotify.command(String(body.op ?? ""), body.value);
          music.last = "";
          setTimeout(pollSpotify, 400);
          return send(res, 200, { ok: true });
        }
      }
    } catch (err) {
      if (err instanceof SpotifyError) return send(res, err.status, { error: err.message, code: err.code });
      throw err;
    }
  }

  // The decision model (core/decide.js): GET how it stands (its model,
  // whether a key is kept, never the key); POST /key {key}, with the
  // window's key, keeps it in your credential store; DELETE /key forgets
  // it.
  if (route === "/api/decide" || route === "/api/decide/key") {
    try {
      if (req.method === "GET" && route === "/api/decide") return send(res, 200, await decider.status());
      if (route === "/api/decide/key" && req.method === "POST") {
        if (bearerHeader(req.headers) !== undefined || !windowKey.matches(req.headers["x-xenonet-window"])) return send(res, 403, { error: "this is for XenoNet's own window", code: "yours" });
        const stored = await decider.storeKey((await readJson(req, 2048))?.key);
        openrouter.forget();
        return send(res, 200, stored);
      }
      if (route === "/api/decide/key" && req.method === "DELETE") {
        if (!(await yours(req, res))) return;
        openrouter.forget();
        return send(res, 200, await decider.forgetKey());
      }
    } catch (err) {
      if (err instanceof DecisionError) return send(res, err.status, { error: err.message, code: err.code });
      throw err;
    }
  }

  if (await refusalRoutes({ req, res, url, refusals, yours, readJson, send, demoRecords: demoRefusalRecords })) return;
  if (await integrationRoutes({ req, res, url, integrations, yours, readJson, send, owner: async (req, res) => {
    if (!windowKey.matches(req.headers["x-xenonet-window"])) { send(res, 403, { error: "Use XenoNet's own window to send decisions or launch jobs.", code: "yours" }); return false; }
    const seen = verify.window ? await verify.window(req.socket) : { ok: true };
    if (!seen.ok) send(res, 403, { error: seen.reason, code: "yours" });
    return seen.ok;
  } })) return;

  // What XenoNet can tell someone helping you, safe to share
  // (core/diagnostics.js): GET the bundle, as JSON and Markdown; POST
  // /ask {app, model} makes a folder for an agent to look at it with you
  // and says the command that starts it. Starting an agent is yours to
  // do, never another agent's.
  if (route === "/api/diagnose" || route === "/api/diagnose/ask") {
    const bundle = async () =>
      makeBundle({
        xenonet: { version: VERSION, instance: INSTANCE, packaged: !!process.env.XENONET_LAUNCHER_EXEC, running: true },
        doctor: doctor.last,
        errors: config.demo ? [] : readLogs(logDir(config.stateDir), { since: Date.now() - 24 * 3600e3, level: "warn" }),
        connect: config.demo ? null : await connect.preview().catch(() => null),
        launcher: await doctorContext().launcher().catch(() => null),
        home: config.homeDir,
      });
    if (route === "/api/diagnose" && req.method === "GET") {
      const b = await bundle();
      return send(res, 200, { bundle: b, markdown: bundleMarkdown(b) });
    }
    if (route === "/api/diagnose/ask" && req.method === "POST") {
      // The demo has nothing of yours to ask about, whoever asks: said
      // before the check that it's you, since /api/health says it's a demo.
      if (config.demo) return send(res, 409, { error: "this is the demo: ask about the XenoNet with your own sessions", code: "demo" });
      if (!(await yours(req, res))) return;
      const body = await readJson(req, 4096);
      const app = body?.app === "claude" ? "claude" : "codex";
      const model = typeof body?.model === "string" && /^[A-Za-z0-9._:\/-]{1,80}$/.test(body.model) ? body.model : null;
      const dir = writeLook(config.stateDir, await bundle(), { sources: [root] });
      const cmd = askCommand({ app, model, dir, sources: [root] });
      // As one line any shell takes: the words in double quotes.
      const line = [app, ...cmd.args.map((a) => (/^[A-Za-z0-9._:=\/-]+$/.test(a) ? a : `"${a.replace(/"/g, "")}"`))].join(" ");
      return send(res, 200, { app, model, dir, folder: tilde(dir, config.homeDir), command: line });
    }
  }

  // With hosts on, other hosts' maps and live state are in them, except for
  // ?hosts=0, this machine's own, which is what a relay sends.
  const merged = hub && url.searchParams.get("hosts") !== "0";
  if (req.method === "GET" && route === "/api/graph") {
    const graph = state.graph ?? (await state.refresh());
    return send(res, 200, merged ? hub.graph(graph) : graph);
  }

  if (req.method === "GET" && route === "/api/live") return send(res, 200, merged ? hub.live(state.liveSnapshot()) : state.liveSnapshot());

  // Event history (last 24h) for replay and digests.
  if (req.method === "GET" && route === "/api/history") {
    const since = Number(url.searchParams.get("since") ?? Date.now() - 24 * 3600e3);
    return send(res, 200, { events: state.live.historySince(since), now: Date.now() });
  }

  if (req.method === "GET" && route === "/api/search") {
    const q = url.searchParams.get("q") ?? "";
    return send(res, 200, { results: await state.searchTranscripts(q), status: state.search.status });
  }

  if (req.method === "POST" && route === "/api/ack") {
    const { sessionId } = await readJson(req);
    if (!state.index.has(sessionId)) return send(res, 400, { error: "XenoNet doesn't know that session" });
    store.ack(sessionId);
    broadcast("live", state.liveSnapshot());
    return send(res, 200, { ok: true });
  }

  if (req.method === "GET" && route === "/api/collision") {
    const a = url.searchParams.get("a"), b = url.searchParams.get("b");
    return send(res, 200, { a: { id: a, title: state.titleOf(a), branch: state.index.get(a)?.branch ?? null }, b: { id: b, title: state.titleOf(b), branch: state.index.get(b)?.branch ?? null }, files: state.live.collisionDiffs(a, b) });
  }

  if (route === "/api/handoff") {
    if (req.method === "GET") {
      const target = url.searchParams.get("target") === "codex" ? "codex" : "claude";
      try {
        return send(res, 200, { target, ...(await state.handoff(url.searchParams.get("nodeId"), target)) });
      } catch (err) {
        return send(res, 400, { error: String(err.message ?? err) });
      }
    }
    if (req.method === "POST") {
      const { nodeId, target, brief } = await readJson(req);
      const node = state.index.get(nodeId);
      if (!node || node.kind !== "session" || !["claude", "codex"].includes(target) || typeof brief !== "string" || !brief.trim() || brief.length > MAX_BRIEF) return send(res, 400, { error: "bad handoff" });
      if (config.demo) return send(res, 409, { error: "Demo sessions exist only in XenoNet, so they can't be handed off." });
      const url2 = handoffUrl(target, node.worktree || node.cwd, brief);
      await open(url2);
      return send(res, 200, { ok: true, target });
    }
  }

  if (req.method === "GET" && route === "/api/digest") {
    const since = Number(url.searchParams.get("since") ?? Date.now() - 2 * 3600e3);
    return send(res, 200, state.digest(since));
  }
  if (req.method === "GET" && route === "/api/daylog") return send(res, 200, state.dayLog());
  if (req.method === "GET" && route === "/api/constellation") return send(res, 200, state.constellation({ names: url.searchParams.get("names") === "1" }));
  if (req.method === "GET" && route === "/api/constellation/labels") return send(res, 200, state.exportLabels());
  if (req.method === "POST" && route === "/api/narrate") {
    const { kind, since } = await readJson(req);
    try {
      return send(res, 200, { text: await state.narrate(kind === "day" ? "day" : "away", Number(since) || Date.now() - 2 * 3600e3) });
    } catch (err) {
      return send(res, 502, { error: String(err.message ?? err) });
    }
  }

  if (route === "/api/layout") {
    if (req.method === "GET") return send(res, 200, store.getLayout());
    if (req.method === "POST") {
      const { positions } = await readJson(req);
      store.setLayout(positions);
      return send(res, 200, { ok: true });
    }
    if (req.method === "DELETE") {
      store.clearLayout();
      state.refresh().catch(() => {});
      return send(res, 200, { ok: true });
    }
  }

  // The projects folder itself: which it is, checking another, changing it.
  if (route.startsWith("/api/workspace/")) return workspaceRoute(req, res, url);

  // How the projects folder is read: the last scan, your rules, Codex's picks.
  if (route.startsWith("/api/projects")) {
    try {
      if (req.method === "GET" && route === "/api/projects") return send(res, 200, await state.projectDetection());
      if (req.method === "POST" && route === "/api/projects/rule") {
        if (!(await yours(req, res))) return;
        const { path: dir, as } = await readJson(req);
        return send(res, 200, await state.setProjectRule(dir, as ?? null));
      }
      // The rules for the folder in use; with ?all=1, every folder's.
      if (req.method === "DELETE" && route === "/api/projects/rules") return (await yours(req, res)) ? send(res, 200, await state.clearProjectRules({ all: url.searchParams.get("all") === "1" })) : undefined;
      if (req.method === "POST" && route === "/api/projects/rescan") return send(res, 200, await state.rescanProjects());
      // The Codex pass: GET shows exactly what would be sent, POST sends that
      // (it must name the same fingerprint), DELETE forgets the picks. Both
      // of those are yours.
      if (req.method === "GET" && route === "/api/projects/codex") return send(res, 200, { ...(await state.codexOutline()), last: (await state.projectDetection()).codex, demo: config.demo, decider: await decider.status() });
      if (req.method === "POST" && route === "/api/projects/codex") {
        if (!(await yours(req, res))) return;
        const { fingerprint, via } = await readJson(req);
        if (!fingerprint) return send(res, 400, { error: "Look at what will be sent first" });
        return send(res, 200, await state.askCodexAboutProjects(fingerprint, { decider: via === "decider" ? decider : null }));
      }
      if (req.method === "DELETE" && route === "/api/projects/codex") return (await yours(req, res)) ? send(res, 200, await state.forgetCodexPicks()) : undefined;
    } catch (err) {
      return send(res, err.status ?? 400, { error: String(err.message ?? err) });
    }
  }

  // Identities: what was found, the names you gave them, ones you hid, and
  // the sessions and projects you pinned to one (see identities.js).
  if (route.startsWith("/api/identities")) {
    try {
      if (req.method === "GET" && route === "/api/identities") return send(res, 200, state.identityDetection());
      if (req.method === "POST" && ["/api/identities/name", "/api/identities/hidden", "/api/identities/pin"].includes(route) && !(await yours(req, res))) return;
      if (req.method === "POST" && route === "/api/identities/name") {
        const { id, name } = await readJson(req);
        return send(res, 200, await state.nameIdentity(id, name ?? null));
      }
      if (req.method === "POST" && route === "/api/identities/hidden") {
        const { id, hidden } = await readJson(req);
        return send(res, 200, await state.hideIdentity(id, hidden));
      }
      if (req.method === "POST" && route === "/api/identities/pin") {
        const { nodeId, identity } = await readJson(req);
        return send(res, 200, await state.pinIdentity(nodeId, identity ?? null));
      }
      if (req.method === "POST" && route === "/api/identities/rescan") return send(res, 200, await state.rescanIdentities());
    } catch (err) {
      return send(res, err.status ?? 400, { error: String(err.message ?? err) });
    }
  }

  // With hosts on, live hosts' alerts count in the tray and the menu bar too.
  if (req.method === "GET" && route === "/api/summary") return send(res, 200, hub ? hub.summary(state.summary()) : state.summary());

  if (req.method === "GET" && route === "/api/events") {
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store", connection: "keep-alive" });
    res.write(`retry: 3000\n\n`);
    res.ownOnly = !!hub && url.searchParams.get("hosts") === "0";
    clients.add(res);
    const ping = setInterval(() => res.write(`: ping\n\n`), 25000);
    req.on("close", () => {
      clearInterval(ping);
      clients.delete(res);
    });
    return;
  }

  // Sessions by id or title words, newest first, for the xenonet command; with
  // cwd, only those that ran in that folder (or one inside it).
  if (req.method === "GET" && route === "/api/sessions") {
    const q = (url.searchParams.get("q") ?? "").trim().toLowerCase();
    const words = q.split(/\s+/).filter(Boolean);
    const cwd = url.searchParams.get("cwd");
    const under = (dir) => !!dir && (dir === cwd || dir.startsWith(cwd + path.sep));
    const found = [...state.index.values()]
      .filter((n) => n.kind === "session")
      .filter((n) => !q || n.id.toLowerCase().startsWith(q) || n.rawId?.toLowerCase().startsWith(q) || words.every((w) => `${n.title ?? ""} ${n.cwd ?? ""}`.toLowerCase().includes(w)))
      .filter((n) => !cwd || under(n.cwd) || under(n.worktree))
      .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0))
      .slice(0, Math.min(Number(url.searchParams.get("limit")) || 20, 200));
    return send(res, 200, found.map((n) => ({ id: n.id, app: n.app, title: n.title, cwd: n.cwd ?? null, branch: n.branch ?? null, updatedAt: n.updatedAt ?? null, activity: n.activity ?? null })));
  }

  // A session's models, tokens, costs and timing (core/stats.js).
  const stats = /^\/api\/session\/([^/]+)\/stats$/.exec(route);
  if (req.method === "GET" && stats) {
    const node = state.index.get(decodeURIComponent(stats[1]));
    if (!node || node.kind !== "session") return send(res, 404, { error: "XenoNet doesn't know that session" });
    const result = await state.sessionStats(node.id).catch((err) => ({ error: String(err?.message ?? err) }));
    if (!result) return send(res, 404, { error: "XenoNet can't find this session's transcript" });
    if (result.error) return send(res, 502, result);
    return send(res, 200, { ...result, title: node.title, cwd: node.cwd ?? null, branch: node.branch ?? null, model: node.model ?? null });
  }

  const transcript = /^\/api\/session\/([^/]+)\/transcript$/.exec(route);
  if (req.method === "GET" && transcript) {
    const node = state.index.get(decodeURIComponent(transcript[1]));
    if (!node || node.kind !== "session") return send(res, 404, { error: "XenoNet doesn't know that session" });
    const opts = { cursor: url.searchParams.get("cursor"), limit: Number(url.searchParams.get("limit") ?? 120) };
    try {
      const page = await state.readTranscript(node, opts);
      return send(res, 200, page);
    } catch (err) {
      return send(res, 502, { error: String(err?.message ?? err) });
    }
  }

  // A session's new name, saved in its own app (state.renameSession). It
  // changes another app's files, so it's yours to do.
  if (req.method === "POST" && route === "/api/session/rename") {
    if (!(await yours(req, res))) return;
    const { nodeId, title } = await readJson(req);
    try {
      return send(res, 200, await state.renameSession(nodeId, title));
    } catch (err) {
      return send(res, err.status ?? 502, { error: String(err.message ?? err) });
    }
  }

  if (req.method === "POST" && route === "/api/open") {
    const { nodeId, action } = await readJson(req);
    if (action === "needs-input") {
      await open(NEEDS_INPUT_URL);
      return send(res, 200, { ok: true, target: NEEDS_INPUT_URL });
    }
    const node = state.index.get(nodeId);
    if (!node?.actions?.some((a) => a.id === action && !a.client)) return send(res, 400, { error: "that action isn't available here" });
    const target = urlFor(node, action);
    if (!target) return send(res, 400, { error: "there's nothing to open for this item" });
    if (config.demo && node.kind === "session" && /^(claude|codex):/.test(target)) return send(res, 409, { error: "Demo sessions exist only in XenoNet, so they can't open in an app." });
    await open(target);
    return send(res, 200, { ok: true, target });
  }

  // Optional Claude Code HTTP hooks (see README). Transcript tailing already
  // covers tool calls; hooks add events transcripts don't record, like
  // permission prompts and notifications.
  if (req.method === "POST" && route === "/api/hook/claude") {
    const body = await readJson(req).catch(() => ({}));
    if (typeof body.session_id === "string" && /^[0-9a-f-]{36}$/i.test(body.session_id)) {
      const ok = await verify.check(req.socket, `claude:${body.session_id}`, { hook: true });
      if (!ok.ok) return send(res, 403, { error: ok.reason });
      state.live.noteHook(`claude:${body.session_id}`, body);
      if (body.hook_event_name === "SessionStart") state.refresh().catch(() => {});
      // What the engine wants the agent to hear: who else is here, an edit
      // warning, messages.
      const out = await engine.hook("claude", body.hook_event_name, body).catch(() => null);
      if (out) return send(res, 200, out);
    }
    res.writeHead(204).end();
    return;
  }

  // A session's Stop hook that listens after its turns (xenonet doctor
  // --live, core/delivery/setup.js): it waits for a message up to the
  // window the session started with, and hands it over to keep it going.
  // If the session stops waiting first, the message stays unread for its
  // next turn.
  if (req.method === "POST" && route === "/api/hook/claude/listen") {
    const body = await readJson(req).catch(() => ({}));
    if (body.hook_event_name === "Stop" && typeof body.session_id === "string" && /^[0-9a-f-]{36}$/i.test(body.session_id)) {
      const me = `claude:${body.session_id}`;
      const ok = await verify.check(req.socket, me, { hook: true });
      if (!ok.ok) return send(res, 403, { error: ok.reason });
      const seconds = Math.min(Math.max(Number(url.searchParams.get("seconds")) || 300, 5), 3600);
      const gone = new AbortController();
      res.on("close", () => res.writableEnded || gone.abort());
      const out = await engine.listen(me, { ms: seconds * 1000, signal: gone.signal }).catch(() => null);
      if (out && (gone.signal.aborted || res.destroyed)) engine.store.markUnseen(me, out.ids);
      else if (out) return send(res, 200, { decision: out.decision, reason: out.reason });
    }
    if (!res.destroyed) res.writeHead(204).end();
    return;
  }

  // Dev only: rare states to check the visuals by (see simulate).
  if (dev && req.method === "POST" && route === "/api/dev/simulate") {
    const { kind, ...named } = await readJson(req);
    const done = simulate(kind, named);
    if (done?.error) return send(res, 400, { error: done.error });
    return done ? send(res, 200, done) : send(res, 400, { error: "unknown simulation or session" });
  }

  // XenoNet Dev's control channel (see `relay` above). JSON only, which a
  // cross-site form can't send.
  if (req.method === "POST" && route === "/api/dev/control") {
    if (!controlled) return send(res, 503, { error: "this server has no window to control: it isn't running inside XenoNet Dev (npm run app, or npm run drive -- launch)" });
    if (!/^application\/json\b/i.test(req.headers["content-type"] ?? "")) return send(res, 415, { error: "send JSON (content-type: application/json)" });
    const reply = await relay(await readJson(req, 1 << 16));
    // Always JSON: send() passes strings through as they are, and a result can
    // be one.
    return reply.error ? send(res, reply.late ? 504 : 400, { error: reply.error }) : send(res, 200, JSON.stringify(reply.result ?? null));
  }

  // Dev only: save a frame of the WebGL canvas for visual checks
  // (window.__xenonet.capture() in the browser console).
  if (dev && req.method === "POST" && route === "/api/dev/capture") {
    const { dataUrl } = await readJson(req, 64 << 20);
    const m = /^data:image\/png;base64,(.+)$/.exec(dataUrl ?? "");
    if (!m) return send(res, 400, { error: "expected a PNG data URL" });
    const dir = path.join(root, ".xenonet");
    fs.mkdirSync(dir, { recursive: true });
    const name = String(url.searchParams.get("name") ?? "capture").replace(/[^a-z0-9-]/gi, "") || "capture";
    const file = path.join(dir, `${name}.png`);
    fs.writeFileSync(file, Buffer.from(m[1], "base64"));
    return send(res, 200, { ok: true, file });
  }

  return send(res, 404, { error: "not found" });
}

function serveStatic(req, res, url) {
  const found = staticFile(path.join(root, "dist"), url.pathname);
  if (found.status === 503) return send(res, 503, found.error, { "content-type": "text/plain" });
  if (found.status) return send(res, found.status, { error: found.error });
  res.writeHead(200, { "content-type": found.type, ...found.headers });
  fs.createReadStream(found.file).pipe(res);
}

const server = http.createServer();
let vite = null;

server.on("request", async (req, res) => {
  // Transcripts are private: refuse DNS-rebinding hosts and cross-site posts.
  if (!ALLOWED_HOSTS.has(req.headers.host ?? "")) return send(res, 421, { error: "bad host" });
  if (req.method !== "GET" && req.headers.origin && !ALLOWED_ORIGINS.has(req.headers.origin)) return send(res, 403, { error: "bad origin" });
  const url = new URL(req.url, `http://${req.headers.host}`);
  try {
    if (upstream?.handles(url.pathname)) return upstream.pass(req, res, url);
    if (url.pathname.startsWith("/api/")) return await api(req, res, url);
    if (vite) return vite.middlewares(req, res);
    return serveStatic(req, res, url);
  } catch (err) {
    if (err.status && !res.headersSent) return send(res, err.status, { error: err.message });
    console.error(err);
    log.error("service.route", `${req.method} ${url.pathname.replace(/[^/]{24,}/g, "…")}: ${err.message}`, { err });
    if (!res.headersSent) send(res, 500, { error: "internal error" });
  }
});

if (dev) {
  const { createServer } = await import("vite");
  // XENONET_HOT=0 (XenoNet Dev's test instance) keeps the page as it loaded,
  // whatever is edited meanwhile.
  vite = await createServer({ root, server: { middlewareMode: true, ws: { server }, ...(process.env.XENONET_HOT === "0" ? { hmr: false, watch: null } : {}) }, appType: "spa" });
}

server.on("error", async (err) => {
  // Who holds the port, when it's another XenoNet: running XenoNet where it's
  // developed, the installed one and XenoNet Dev take each other's ports.
  let holder = "";
  if (err.code === "EADDRINUSE") {
    const h = await fetch(`http://127.0.0.1:${config.port}/api/health`, { signal: AbortSignal.timeout(1500) }).then((r) => (r.ok ? r.json() : null)).catch(() => null);
    holder = h ? ` by ${h.name ?? "another XenoNet"}${h.instance ? ` (${h.instance})` : ""}${h.pid ? `, pid ${h.pid}` : ""}` : " by a program that isn't XenoNet";
  }
  const words = err.code === "EADDRINUSE" ? `port ${config.port} is already in use${holder}` : err.message;
  console.error(`[xenonet] ${words}`);
  log.error(err.code === "EADDRINUSE" ? "service.port-in-use" : "service.listen", words, { err, fix: err.code === "EADDRINUSE" ? "Quit the other XenoNet, or start this one with another XENONET_PORT." : null });
  process.parentPort?.postMessage({ type: "error", code: err.code, message: words });
  process.exit(1);
});

server.listen(config.port, config.host, async () => {
  if (process.env.XENONET_STANDALONE === "1") {
    // No app started this server, so the Desktop that connects is the one
    // the window's requests come from (verify.desktop).
    const onDesktop = async (socket) => {
      const seen = await verify.desktop(socket);
      if (socket && !seen.ok) console.warn(`[xenonet] the window's own routes stay closed: ${seen.reason}`);
      return seen;
    };
    try { components = await componentService({ version: VERSION, windowKey, windowState: (value) => actors?.you.window(value), shutdown, onDesktop }); }
    catch (err) { console.error(err); process.exit(1); }
  }
  console.log(`[xenonet] http://${config.host}:${config.port}${dev ? "  (dev)" : ""}`);
  console.log(`[xenonet] projects: ${config.projectsDir}`);
  if (config.demo) console.log("[xenonet] demo mode: made-up projects and sessions, nothing real is read");
  if (upstream) console.log(`[xenonet] the page reads ${upstream.url.host}, and changes nothing there`);
  else if (process.env.XENONET_UPSTREAM) console.log("[xenonet] XENONET_UPSTREAM is for XenoNet Dev (--dev) only; this XenoNet reads its own");
  // Running inside the desktop app's utility process: hand it the window
  // key, over this port only, and tell it we're up.
  process.parentPort?.postMessage({ type: "window-key", key: windowKey.reveal() });
  process.parentPort?.postMessage({ type: "ready", port: config.port });
  doctor.start();
  actors?.start();
  observers?.start();
  hub?.start();
  assistant?.sync();
  gatewayControl?.init().then(() => gatewayControl.gateway.status().listening && console.log(`[xenonet] the actor gateway listens on 127.0.0.1:${gatewayControl.gateway.port}`));
  tunnels?.autostart().catch(() => {});
  pollSpotify();
  state.start().catch((e) => {
    console.error("[xenonet] start failed:", e);
    log.error("service.start", `reading sessions failed to start: ${e.message}`, { err: e });
  });
});

const shutdown = () => {
  clearTimeout(music.timer);
  system.stop();
  tunnels?.stopAll();
  assistant?.stop();
  gatewayControl?.stop();
  wakes?.stop();
  components?.close();
  hub?.stop();
  observers?.stop();
  actors?.stop();
  state.stop();
  codexClient.stop();
  drift.flush();
  vite?.close();
  server.close();
  process.exit(0);
};
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
