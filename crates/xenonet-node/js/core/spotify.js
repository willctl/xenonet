// core/spotify.js in Rust (crates/xenonet-app/src/spotify.rs): Spotify as a
// remote for the Spotify app you already use. The sign-in (PKCE), the
// tokens and each call's decisions are the Rust's; this awaits what it
// asks for, with the credential store and fetch given. The refresh token
// goes only to the credential store, and the access token only into a
// request's header.
import { native } from "../index.js";

const C = native.appSpotifyConstants();
export const SCOPES = C.scopes;
export const REFRESH_TARGET = C.refreshTarget;
export const CLIENT_ID = new RegExp(C.clientId, "i");

export const challengeOf = (verifier) => native.appSpotifyChallenge(String(verifier));

export class SpotifyError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}

const spotifyError = (e) => new SpotifyError(e.status, e.code, e.message);
function settle(answer) {
  if (!answer.error) return answer.ok;
  if (answer.error.spotify) throw spotifyError(answer.error.spotify);
  const { message, ...rest } = answer.error.fs;
  throw Object.assign(new Error(message), rest);
}

export const nowPlaying = (p) => native.appSpotifyNowPlaying(p);

export function createSpotify({ file, secrets, port, fetch = globalThis.fetch, now = Date.now, log = null }) {
  const core = new native.AppSpotify(String(file), port);

  // A job: what the Rust asks for, done here, until it's over.
  async function run(kind, args) {
    let { job, ask } = core.start(kind, args, Number(now()));
    let thrown;
    for (;;) {
      if ("done" in ask) return ask.done;
      if (ask.fail) throw spotifyError(ask.fail);
      if (ask.rethrow) throw thrown;
      let got;
      if (ask.log) {
        log?.[ask.log.level](ask.log.code, ask.log.message);
        got = { value: null };
      } else if (ask.secret) {
        const { op, target, value, comment } = ask.secret;
        try {
          got = { value: await (op === "set" ? secrets.set(target, value, { comment }) : secrets[op](target)) };
        } catch (err) {
          thrown = err;
          got = { threw: String(err?.message ?? err) };
        }
      } else {
        const f = ask.fetch;
        let res;
        try {
          res = await fetch(f.url, { method: f.method, headers: f.headers, ...(f.body == null ? {} : { body: f.body }), signal: AbortSignal.timeout(f.timeoutMs) });
        } catch (err) {
          thrown = err;
          got = { threw: String(err?.message) };
        }
        if (res) {
          const body = f.skip.includes(res.status) ? null : await res.json().catch(() => f.fallback);
          got = { res: { ok: !!res.ok, status: res.status, body, retryAfter: res.headers?.get?.("retry-after") ?? null } };
        }
      }
      ({ job, ask } = core.resume(job, got, Number(now())));
    }
  }

  return {
    settings: () => core.settings(),
    status: () => run("status", {}),
    setApp: (clientId) => settle(core.setApp(String(clientId ?? ""))),
    set: ({ device, duck } = {}) => settle(core.set(device, duck)),
    begin: () => settle(core.begin(Number(now()))),
    callback: async ({ code, state, error }) => run("callback", { code, state, error }),
    disconnect: () => run("disconnect", {}),
    player: () => run("player", {}),
    command: async (op, value = null) => run("command", { op, value }),
  };
}

export const duckPlan = ({ waiting, player, device, held, dip = 0.35 }) => native.appSpotifyDuck(!!waiting, player, device, held, Number(dip));

// The demo's Spotify: a made-up player that plays and pauses, so the page
// can be seen and tested; nothing reaches Spotify. It starts paused.
export function createDemoSpotify() {
  const p = native.appSpotifyDemoPlayer();
  const settings = native.appSpotifyDemoSettings();
  return {
    settings: () => ({ ...settings }),
    status: async () => ({ app: true, connected: true, device: settings.device, duck: settings.duck, redirect: C.redirect, demo: true }),
    setApp: () => ({ app: true }),
    set: ({ device, duck } = {}) => (device !== undefined && (settings.device = device), duck !== undefined && (settings.duck = !!duck), { device: settings.device, duck: settings.duck }),
    begin: () => ({ url: null, demo: true }),
    callback: async () => ({ connected: true }),
    disconnect: async () => ({ connected: false }),
    player: async () => ({ ...p, device: { ...p.device } }),
    async command(op, value) {
      Object.assign(p, native.appSpotifyDemoCommand(p, op, value));
      return null;
    },
  };
}
