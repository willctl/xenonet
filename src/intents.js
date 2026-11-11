// The person's intents (crates/xenonet-runtime/src/intents.rs): work asked
// for here, done on the machine it names under one of their delegations.
// The view lists them with how each stands, and holds the form for a new
// one. Making and cancelling sign as the person (the window's own routes,
// where the device signs), so they're the person's clicks: the driver can
// fill the form in, never send it.

import { escapeHtml as h } from "./format.js";
import "./intents.css";

const APPS = [["claude", "Claude Code"], ["codex", "Codex"], ["grok", "Grok Build"]];
const OPEN = new Set(["made", "assigned", "running"]);
const blank = () => ({ goal: "", target: "", project: "", app: "claude", hours: 24 });
const stamp = (at) => (Number.isFinite(at) ? new Date(at).toLocaleString() : "");

export function createIntentsPanel({ modals, calls }) {
  // error: why the list couldn't be read; said: why the person's last Make or
  // Cancel didn't go through, until they do another or refresh.
  let panel = null, data = null, selected = null, error = null, said = null, draft = blank(), busy = false;
  const el = (s) => panel?.querySelector(s);
  const brief = (i) => ({ id: i.id, state: i.state, maker: i.maker, target: i.target, project: i.project, app: i.app, goal: i.goal, note: i.note ?? null, outcome: i.outcome ?? null, here: !!i.here });
  const state = () => ({ open: !!panel, intents: data?.intents.map(brief) ?? [], machines: data?.machines ?? [], projects: (data?.projects ?? []).map((p) => p.id), selected, draft: { ...draft }, error: said ?? error });

  async function load() {
    const r = await calls.call("GET", "/api/intents");
    if (!panel) return;
    if (!r.ok) {
      error = r.error || "XenoNet couldn't read your intents";
      data = null;
    } else {
      error = null;
      data = r.data;
    }
    render();
  }

  function options(list, value, label = (x) => x) {
    return list.map((x) => { const v = typeof x === "string" ? x : x.id; return `<option value="${h(v)}"${v === value ? " selected" : ""}>${h(label(x))}</option>`; }).join("");
  }

  function render() {
    if (!panel) return;
    const intents = data?.intents ?? [];
    if (!draft.target && data?.machines?.length) draft.target = data.machines[0];
    if (!draft.project && data?.projects?.length) draft.project = data.projects[0].id;
    el("[data-in-target]").innerHTML = options(data?.machines ?? [], draft.target, (m) => (m === data?.here ? `${m} (this machine)` : m));
    el("[data-in-project]").innerHTML = options(data?.projects ?? [], draft.project, (p) => (p.name ? `${p.name} · ${p.id}` : p.id));
    el("[data-in-app]").value = draft.app;
    el("[data-in-goal]").value = draft.goal;
    el("[data-in-hours]").value = String(draft.hours);
    el("[data-in-status]").textContent = said ?? error ?? (intents.length ? `${intents.length} intent${intents.length === 1 ? "" : "s"}` : "No intents yet.");
    el("[data-in-list]").innerHTML = intents.length
      ? intents.map((i) => `<div class="in-row" data-in-id="${h(i.id)}" aria-pressed="${i.id === selected}" role="button" tabindex="0">
          <div class="in-row-top"><span class="in-state in-${h(i.state)}">${h(i.state)}</span><strong>${h(i.maker)} → ${h(i.target)}</strong><span class="in-id">${h(i.id)}</span></div>
          <div class="in-goal">${h(i.goal)}</div>
          ${i.outcome ? `<div class="in-said">${h(i.outcome)}</div>` : i.note ? `<div class="in-said">${h(i.note)}</div>` : ""}
          <div class="in-row-bottom"><span>${h(i.app)} · ${h(i.project)}</span><time>${h(stamp(i.madeAt))}</time>${OPEN.has(i.state) ? `<button class="act" data-in-cancel="${h(i.id)}" data-yours>Cancel</button>` : ""}</div>
        </div>`).join("")
      : '<p class="in-note">Make one above: say what you want done and which machine does it.</p>';
    el("[data-in-make]").disabled = busy;
  }

  function fill(value = {}) {
    for (const k of Object.keys(draft)) if (value[k] !== undefined && value[k] !== null) draft[k] = k === "hours" ? Number(value[k]) : String(value[k]);
    render();
  }

  // What the form says now, as typed.
  function read() {
    draft = { goal: el("[data-in-goal]").value, target: el("[data-in-target]").value, project: el("[data-in-project]").value, app: el("[data-in-app]").value, hours: Number(el("[data-in-hours]").value) || 24 };
  }

  async function make() {
    read();
    busy = true; render();
    const r = await calls.call("POST", "/api/intents", { goal: draft.goal, target: draft.target, project: draft.project, app: draft.app, hours: draft.hours });
    busy = false;
    if (!r.ok) said = r.error || "XenoNet didn't make the intent";
    else { said = null; draft = { ...blank(), target: draft.target, project: draft.project }; selected = r.data.id; }
    await load();
  }

  async function cancel(id) {
    const r = await calls.call("POST", `/api/intents/${encodeURIComponent(id)}/cancel`, {});
    said = r.ok ? null : r.error || "XenoNet didn't cancel it";
    await load();
  }

  function select(id) {
    if (id && !data?.intents.some((i) => i.id === id)) throw new Error(`No intent ${id} here`);
    selected = id ?? null;
    render();
  }

  async function open() {
    if (panel) return state();
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">Intents</div><h2>Your intents</h2><div class="m-sub">Ask for work here, done on the machine you name, under your delegation.</div></div><button class="act" data-mclose>Close ✕</button></header>
      <div class="m-body in-body">
        <form class="in-form" data-in-form>
          <label class="in-wide">Goal<textarea data-in-goal rows="3" placeholder="What should be done, and what done looks like"></textarea></label>
          <label>Machine<select data-in-target aria-label="Machine"></select></label>
          <label>Project<select data-in-project aria-label="Project"></select></label>
          <label>App<select data-in-app aria-label="App">${APPS.map(([v, n]) => `<option value="${v}">${n}</option>`).join("")}</select></label>
          <label>Hours<input data-in-hours type="number" min="1" max="720" step="1" aria-label="Hours"></label>
          <div class="in-actions"><span class="in-note">Your device signs it (Touch ID on a Mac).</span><button class="act" type="submit" data-in-make data-yours>Make</button></div>
        </form>
        <div class="in-toolbar"><span data-in-status role="status" aria-live="polite"></span><button class="act" data-in-refresh>Refresh</button></div>
        <div data-in-list class="in-list" aria-label="Intents"></div>
      </div>`, { wide: true, cls: "in-panel", accent: "var(--violet)", onClose: () => { panel = null; } });
    panel.addEventListener("submit", (e) => { e.preventDefault(); make(); });
    panel.addEventListener("input", (e) => { if (e.target.closest("[data-in-form]")) read(); });
    panel.addEventListener("click", (e) => {
      const stop = e.target.closest("[data-in-cancel]");
      if (stop) return void cancel(stop.dataset.inCancel);
      if (e.target.closest("[data-in-refresh]")) return void load();
      const row = e.target.closest("[data-in-id]");
      if (row) select(row.dataset.inId);
    });
    await load();
    return state();
  }

  return {
    open,
    get state() { return state(); },
    // The engine said the intents changed.
    changed() { if (panel) load(); },
    async action(action = "open", value) {
      if (action === "close") { if (panel) modals.close(); panel = null; return state(); }
      if (!panel) await open();
      if (action === "open") return state();
      if (action === "refresh") { said = null; await load(); }
      else if (action === "select") select(value);
      else if (action === "draft") fill(value ?? {});
      // Making and cancelling sign as the person: their clicks, not the
      // driver's.
      else throw new Error(`Unknown intents action: ${action}`);
      return state();
    },
  };
}
