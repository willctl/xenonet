// Spotify as a remote for your own app (core/spotify.js): PKCE sign-in that
// comes back once, in time, with its own state; the refresh token in the
// credential store and kept when Spotify sends no new one; one that won't
// refresh means connecting again; a 401 refreshed once, a 429 waited out;
// what's playing as the page gets it; and the volume dip while you're
// needed, on this computer's device only, given back unless you changed
// it. Spotify and the tokens are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { createSpotify, challengeOf, nowPlaying, duckPlan, REFRESH_TARGET, SCOPES } from "../core/spotify.js";
import { createMemorySecrets } from "../core/secret-store.js";

const CLIENT = "0123456789abcdef0123456789abcdef";

function fakeSpotify() {
  const calls = [];
  let tokens = 0;
  const replies = [];
  const fetch = async (url, init = {}) => {
    calls.push({ url: String(url), method: init.method ?? "GET", headers: init.headers ?? {}, body: init.body ?? null });
    const next = replies.shift();
    if (next) return next;
    if (String(url).endsWith("/api/token")) {
      tokens++;
      const form = new URLSearchParams(init.body);
      return { ok: true, status: 200, headers: { get: () => null }, json: async () => ({ access_token: `access-${tokens}`, expires_in: 3600, ...(form.get("grant_type") === "authorization_code" ? { refresh_token: "refresh-first" } : {}) }) };
    }
    return { ok: true, status: 200, headers: { get: () => null }, json: async () => ({ is_playing: true, device: { id: "dev-here", name: "This PC", volume_percent: 70 }, item: { name: "Night Drive", artists: [{ name: "A" }, { name: "B" }], external_urls: { spotify: "https://open.spotify.com/track/x" } } }) };
  };
  return { calls, fetch, replies };
}

