import { ENVIRONMENTS } from "../core/environments.js";
import { environmentState, syncEnvironment, previewEnvironment, revertEnvironment, useEnvironment } from "./environment.js";
import { escapeHtml as h } from "./format.js";
import "./environments.css";

export function createEnvironments({ modals, post, toast, sfx, note, onCommands }) {
  let panel = null;
  let kept = false;
  let returnFocus = null;
  const commands = () => [
    { id: "environment-picker", label: "Environment…", keywords: "sky skybox background appearance aurora trench vault preview", run: open },
    ...ENVIRONMENTS.map((e) => ({ id: `environment-${e.id}`, label: `Environment: ${e.name}`, keywords: e.about, logs: true, run: () => set(e.id).catch((err) => toast(err.message, true)) })),
  ];
  async function load() {
    const r = await fetch("/api/environment");
    if (!r.ok) throw new Error("Couldn't read the environments");
    const body = await r.json();
    syncEnvironment(body.current);
    onCommands(commands());
    return body;
  }
  function valid(id) { if (!ENVIRONMENTS.some((e) => e.id === id)) throw new Error("choose an environment from the list"); }
  function preview(id) { valid(id); previewEnvironment(id); render(); }
  function revert() { revertEnvironment(); if (panel) modals.close(); }
  async function set(id) {
    valid(id);
    const r = await post("/api/environment", { id });
    const body = await r.json();
    if (!r.ok) throw new Error(body.error ?? "Couldn't change the environment");
    useEnvironment(body.id);
    note("environment", body.id);
    sfx("success");
    render();
    return body;
  }
  async function keep() {
    await set(environmentState().id);
    kept = true;
    if (panel) modals.close();
  }
  function render() {
    if (!panel) return;
    const state = environmentState();
    panel.innerHTML = `<header class="m-head"><div><div class="m-chip">Appearance · Environment</div><h2>Choose your surroundings</h2><div class="m-sub">Independent of your theme. Every environment follows its colors.</div></div><button class="act" data-env-cancel>Cancel ✕</button></header><div class="m-body"><div class="environment-cards" role="radiogroup" aria-label="Environment">${ENVIRONMENTS.map((e) => `<button class="environment-card" role="radio" aria-checked="${state.id === e.id}" tabindex="${state.id === e.id ? 0 : -1}" data-env="${e.id}"><span class="environment-art ${e.shape}" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></span><span class="environment-name">${h(e.name)}${state.inUse === e.id ? '<span class="tag">in use</span>' : ""}</span><span class="environment-about">${h(e.about)}</span></button>`).join("")}</div><div class="environment-footer"><span class="dim">Live preview · Arrow keys to browse · Enter to keep · Esc to cancel</span><button class="act primary" data-env-keep>Keep ${h(ENVIRONMENTS.find((e) => e.id === state.id).name)}</button></div></div>`;
    panel.querySelectorAll("[data-env]").forEach((b) => {
      b.onclick = () => { preview(b.dataset.env); panel.querySelector(`[data-env="${environmentState().id}"]`).focus(); };
      b.onkeydown = (e) => {
        if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(e.key)) {
          e.preventDefault();
          const i = ENVIRONMENTS.findIndex((x) => x.id === environmentState().id);
          const next = e.key === "Home" ? 0 : e.key === "End" ? ENVIRONMENTS.length - 1 : (i + (["ArrowLeft", "ArrowUp"].includes(e.key) ? -1 : 1) + ENVIRONMENTS.length) % ENVIRONMENTS.length;
          preview(ENVIRONMENTS[next].id);
          panel.querySelector(`[data-env="${environmentState().id}"]`).focus();
        } else if (e.key === "Enter") { e.preventDefault(); keep().catch((err) => toast(err.message, true)); }
      };
    });
    panel.querySelector("[data-env-cancel]").onclick = revert;
    panel.querySelector("[data-env-keep]").onclick = () => keep().catch((err) => toast(err.message, true));
  }
  function open() {
    if (panel) return;
    kept = false;
    returnFocus = document.activeElement;
    panel = modals.open("", { cls: "environment-panel", onClose: () => { if (!kept) revertEnvironment(); panel = null; returnFocus?.focus?.(); } });
    render();
    panel.querySelector(`[data-env="${environmentState().id}"]`).focus();
  }
  return { load, set, preview, keep, revert, open, state: () => ({ ...environmentState(), picker: !!panel }), list: ENVIRONMENTS };
}
