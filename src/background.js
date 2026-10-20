// Background service: whether XenoNet's service starts at login, and the
// offer Desktop makes once during setup (electron/background.js, through
// the xenonet command; docs/guide/always-on.md). Turning it on or off is
// yours: the switch and the offer's Start at login are data-yours, which
// the control channel doesn't press outside the demo and test instances.
import { escapeHtml as h } from "./format.js";

const WHY = "XenoNet's service is what lets sessions message each other, claim files and hear about edit collisions. Started at login, it keeps doing that while Desktop is closed, and starts again if it stops unexpectedly.";

// What the panel says of where the setting stands.
export function backgroundWords(s) {
  if (!s) return "Checking…";
  if (s.browser) return "Desktop sets this. In a terminal, xenonet service install starts the service at login.";
  if (!s.supported) return s.reason || "Starting the service at login isn't available on this system yet.";
  if (!s.live) return "XenoNet Dev starts and stops its own service. This setting is for installed XenoNet.";
  if (s.error) return `XenoNet couldn't ask ${s.supervisor ?? "the system"} about the service: ${s.error}`;
  if (!s.registered) return "Off. Desktop starts the service when it opens, and it runs until you log out.";
  if (s.supervised) return `On. The service is running under ${s.supervisor}, and starts at login.`;
  if (s.running) return `On. The service Desktop started keeps running for now, and the one ${s.supervisor} starts takes over when it stops.`;
  return `On. ${s.supervisor} starts the service at login.`;
}

export function createBackground({ modals, desktop, toast }) {
  const api = desktop?.background;
  let state = null;
  let panel = null;
  let offering = null;
  let busy = false;
  let message = "";
  // Why the last change failed, until one succeeds.
  let failed = null;

  async function refresh() {
    state = api ? await api.act("status") : { browser: true, live: false, supported: false };
    return state;
  }

  function render() {
    if (!panel?.isConnected) return;
    const s = state;
    const can = !!s && !s.browser && s.supported && s.live && !busy;
    panel.querySelector(".m-sub").textContent = !s ? "" : s.registered ? "Starts at login" : "Starts with Desktop";
    panel.querySelector(".m-body").innerHTML = `
      <p>${h(WHY)}</p>
      <p><label><input type="checkbox" data-background-login data-yours ${s?.registered ? "checked" : ""} ${can ? "" : "disabled"}> Start the service at login</label></p>
      <p role="status" data-background-status>${h(backgroundWords(s))}</p>
      ${s?.registered && s.supervisor === "systemd" && s.linger === false ? "<p>It starts when you log in. To start it at boot, before anyone logs in, run <code>loginctl enable-linger</code> once.</p>" : ""}
      ${s?.file ? `<p class="dim">${h(s.supervisor)} · ${h(s.file)}</p>` : ""}
      ${message ? `<p role="status" data-background-message>${h(message)}</p>` : ""}`;
  }

  async function set(on, { force = false } = {}) {
    if (!api) throw new Error(backgroundWords({ browser: true }));
    busy = true;
    message = on ? "Turning it on…" : "Turning it off…";
    render();
    try {
      state = await api.act("set", { on, force });
      failed = null;
      // The status line says where it stands; this says only what it
      // doesn't.
      const kept = { terminals: "The service running now has terminals open, so it keeps running until it stops." };
      message = on ? (kept[state.result?.takeover] ?? "") : state.result?.stopped ? "The service that started at login stopped, and Desktop started its own." : "";
      return state;
    } catch (err) {
      message = failed = err.message;
      await refresh().catch(() => {});
      throw err;
    } finally {
      busy = false;
      render();
    }
  }

  async function open() {
    message = "";
    // Open already: read again, in place.
    if (panel?.isConnected) {
      await refresh().catch(() => {});
      render();
      return state;
    }
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">XenoNet</div><h2>Background service</h2><div class="m-sub"></div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"></div>`, { onClose: () => (panel = null) });
    panel.addEventListener("change", (event) => {
      if (event.target.matches("[data-background-login]")) void set(event.target.checked).catch((err) => toast(err.message, true));
    });
    render();
    await refresh().catch((err) => (state = { browser: false, live: false, supported: false, reason: err.message }));
    render();
    return state;
  }

  // The setup offer, once: shown when Desktop says it's due. True when it's
  // showing. Closed without an answer counts as Not now: it's asked once.
  // Another panel opening over it isn't an answer: it comes back once that
  // one closes. One offer at a time, however many ask for it at once.
  let unanswered = false;
  let asking = null;
  function offer() {
    if (!api) return Promise.resolve(false);
    if (offering?.isConnected) return Promise.resolve(true);
    asking ??= show().finally(() => (asking = null));
    return asking;
  }

  async function show() {
    await refresh();
    if (!state?.offer) return false;
    unanswered = true;
    offering = modals.open(
      `<header class="m-head"><div><div class="m-chip">Setup</div><h2>Keep XenoNet running in the background?</h2><div class="m-sub">Background service</div></div></header>
      <div class="m-body">
        <p>${h(WHY)}</p>
        <p>You can change this later in Background service, in the command palette.</p>
        <div class="m-foot"><button class="act" data-background-offer="later">Not now</button><span class="grow"></span><button class="act primary" data-background-offer="on" data-yours>Start at login</button></div>
      </div>`,
      {
        onClose: () => {
          offering = null;
          // Replaced by another panel, the modal is still open (modals.js
          // takes the class away only on a close).
          const replaced = document.getElementById("modal")?.classList.contains("open");
          if (unanswered && replaced) offerWhenIdle({ wait: 1000 });
          else if (unanswered) void api.act("dismiss").then((s) => (state = s)).catch(() => {});
          unanswered = false;
        },
      },
    );
    offering.addEventListener("click", (event) => {
      const b = event.target.closest("[data-background-offer]");
      if (b) void answer(b.dataset.backgroundOffer === "on").catch((err) => toast(err.message, true));
    });
    return true;
  }

  async function answer(on) {
    unanswered = false;
    if (offering?.isConnected) modals.close();
    offering = null;
    if (!on) {
      state = await api.act("dismiss");
      return state;
    }
    await set(true);
    toast("XenoNet's service starts at login now.");
    return state;
  }

  // Shown once the page has settled, and not over a panel you have open.
  function offerWhenIdle({ wait = 4000, tries = 12 } = {}) {
    if (!api) return;
    const attempt = (left) =>
      setTimeout(() => {
        if (document.getElementById("modal")?.hidden === false) return left > 0 && attempt(left - 1);
        void offer().catch(() => {});
      }, wait);
    attempt(tries);
  }

  // Not now, as the offer's own button says it.
  async function dismiss() {
    if (!api) return state;
    return answer(false);
  }

  // Where the switch and the offer's buttons are, in CSS pixels, for a
  // click through the control channel (which won't press what's yours
  // outside the demo and test instances).
  const centre = (sel) => {
    const r = document.querySelector(`#modal ${sel}`)?.getBoundingClientRect();
    return r?.width ? { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) } : null;
  };

  return {
    open,
    offer,
    offerWhenIdle,
    dismiss,
    get state() {
      if (!state) return null;
      const at = { login: centre("[data-background-login]"), accept: centre('[data-background-offer="on"]'), later: centre('[data-background-offer="later"]') };
      return { ...state, offering: !!offering?.isConnected, message, failed, at };
    },
  };
}
