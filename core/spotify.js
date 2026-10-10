// Spotify, as a remote for the Spotify app you already use
// (docs/guide/spotify.md): what's playing, play and pause, the next track,
// and your volume dipped while something waits on you. XenoNet plays no
// audio of Spotify's itself and never mixes it with its own music: while
// Spotify plays, XenoNet's generative music steps aside (src/ambient.js).
//
// You connect with an app of your own on Spotify's developer dashboard
// (its client id isn't a secret), through Authorization Code with PKCE:
// XenoNet opens Spotify's sign-in in your browser, which comes back to this
// XenoNet at http://127.0.0.1:<port>/api/spotify/callback, once, within ten
// minutes, with the state it was sent with. The refresh token goes to your
// credential store (core/secret-store.js); the access token stays in this
// service's memory and never reaches the page. Spotify's player needs
// Premium.
//
// What reaches Spotify: sign-in and token refreshes, reads of what's
// playing, and the commands you give or turn on. Nothing of your sessions.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const SCOPES = ["user-read-playback-state", "user-modify-playback-state", "user-read-currently-playing"];
export const REFRESH_TARGET = "XenoNet/Spotify/refresh";
const ACCOUNTS = "https://accounts.spotify.com";
const API = "https://api.spotify.com/v1";
const PENDING_MS = 10 * 60e3;
const EARLY_MS = 90e3;
export const CLIENT_ID = /^[0-9a-f]{32}$/i;

const b64url = (buf) => buf.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
export const challengeOf = (verifier) => b64url(crypto.createHash("sha256").update(verifier).digest());

export class SpotifyError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}
const fail = (status, code, message) => {
  throw new SpotifyError(status, code, message);
};

// What's playing, as the page shows it: never more than this.
export function nowPlaying(p) {
  if (!p || !p.item) return { playing: false, title: null, artists: null, url: null, device: p?.device ? { id: p.device.id, name: p.device.name, volume: p.device.volume_percent ?? null } : null };
  const item = p.item;
  // A track's artists, or a podcast episode's show.
  const names = item.artists?.length ? item.artists.map((a) => a?.name) : [item.show?.name];
  return {
    playing: !!p.is_playing,
    title: String(item.name ?? "").slice(0, 200),
    artists: names.filter(Boolean).join(", ").slice(0, 200) || null,
    url: typeof item.external_urls?.spotify === "string" && /^https:\/\/open\.spotify\.com\//.test(item.external_urls.spotify) ? item.external_urls.spotify : null,
    device: p.device ? { id: p.device.id, name: p.device.name, volume: p.device.volume_percent ?? null } : null,
  };
}

