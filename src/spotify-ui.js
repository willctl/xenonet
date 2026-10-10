// Spotify in the console (core/spotify.js; docs/guide/spotify.md): what's
// playing, under the status panel, linked to Spotify, with play or pause
// and the next track; and the window to connect it. While Spotify plays,
// XenoNet's own music steps aside (ambient.yieldTo), never mixed with it.
//
// Its commands go through the window's own routes with its key; the page
// never holds a Spotify token.
import { escapeHtml as h } from "./format.js";

export function createSpotifyChip({ el, calls, modals, toast, sfx, ambient }) {
  let now = null;
  let status = null;

  function render() {
    const n = now;
    ambient.yieldTo(!!n?.playing);
    if (!n || !n.title) {
      el.hidden = true;
      el.innerHTML = "";
      return;
    }
    el.hidden = false;
    const what = `${h(n.title)}${n.artists ? ` <span class="dim">· ${h(n.artists)}</span>` : ""}`;
    el.innerHTML = `<span class="sp-mark" title="Spotify${n.device ? ` on ${h(n.device.name)}` : ""}">♫</span>${n.url ? `<a class="sp-what" href="${h(n.url)}" target="_blank" rel="noreferrer" title="Open in Spotify">${what}</a>` : `<span class="sp-what">${what}</span>`}<button class="sp-btn" data-sp="${n.playing ? "pause" : "play"}" title="${n.playing ? "Pause" : "Play"}">${n.playing ? "❚❚" : "▶"}</button><button class="sp-btn" data-sp="next" title="Next track">⏭</button>`;
  }

  el.addEventListener("click", async (e) => {
    const b = e.target.closest("[data-sp]");
    if (!b) return;
    b.disabled = true;
    const r = await calls.call("POST", "/api/spotify/player", { op: b.dataset.sp });
    if (!r.ok) {
      sfx("error");
      toast(r.error ?? "Spotify didn't take it", true);
    } else if (now && (b.dataset.sp === "play" || b.dataset.sp === "pause")) {
      now = { ...now, playing: b.dataset.sp === "play" };
      render();
    }
    b.disabled = false;
  });

  async function refresh() {
    status = await fetch("/api/spotify").then((r) => (r.ok ? r.json() : null)).catch(() => null);
    now = status?.now ?? null;
    render();
    return status;
  }

  // Connecting: your Spotify app's client id, then Spotify's sign-in in
  // your browser; once connected, which device is this computer and
  // whether the volume dips while a session waits on you.
  async function open() {
    await refresh();
    const s = status ?? {};
    const steps = `<ol class="sp-steps"><li>On <a href="https://developer.spotify.com/dashboard" target="_blank" rel="noreferrer">Spotify's developer dashboard</a>, create an app with the Web API.</li><li>Add <code>${h(s.redirect ?? "http://127.0.0.1/api/spotify/callback")}</code> as its redirect URI.</li><li>Paste its client id here, then Connect, and sign in to Spotify in your browser.</li></ol>`;
    const body = !s.connected
      ? `<p class="pj-note">XenoNet can show what's playing in Spotify and play, pause or skip it, and step its own music aside while Spotify plays. It works through an app of your own on Spotify's dashboard, and Spotify's player needs Premium.</p>${steps}<div class="ac"><div class="ac-form"><label class="ac-field"><span class="ac-k">client id</span><input class="ac-input" data-sp-client maxlength="32" spellcheck="false" autocomplete="off" placeholder="32 letters and digits" /></label></div></div><div class="m-foot"><span class="grow"></span><button class="act primary" data-sp-connect data-yours>Connect</button></div>`
      : `<p class="pj-note">Spotify is connected${s.demo ? " (made up, in the demo)" : ""}. XenoNet reads what's playing every few seconds while a window is open, and sends only the commands you give or turn on. Nothing of your sessions reaches Spotify.</p><div class="ac"><div class="ac-form"><div class="ac-field"><span class="ac-k">this computer</span><div class="ac-caps"><button class="toggle ${s.device && s.device === now?.device?.id ? "on" : ""}" data-sp-here>${now?.device ? `${h(now.device.name)} plays here` : "Play something in Spotify first"}</button></div><small class="dim">the only device XenoNet turns down</small></div><div class="ac-field"><span class="ac-k">while you're needed</span><div class="ac-caps"><button class="toggle ${s.duck ? "on" : ""}" data-sp-duck aria-pressed="${!!s.duck}">Dip the volume while a session waits on you</button></div><small class="dim">it comes back after, unless you changed it</small></div></div></div><div class="m-foot"><button class="act" data-sp-disconnect>Disconnect</button><span class="grow"></span><button class="act primary" data-mclose>Done</button></div>`;
    const panel = modals.open(`<header class="m-head"><div><div class="m-chip">♫ spotify</div><h2>Spotify</h2><div class="m-sub">${s.connected ? "Connected" : "Not connected"}</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body">${body}</div>`, { accent: "var(--accent)" });
    panel.addEventListener("click", async (e) => {
      const t = e.target.closest("button");
      if (!t || t.disabled) return;
      if (t.matches("[data-sp-connect]")) {
        const id = panel.querySelector("[data-sp-client]")?.value.trim();
        if (id) {
          const set = await calls.call("POST", "/api/spotify/app", { clientId: id });
          if (!set.ok) return toast(set.error ?? "Spotify's client id wasn't taken", true);
        }
        const r = await calls.call("POST", "/api/spotify/connect");
        if (!r.ok) return toast(r.error ?? "Couldn't start connecting", true);
        if (r.data.url) window.open(r.data.url, "_blank", "noopener");
        toast("Sign in to Spotify in your browser, then come back");
        modals.close();
      } else if (t.matches("[data-sp-here]")) {
        const r = await calls.call("POST", "/api/spotify/settings", { device: "current" });
        if (r.ok) (sfx("toggle"), modals.close(), open());
      } else if (t.matches("[data-sp-duck]")) {
        const r = await calls.call("POST", "/api/spotify/settings", { duck: !s.duck });
        if (r.ok) (sfx("toggle"), modals.close(), open());
      } else if (t.matches("[data-sp-disconnect]")) {
        await fetch("/api/spotify/disconnect", { method: "POST" });
        now = null;
        render();
        modals.close();
        toast("Spotify is disconnected");
      }
    });
  }

  return {
    refresh,
    open,
    // What's playing, from the server's spotify event.
    heard(next) {
      now = next;
      render();
    },
    get state() {
      return { connected: !!status?.connected, now, yielding: ambient.yielding, shown: !el.hidden, text: el.textContent.trim() };
    },
  };
}