function rig() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-spotify-"));
  const secrets = createMemorySecrets();
  const sp = fakeSpotify();
  let t = 1_800_000_000_000;
  const spotify = createSpotify({ file: path.join(dir, "spotify.json"), secrets, port: 47840, fetch: sp.fetch, now: () => t });
  return { spotify, secrets, sp, dir, advance: (ms) => (t += ms), done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

test("connecting: your app's client id, a challenge and state to Spotify, and only the same state back, once, within ten minutes", async () => {
  const r = rig();
  try {
    assert.throws(() => r.spotify.begin(), /client id first/);
    assert.throws(() => r.spotify.setApp("not-an-id"), /32 letters and digits/);
    r.spotify.setApp(CLIENT);
    const { url } = r.spotify.begin();
    const q = new URL(url).searchParams;
    assert.equal(new URL(url).origin + new URL(url).pathname, "https://accounts.spotify.com/authorize");
    assert.deepEqual([q.get("client_id"), q.get("response_type"), q.get("code_challenge_method"), q.get("redirect_uri"), q.get("scope")], [CLIENT, "code", "S256", "http://127.0.0.1:47840/api/spotify/callback", SCOPES.join(" ")]);
    await assert.rejects(r.spotify.callback({ code: "c", state: "someone-elses" }), /isn't the one XenoNet started/);
    await assert.rejects(r.spotify.callback({ code: "c", state: q.get("state") }), /isn't the one XenoNet started/, "used once, even after a wrong one");
    const again = new URL(r.spotify.begin().url).searchParams;
    await r.spotify.callback({ code: "the-code", state: again.get("state") });
    const exchange = r.sp.calls.find((c) => c.url.endsWith("/api/token"));
    const form = new URLSearchParams(exchange.body);
    assert.equal(form.get("grant_type"), "authorization_code");
    assert.equal(challengeOf(form.get("code_verifier")), again.get("code_challenge"), "the verifier matches the challenge");
    assert.equal(await r.secrets.get(REFRESH_TARGET), "refresh-first");
    assert.deepEqual((await r.spotify.status()).connected, true);
    const late = new URL(r.spotify.begin().url).searchParams;
    r.advance(11 * 60e3);
    await assert.rejects(r.spotify.callback({ code: "c", state: late.get("state") }), /took too long/);
  } finally {
    r.done();
  }
});

test("playing: what the page gets, the token only in a header, refreshed a little early, the refresh token kept when none comes back", async () => {
  const r = rig();
  try {
    r.spotify.setApp(CLIENT);
    await r.secrets.set(REFRESH_TARGET, "refresh-kept");
    const now = await r.spotify.player();
    assert.deepEqual(now, { playing: true, title: "Night Drive", artists: "A, B", url: "https://open.spotify.com/track/x", device: { id: "dev-here", name: "This PC", volume: 70 } });
    const call = r.sp.calls.find((c) => c.url.endsWith("/me/player"));
    assert.equal(call.headers.authorization, "Bearer access-1");
    assert.ok(!call.url.includes("access-1"), "never in the URL");
    assert.equal(await r.secrets.get(REFRESH_TARGET), "refresh-kept", "no new refresh token: the old one stays");
    r.advance(3600e3 - 60e3);
    await r.spotify.player();
    assert.equal(r.sp.calls.filter((c) => c.url.endsWith("/api/token")).length, 2, "refreshed before it ran out");
    // A 401 is refreshed once and tried again.
    r.sp.replies.push({ ok: false, status: 401, headers: { get: () => null }, json: async () => ({}) });
    assert.equal((await r.spotify.player()).title, "Night Drive");
    // A 429 waits as long as Spotify says.
    r.sp.replies.push({ ok: false, status: 429, headers: { get: (h) => (h === "retry-after" ? "7" : null) }, json: async () => ({}) });
    await assert.rejects(r.spotify.player(), (e) => e.code === "busy");
    await assert.rejects(r.spotify.player(), (e) => e.code === "busy", "still waiting");
    r.advance(8000);
    assert.ok(await r.spotify.player());
    // Commands, to this computer's device once it's chosen.
    r.spotify.set({ device: "dev-here" });
    r.sp.replies.push({ ok: true, status: 204, headers: { get: () => null }, json: async () => null });
    await r.spotify.command("pause");
    assert.match(r.sp.calls.at(-1).url, /\/me\/player\/pause\?device_id=dev-here$/);
    await assert.rejects(r.spotify.command("volume", 140), /0 to 100/);
    await assert.rejects(r.spotify.command("rm -rf"), /play, pause/);
    // Premium is what the player needs.
    r.sp.replies.push({ ok: false, status: 403, headers: { get: () => null }, json: async () => ({ error: { message: "Player command failed: Premium required" } }) });
    await assert.rejects(r.spotify.command("next"), (e) => e.code === "premium");
  } finally {
    r.done();
  }
});

test("a refresh token Spotify won't take any more is forgotten: connect again", async () => {
  const r = rig();
  try {
    r.spotify.setApp(CLIENT);
    await r.secrets.set(REFRESH_TARGET, "refresh-revoked");
    r.sp.replies.push({ ok: false, status: 400, headers: { get: () => null }, json: async () => ({ error: "invalid_grant", error_description: "Refresh token revoked" }) });
    await assert.rejects(r.spotify.player(), (e) => e.code === "reconnect");
    assert.equal(await r.secrets.get(REFRESH_TARGET), null);
    assert.equal((await r.spotify.status()).connected, false);
  } finally {
    r.done();
  }
});

test("what's playing as the page gets it: a podcast's show for its artist, nothing playing, and only Spotify's own links", () => {
  assert.deepEqual(nowPlaying(null).playing, false);
  assert.equal(nowPlaying({ is_playing: true, item: { name: "Ep 1", show: { name: "The Show" } } }).artists, "The Show");
  assert.equal(nowPlaying({ item: { name: "x", artists: [], external_urls: { spotify: "javascript:alert(1)" } } }).url, null);
});

test("the dip while you're needed: on this computer's device only, given back after, and yours stands if you changed it", () => {
  const player = (volume, id = "here", playing = true) => ({ playing, device: { id, name: "PC", volume } });
  let step = duckPlan({ waiting: true, player: player(70), device: "here", held: null });
  assert.deepEqual([step.op, step.volume, step.held], ["volume", 25, { from: 70, to: 25 }]);
  assert.deepEqual(duckPlan({ waiting: true, player: player(25), device: "here", held: step.held }).op, null, "held while still waiting");
  step = duckPlan({ waiting: false, player: player(25), device: "here", held: step.held });
  assert.deepEqual([step.op, step.volume, step.held], ["volume", 70, null], "given back");
  const changed = duckPlan({ waiting: false, player: player(40), device: "here", held: { from: 70, to: 25 } });
  assert.deepEqual([changed.op, changed.held], [null, null], "you changed it meanwhile: yours stands");
  assert.equal(duckPlan({ waiting: true, player: player(70, "speaker-in-the-kitchen"), device: "here", held: null }).op, null, "never another device");
  assert.equal(duckPlan({ waiting: true, player: player(70, "here", false), device: "here", held: null }).op, null, "nothing to dip when paused");
  assert.equal(duckPlan({ waiting: true, player: player(70), device: null, held: null }).op, null, "not until you've said which device is this computer");
  void crypto;
});