// `file`: <state>/spotify.json (your client id, which device is this
// computer, what's turned on). `secrets`: the credential store.
export function createSpotify({ file, secrets, port, fetch = globalThis.fetch, now = Date.now, log = null }) {
  let settings = read();
  let pending = null; // { state, verifier, at }
  let access = null; // { token, until }
  let retryAt = 0;
  const redirect = () => `http://127.0.0.1:${port}/api/spotify/callback`;

  function read() {
    try {
      const s = JSON.parse(fs.readFileSync(file, "utf8"));
      return { clientId: CLIENT_ID.test(s.clientId ?? "") ? s.clientId : null, device: typeof s.device === "string" ? s.device : null, duck: s.duck === true };
    } catch {
      return { clientId: null, device: null, duck: false };
    }
  }
  function save() {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, `${JSON.stringify(settings)}\n`, { mode: 0o600 });
  }

  async function tokenRequest(form) {
    const res = await fetch(`${ACCOUNTS}/api/token`, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams(form).toString(), signal: AbortSignal.timeout(15000) }).catch((err) => fail(502, "unreachable", `couldn't reach Spotify: ${err.message}`));
    const body = await res.json().catch(() => ({}));
    if (!res.ok) fail(res.status === 400 || res.status === 401 ? 409 : 502, body.error === "invalid_grant" ? "reconnect" : "refused", `Spotify said: ${String(body.error_description ?? body.error ?? res.status).slice(0, 160)}`);
    return body;
  }

  async function keep(body) {
    access = { token: body.access_token, until: now() + Number(body.expires_in ?? 3600) * 1000 };
    // A refresh may come back without a new refresh token: keep the one there is.
    if (body.refresh_token) await secrets.set(REFRESH_TARGET, body.refresh_token, { comment: "Spotify refresh token, kept by XenoNet" });
  }

  // An access token, refreshed a little before it runs out. One Spotify
  // won't refresh any more means connecting again.
  async function token() {
    if (access && now() < access.until - EARLY_MS) return access.token;
    if (!settings.clientId) fail(409, "no-app", "XenoNet has no Spotify app to connect with yet");
    const refresh = await secrets.get(REFRESH_TARGET).catch(() => null);
    if (!refresh) fail(409, "not-connected", "Spotify isn't connected");
    try {
      await keep(await tokenRequest({ grant_type: "refresh_token", refresh_token: refresh, client_id: settings.clientId }));
    } catch (err) {
      if (err.code === "reconnect") {
        await secrets.remove(REFRESH_TARGET).catch(() => {});
        access = null;
      }
      throw err;
    }
    return access.token;
  }

  // A call to Spotify's Web API: the token in its header, a 429's wait
  // kept to before the next call, one 401 refreshed and tried again.
  async function call(method, route, { retry = true } = {}) {
    if (now() < retryAt) fail(429, "busy", "Spotify asked XenoNet to wait a moment");
    const res = await fetch(`${API}${route}`, { method, headers: { authorization: `Bearer ${await token()}` }, signal: AbortSignal.timeout(10000) }).catch((err) => fail(502, "unreachable", `couldn't reach Spotify: ${err.message}`));
    if (res.status === 401 && retry) {
      access = null;
      return call(method, route, { retry: false });
    }
    if (res.status === 429) {
      retryAt = now() + Math.min(30, Number(res.headers.get?.("retry-after")) || 5) * 1000;
      fail(429, "busy", "Spotify asked XenoNet to wait a moment");
    }
    if (res.status === 204) return null;
    const body = await res.json().catch(() => null);
    if (!res.ok) fail(res.status === 403 ? 409 : 502, res.status === 403 ? "premium" : "refused", res.status === 403 ? "Spotify's player needs Premium" : `Spotify said: ${String(body?.error?.message ?? res.status).slice(0, 160)}`);
    return body;
  }

  return {
    settings: () => ({ ...settings }),
    async status() {
      const connected = settings.clientId ? await secrets.has(REFRESH_TARGET).catch(() => false) : false;
      return { app: !!settings.clientId, connected, device: settings.device, duck: settings.duck, redirect: `http://127.0.0.1/api/spotify/callback` };
    },
    setApp(clientId) {
      if (!CLIENT_ID.test(String(clientId ?? "").trim())) fail(400, "bad-request", "a Spotify app's client id is 32 letters and digits, from its page on Spotify's developer dashboard");
      settings = { ...settings, clientId: String(clientId).trim() };
      save();
      return { app: true };
    },
    set({ device, duck } = {}) {
      if (device !== undefined) settings.device = typeof device === "string" && device ? device.slice(0, 120) : null;
      if (duck !== undefined) settings.duck = duck === true;
      save();
      return { device: settings.device, duck: settings.duck };
    },
    // Where to send you to sign in: Spotify's page, with a challenge and a
    // state that the callback must bring back.
    begin() {
      if (!settings.clientId) fail(409, "no-app", "give XenoNet your Spotify app's client id first");
      const verifier = b64url(crypto.randomBytes(48));
      const state = b64url(crypto.randomBytes(16));
      pending = { state, verifier, at: now() };
      const q = new URLSearchParams({ client_id: settings.clientId, response_type: "code", redirect_uri: redirect(), code_challenge_method: "S256", code_challenge: challengeOf(verifier), state, scope: SCOPES.join(" ") });
      return { url: `${ACCOUNTS}/authorize?${q}` };
    },
    // Spotify sending you back: once, in time, with the same state.
    async callback({ code, state, error }) {
      const p = pending;
      pending = null;
      if (!p || !state || state !== p.state) fail(400, "bad-state", "this sign-in isn't the one XenoNet started; start again from XenoNet");
      if (now() - p.at > PENDING_MS) fail(400, "expired", "this sign-in took too long; start again from XenoNet");
      if (error) fail(400, "declined", `Spotify said: ${String(error).slice(0, 80)}`);
      if (!code) fail(400, "bad-request", "Spotify sent no code");
      await keep(await tokenRequest({ grant_type: "authorization_code", code, redirect_uri: redirect(), client_id: settings.clientId, code_verifier: p.verifier }));
      log?.info("spotify.connected", "Spotify is connected");
      return { connected: true };
    },
    async disconnect() {
      await secrets.remove(REFRESH_TARGET).catch(() => {});
      access = null;
      return { connected: false };
    },
    async player() {
      return nowPlaying(await call("GET", "/me/player"));
    },
    // play, pause, next, previous, or volume (0 to 100).
    async command(op, value = null) {
      const device = settings.device ? `?device_id=${encodeURIComponent(settings.device)}` : "";
      if (op === "play") return call("PUT", `/me/player/play${device}`);
      if (op === "pause") return call("PUT", `/me/player/pause${device}`);
      if (op === "next") return call("POST", `/me/player/next${device}`);
      if (op === "previous") return call("POST", `/me/player/previous${device}`);
      if (op === "volume") {
        const v = Math.round(Number(value));
        if (!(v >= 0 && v <= 100)) fail(400, "bad-request", "volume is 0 to 100");
        return call("PUT", `/me/player/volume?volume_percent=${v}${device ? `&${device.slice(1)}` : ""}`);
      }
      fail(400, "bad-request", "play, pause, next, previous or volume");
    },
  };
}

