// The actor gateway's switch, its tunnel and how it stands: whether it's
// on, which tunnel puts it on the internet (server/gateway-tunnel.js; none
// is assumed), the address that gives it, whether that address reaches it
// (server/gateway-reach.js), and how each actor's connection is doing.
// Kept in the engine's own database, so all of it comes back as it was
// when XenoNet starts again, after a reboot say. Changing it is the owner's
// (the routes in server/routes/actors.js check that).
//
// While it's on, XenoNet checks the address every ten minutes (every two
// while it fails, and at once after the computer wakes), and sets the
// tunnel up again when it went: Funnel's setting removed, a child tunnel
// that gave up. The address can change under it (a quick tunnel's does at
// every start, a Tailscale computer can be renamed, the owner can pick
// another tunnel): each actor's last call says which address it used, so
// the window can say which apps still have an old one. A token doesn't
// depend on the address, so the apps keep theirs.
import crypto from "node:crypto";
import { createActorGateway, normalHost } from "./actor-gateway.js";
import { createTunnel, KINDS, KIND_IDS, TUNNEL_NAME } from "./gateway-tunnel.js";
import { checkReach } from "./gateway-reach.js";
import { TOKEN_UNUSED_MS } from "../core/actors/registration.js";

const KEY = "gateway";
const SEEN = "gateway.seen";
const TICK_MS = 60e3;
const OK_EVERY_MS = 10 * 60e3;
const FAILING_EVERY_MS = 2 * 60e3;
// An actor's last call is written down when its address changes, or at
// most once an hour.
const SEEN_SAVE_MS = 3600e3;
// A gateway token that goes unused this much longer stops working: the
// window says so first.
export const EXPIRING_MS = 7 * 24 * 3600e3;

// The gateway's port when nothing names one: 1010 above the service's, so
// 48850 for an installed XenoNet on 47840. That's clear of every instance's
// port (47840 to 47845), their pages' (1000 above theirs) and Xenolith's
// (47850 for its HTTP server, 47859 for its preview), and below the ports
// macOS and Windows hand out for outgoing connections.
export const GATEWAY_OFFSET = 1010;
export const gatewayPortOf = (port) => (port + GATEWAY_OFFSET <= 65535 ? port + GATEWAY_OFFSET : port - GATEWAY_OFFSET);

// The default before that: 10 above the service's (47850, Xenolith's port,
// for an installed XenoNet). A gateway set up then keeps it.
export const OLD_GATEWAY_OFFSET = 10;
export const oldGatewayPortOf = (port) => (port + OLD_GATEWAY_OFFSET <= 65535 ? port + OLD_GATEWAY_OFFSET : gatewayPortOf(port));

const validPort = (p) => (Number.isInteger(p) && p >= 1 && p <= 65535 ? p : null);

const bad = (message) => {
  throw Object.assign(new Error(message), { status: 400, code: "bad-request" });
};
const parse = (s, fallback) => {
  try {
    return JSON.parse(s ?? "") ?? fallback;
  } catch {
    return fallback;
  }
};

