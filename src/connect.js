// Connect your SIs: what XenoNet would add to Claude Code's and Codex's
// settings so that every session, in every project, can see the others and
// message them through XenoNet's engine (core/engine/connect.js does the
// adding). It shows each change before anything happens, then adds it all,
// or takes it all out again. Connect is marked data-yours, as Allow and New
// token are (docs/design/actors/interface.md, section 5): yours alone.
import { escapeHtml as h } from "./format.js";

const STATE = { present: ["✓", "added"], missing: ["+", "to add"], different: ["↻", "to update"] };

export function createConnectPanel({ modals, toast, sfx }) {
  let panel = null;

  function render(d) {
    const changes = d.apps.flatMap((a) => a.changes);
    const done = changes.every((c) => c.state === "present");
    const any = changes.some((c) => c.state !== "missing");
    const app = (a) => `
      <div class="d-k">${h(a.name)}${a.cli ? "" : ` <span class="dim">· command not found</span>`}</div>
      <ol class="d-list cx-list">${a.changes
        .map((c) => {
          const [glyph, label] = STATE[c.state] ?? ["?", c.state];
          return `<li class="cx-${c.state}"><span class="cx-state" title="${label}">${glyph}</span><span class="cx-what">${h(c.what)}<span class="dim">${h(c.where)}</span><code>${h(c.detail)}</code></span></li>`;
        })
        .join("")}</ol>
      ${a.notes.map((n) => `<p class="cx-note">${h(n)}</p>`).join("")}`;
    panel.querySelector(".m-body").innerHTML = `
      <p class="cx-intro">Once connected, each Claude Code and Codex session sees which other SIs work in its repo and on what. It gets a warning before it edits a file another SI is working on, and it can message the others. XenoNet adds only these entries, and keeps a copy of each settings file the first time it changes one. Sessions already running pick it up when they restart.</p>
      ${d.apps.map(app).join("")}
      ${d.demo ? `<p class="cx-note">This is the demo, with made-up sessions. Connect your SIs from XenoNet with your own.</p>` : ""}
      <div class="m-foot">
        <button class="act primary" data-cx-apply data-yours ${done || d.demo ? "disabled" : ""}>${done ? "Connected" : any ? "Update" : "Connect"}</button>
        ${any ? `<button class="act" data-cx-remove>Disconnect</button>` : ""}
        <span class="grow"></span><span class="dim">Through XenoNet on port ${d.port}</span>
      </div>`;
  }

  async function run(method, body) {
    const res = await fetch("/api/engine/connect", { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
    const data = await res.json();
    if (!res.ok) throw new Error(data.error ?? res.statusText);
    return data;
  }

  async function open() {
    panel = modals.open(
      `<header class="m-head"><div><div class="m-chip">⬡ SI</div><h2>Connect your SIs</h2><div class="m-sub">Claude Code and Codex sessions reach each other through XenoNet</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"><div class="m-empty">Checking Claude Code's and Codex's settings…</div></div>`,
      { wide: true, cls: "cx" },
    );
    panel.addEventListener("click", async (e) => {
      const apply = e.target.closest("[data-cx-apply]"), remove = e.target.closest("[data-cx-remove]");
      if (!apply && !remove) return;
      e.target.disabled = true;
      try {
        render(await run(apply ? "POST" : "DELETE", apply ? { apps: ["claude", "codex"] } : null));
        sfx("success");
        toast(apply ? "Connected. Restart running sessions to pick it up." : "Disconnected");
      } catch (err) {
        sfx("error");
        toast(`Couldn't change the settings: ${err.message}`, true);
        e.target.disabled = false;
      }
    });
    try {
      render(await run("GET"));
    } catch (err) {
      panel.querySelector(".m-body").innerHTML = `<div class="m-empty">${h(err.message)}</div>`;
    }
  }

  return { open };
}