// While something waits on you (a question, a collision), the volume
// dips, on this computer's device only, and comes back after, unless you
// changed it meanwhile. Pure: from what's waiting and the player, what to
// do. `held` is the dip under way: { from, to } or null.
export function duckPlan({ waiting, player, device, held, dip = 0.35 }) {
  const here = player?.device && device && player.device.id === device;
  // Not playing here: nothing to dip, and a dip under way is let go when
  // nothing waits any more.
  if (!here || !player.playing || player.device.volume == null) return { op: null, held: waiting ? held : null };
  if (waiting && !held) {
    const to = Math.max(5, Math.round(player.device.volume * dip));
    return to < player.device.volume ? { op: "volume", volume: to, held: { from: player.device.volume, to } } : { op: null, held };
  }
  if (!waiting && held) return player.device.volume === held.to ? { op: "volume", volume: held.from, held: null } : { op: null, held: null };
  // You changed it while it was dipped: yours stands.
  if (waiting && held && player.device.volume !== held.to) return { op: null, held: null };
  return { op: null, held };
}

// The demo's Spotify: a made-up player that plays and pauses, so the page
// can be seen and tested; nothing reaches Spotify. It starts paused, so the
// demo (and its film) keeps XenoNet's own music.
export function createDemoSpotify() {
  const p = { playing: false, title: "Night Drive", artists: "The Demo Ensemble", url: null, device: { id: "demo-device", name: "This computer", volume: 60 } };
  const settings = { clientId: "0".repeat(32), device: "demo-device", duck: true };
  return {
    settings: () => ({ ...settings }),
    status: async () => ({ app: true, connected: true, device: settings.device, duck: settings.duck, redirect: "http://127.0.0.1/api/spotify/callback", demo: true }),
    setApp: () => ({ app: true }),
    set: ({ device, duck } = {}) => (device !== undefined && (settings.device = device), duck !== undefined && (settings.duck = !!duck), { device: settings.device, duck: settings.duck }),
    begin: () => ({ url: null, demo: true }),
    callback: async () => ({ connected: true }),
    disconnect: async () => ({ connected: false }),
    player: async () => ({ ...p, device: { ...p.device } }),
    async command(op, value) {
      if (op === "play") p.playing = true;
      else if (op === "pause") p.playing = false;
      else if (op === "next") p.title = p.title === "Night Drive" ? "Low Orbit" : "Night Drive";
      else if (op === "volume") p.device.volume = Math.round(value);
      return null;
    },
  };
}