// `port`: the gateway's port when one is named (XENONET_GATEWAY_PORT);
// otherwise it's the one its saved setting has, the old default for one set
// up before ports were saved, else the default for `servicePort`. `probe`: "local" checks the loop on this computer alone,
// without the internet (XENONET_GATEWAY_PROBE=local, for tests). `dir`: the
// state folder, for the pid of a tunnel XenoNet runs. The rest stand in for
// tests.
export function createGatewayControl({ engine, registration, actors, runs = null, onRun = () => {}, store, port = null, servicePort = 47840, version, dir = null, probe = null, log = () => {}, now = Date.now, tunnelOptions = {}, reachCheck = checkReach, setTimer = setTimeout, clearTimer = clearTimeout, every = setInterval, stopEvery = clearInterval }) {
  // { on, host, tunnel: { kind, name, host }, port }: host is the address in
  // use, the one the owner gave or the one the tunnel got; tunnel.host is
  // the one the owner gave, for the kinds that ask; port is the one the
  // gateway had when it was set up. A setting from before tunnels were
  // picked here, a host alone, is the owner's own tunnel.
  const saved = () => {
    const s = parse(store.meta(KEY), {});
    const host = normalHost(s.host);
    const kind = KINDS[s.tunnel?.kind] ? s.tunnel.kind : host ? "own" : null;
    const tunnel = kind ? { kind, name: TUNNEL_NAME.test(s.tunnel?.name ?? "") ? s.tunnel.name : null, host: normalHost(s.tunnel?.host) ?? (kind === "own" ? host : null) } : null;
    return { on: s.on === true, host, tunnel, port: validPort(s.port) };
  };
  // A gateway once set up keeps its port, so the tunnels and apps pointed at
  // it keep working when the default moves or XenoNet's own port does. One
  // set up before ports were saved (on, with a tunnel or a public host) was
  // on the old default: it keeps that, saved now so it stays put. Only one
  // never set up takes the default.
  const before = saved();
  const setUp = before.on || !!before.tunnel || !!before.host;
  const keepOld = port == null && !before.port && setUp;
  port ??= before.port ?? (setUp ? oldGatewayPortOf(servicePort) : gatewayPortOf(servicePort));
  const save = (s) => store.setMeta(KEY, JSON.stringify({ ...s, port }));
  if (keepOld) save(before);

  // Each actor's last call through the gateway: { host, at } by slug.
  const seen = new Map(Object.entries(parse(store.meta(SEEN), {})));
  const refused = new Map(); // actor id → { code, reason, at }
  function called({ slug, id, host, at }) {
    refused.delete(id);
    const had = seen.get(slug);
    const next = { host: host ?? had?.host ?? null, at, savedAt: had?.savedAt ?? 0 };
    seen.set(slug, next);
    if (!had || had.host !== next.host || at - next.savedAt >= SEEN_SAVE_MS) {
      next.savedAt = at;
      store.setMeta(SEEN, JSON.stringify(Object.fromEntries(seen)));
    }
  }

  const gateway = createActorGateway({
    engine,
    registration,
    actors,
    runs,
    onRun,
    port,
    host: saved().host,
    version,
    log,
    onCall: called,
    onRefused: ({ actor, code, reason, at }) => actor && refused.set(actor, { code, reason, at }),
  });

  // The address a tunnel got (Tailscale's name, a quick tunnel's, ngrok's).
  function learned(name) {
    const h = normalHost(name);
    const s = saved();
    if (!h || s.host === h) return;
    s.host = h;
    save(s);
    gateway.setHost(h);
    log("gateway.address", `the gateway's address is now ${h}`);
    soon(3000);
  }
  const tunnel = createTunnel({ port, dir, log, onHost: learned, onReady: () => soon(1500), ...tunnelOptions });

  let problem = null;
  let reach = { state: "unknown", stage: null, why: null, at: null, okAt: null };
  let checking = null;
  let soonTimer = null;
  let ticker = null;
  let lastTick = now();

  async function check() {
    if (checking) return checking;
    checking = (async () => {
      const s = saved();
      if (!s.on || !gateway.status().listening) {
        reach = { ...reach, state: "off", stage: null, why: null };
        return;
      }
      const before = reach.state;
      reach = { ...reach, state: "checking" };
      const r = await reachCheck({ host: s.host, nonce: crypto.randomBytes(16).toString("hex"), expect: (w) => gateway.expectProbe(w), local: probe === "local" ? port : null }).catch((err) => ({ ok: false, stage: "error", why: err.message }));
      const t = now();
      reach = { state: r.ok ? "ok" : "failing", stage: r.stage, why: r.why, at: t, okAt: r.ok ? t : reach.okAt, local: probe === "local" };
      if (before !== reach.state) log("gateway.reach", r.ok ? `the gateway is reachable at ${s.host}` : r.why);
    })().finally(() => {
      checking = null;
    });
    return checking;
  }
  function soon(ms) {
    clearTimer(soonTimer);
    soonTimer = setTimer(() => {
      soonTimer = null;
      check().catch(() => {});
    }, ms);
    soonTimer?.unref?.();
  }

  // Every minute: a check when one is due, or at once after the computer
  // slept (the clock jumped), with the tunnel set up again first.
  async function tick() {
    const t = now();
    const woke = t - lastTick > 3 * TICK_MS;
    lastTick = t;
    const s = saved();
    if (!s.on || checking) return;
    const due = woke || !reach.at || t - reach.at >= (reach.state === "ok" ? OK_EVERY_MS : FAILING_EVERY_MS);
    if (!due) return;
    // Its port was taken: try it again, and the tunnel with it.
    if (!gateway.status().listening) {
      reach = { ...reach, at: t };
      return apply(s);
    }
    if (!s.tunnel) return;
    await tunnel.ensure().catch((err) => log("gateway.tunnel", err.message));
    await check();
  }

  async function apply(s, { takeOver = false } = {}) {
    gateway.setHost(s.host);
    const had = problem;
    problem = null;
    try {
      if (s.on) await gateway.start();
      else await gateway.stop();
    } catch (err) {
      problem = err.code === "EADDRINUSE" ? `port ${port} is in use by another program` : String(err?.message ?? err);
      if (problem !== had) log("gateway.error", problem);
    }
    // Its port taken (another XenoNet's gateway, say): a tunnel XenoNet runs
    // stops, but Funnel's setting, which may be that one's, stays.
    if (problem) {
      tunnel.stop();
      reach = { ...reach, state: "failing", stage: "port", why: problem, at: now() };
      return;
    }
    if (reach.stage === "port") reach = { ...reach, state: "unknown", stage: null, why: null };
    if (!s.on || !s.tunnel) {
      await tunnel.down().catch((err) => log("gateway.tunnel", err.message));
      reach = { ...reach, state: s.on ? "failing" : "off", stage: s.on ? "address" : null, why: null };
      return;
    }
    const { kind, name, host } = s.tunnel;
    await tunnel.up({ kind, name, host }, { takeOver }).catch((err) => log("gateway.tunnel", err.message));
    soon(["own", "tailscale"].includes(kind) ? 1500 : 10e3);
  }

  // The actors with a gateway token, or refused one lately, each with how
  // its connection stands.
  function actorsView(host) {
    const t = now();
    const out = [];
    const ids = new Set();
    const row = (id, extra) => {
      const info = actors.info(id);
      const sn = seen.get(id.replace(/^actor:/, ""));
      return { id, name: info?.name ?? id.replace(/^actor:/, ""), product: info?.product?.id ?? null, lastAt: sn?.at ?? null, lastHost: sn?.host ?? null, oldAddress: !!(host && sn?.host && sn.host !== host), refused: refused.get(id) ?? null, ...extra };
    };
    for (const r of store.actorTokenTimes()) {
      if (!r.hasGateway) continue;
      const id = `actor:${r.slug}`;
      ids.add(id);
      const expiresAt = r.gatewayAt != null ? r.gatewayAt + TOKEN_UNUSED_MS : null;
      out.push(row(id, { token: true, expiresAt, expiring: expiresAt != null && expiresAt - t < EXPIRING_MS }));
    }
    for (const id of refused.keys()) if (!ids.has(id)) out.push(row(id, { token: false, expiresAt: null, expiring: false }));
    return out;
  }

  return {
    gateway,
    tunnel,
    // At start, as it was left, with the clock started.
    async init() {
      await apply(saved());
      ticker = every(() => tick().catch(() => {}), TICK_MS);
      ticker?.unref?.();
    },
    async stop() {
      stopEvery(ticker);
      clearTimer(soonTimer);
      tunnel.stop();
      await gateway.stop();
    },
    // { on, tunnel: { kind, name, host }, check, repair }: any of them.
    // Another tunnel stops the one before. repair sets the tunnel up again,
    // letting Funnel replace what it forwarded to, and checks at once.
    async set({ on, tunnel: pick, check: checkNow = false, repair = false } = {}) {
      const s = saved();
      if (pick !== undefined) {
        if (pick === null) s.tunnel = null;
        else {
          const kind = String(pick?.kind ?? "");
          const k = KINDS[kind];
          if (!k) bad(`a tunnel is one of ${KIND_IDS.join(", ")}`);
          const name = pick.name == null || pick.name === "" ? null : String(pick.name).trim();
          if (k.asks.name && name && !TUNNEL_NAME.test(name)) bad("a tunnel's name is letters, digits, dots, dashes and underscores, as cloudflared tunnel list shows it");
          if (k.asks.name === "required" && !name) bad("say the tunnel's name, as cloudflared tunnel list shows it");
          const given = pick.host == null || pick.host === "" ? null : normalHost(pick.host);
          if (k.asks.host && pick.host && !given) bad("that isn't a host name, such as xenonet.example.com");
          if (k.asks.host === "required" && !given) bad("say the address it answers for, such as xenonet.example.com");
          const before = s.tunnel;
          s.tunnel = { kind, name: k.asks.name ? name : null, host: k.asks.host ? given : null };
          // The address: the one given, else the one this kind got before,
          // else none until the tunnel says.
          if (s.tunnel.host) s.host = s.tunnel.host;
          else if (before?.kind !== kind || before?.host) s.host = null;
        }
      }
      if (on !== undefined) s.on = on === true;
      save(s);
      await apply(s, { takeOver: repair === true });
      if (repair || checkNow) await check();
      return this.status();
    },
    check: async () => {
      await check();
      return reach;
    },
    async status() {
      const s = saved();
      const g = gateway.status();
      const tv = tunnel.view();
      const kind = s.tunnel?.kind ?? null;
      const why =
        problem ??
        (s.on && !s.tunnel ? "pick how the gateway reaches the internet" : null) ??
        (s.on && tv.state === "failed" ? tv.why : null) ??
        (s.on && reach.state === "failing" ? reach.why : null);
      return {
        on: s.on,
        ...g,
        tunnel: s.tunnel ? { ...tv, kind, label: KINDS[kind].label, through: KINDS[kind].through, name: s.tunnel.name, given: s.tunnel.host, stable: KINDS[kind].stable } : { ...tv, kind: null, label: null, through: null, name: null, given: null, stable: null },
        reach,
        problem: why,
        kinds: await tunnel.kinds(),
        actors: actorsView(s.host),
      };
    },
    url: () => gateway.status().url,
  };
}
