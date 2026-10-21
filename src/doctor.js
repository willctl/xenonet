// XenoNet's doctor in the console: every check (core/doctor), grouped, with a
// fix for each problem, and a button to check again. The palette opens it
// (Check XenoNet's setup), and so do the doctor's alerts and notifications.
//
// Under the checks, what went wrong lately (XenoNet's error log, core/log.js)
// by code, and two ways to get help: Copy diagnostics, a summary safe to
// share (core/diagnostics.js), and Ask Codex or Claude Code, which starts a
// session of the one you pick on this computer, in a terminal here, to look
// at it with you. That session never files anything; sending an issue stays
// yours.
import { escapeHtml as h, timeAgo } from "./format.js";
import { tailedApp } from "../core/apps.js";

const MARK = { ok: "✓", info: "·", warn: "!", fail: "✗", skip: "–" };
const LEVEL_MARK = { error: "✗", warn: "!", info: "·" };
const ASK_KEY = "xenonet.diagnose.ask";

// The app and model you last asked with, in this window only.
function lastAsk() {
  try {
    const v = JSON.parse(localStorage.getItem(ASK_KEY) ?? "null");
    return { app: tailedApp(v?.app, "codex"), model: typeof v?.model === "string" ? v.model : "" };
  } catch {
    return { app: "codex", model: "" };
  }
}

// `terminal(dir, command)` runs a command in a new terminal tab in a
// folder, where the window has terminals (Desktop); null where it hasn't.
export function createDoctorPanel({ modals, sfx, toast, onResult = () => {}, terminal = null }) {
  let panel = null;
  let last = null;
  let asking = null; // the Ask Codex or Claude Code view's choices, while it shows

  function render(r) {
    last = r;
    asking = null;
    const groups = [...new Set(r.results.map((x) => x.group))];
    const { fail = 0, warn = 0 } = r.summary;
    const verdict = fail ? `${fail} problem${fail === 1 ? "" : "s"}${warn ? ` and ${warn} warning${warn === 1 ? "" : "s"}` : ""}` : warn ? `${warn} warning${warn === 1 ? "" : "s"}` : "Everything XenoNet needs is in place";
    panel.querySelector(".m-sub").textContent = `${verdict} · checked ${timeAgo(r.at)} in ${(r.ms / 1000).toFixed(1)} s`;
    panel.querySelector(".m-body").innerHTML = `
      ${groups
        .map(
          (g) => `<div class="d-k">${h(g)}</div><ol class="d-list dr-list">${r.results
            .filter((x) => x.group === g)
            .map((x) => `<li class="dr-${x.status}"><span class="dr-mark" title="${x.status}">${MARK[x.status]}</span><span class="dr-what">${h(x.title)}<span class="dim">${h(x.detail)}</span>${x.fix && (x.status === "warn" || x.status === "fail" || x.status === "info") ? `<span class="dr-fix">${h(x.fix)}</span>` : ""}</span></li>`)
            .join("")}</ol>`,
        )
        .join("")}
      <div class="d-k">Lately, in XenoNet's log</div>
      <ol class="d-list dr-list" data-dr-errors><li class="dr-skip"><span class="dr-mark">–</span><span class="dr-what"><span class="dim">Reading…</span></span></li></ol>
      <div class="m-foot"><button class="act primary" data-dr-run>Check again</button><button class="act" data-dr-copy title="A summary that's safe to share: versions, problems and errors by code, never transcripts, names or keys">Copy diagnostics</button><button class="act" data-dr-ask title="A Codex or Claude Code session here looks at it with you, and never files anything">Ask Codex or Claude Code…</button><span class="grow"></span><span class="dim">XenoNet checks every half hour; in a terminal, xenonet doctor checks at once</span></div>`;
    loadErrors();
  }

  // The last day's errors and warnings, by code.
  async function loadErrors() {
    const list = panel?.querySelector("[data-dr-errors]");
    if (!list) return;
    const r = await fetch(`/api/logs?summary=1&level=warn&since=${Date.now() - 24 * 3600e3}`).then((x) => (x.ok ? x.json() : null)).catch(() => null);
    if (!list.isConnected) return;
    const rows = r?.summary ?? [];
    list.innerHTML = rows.length
      ? rows
          .slice(0, 12)
          .map((e) => `<li class="dr-${e.level === "error" ? "fail" : e.level}"><span class="dr-mark" title="${h(e.level)}">${LEVEL_MARK[e.level] ?? "·"}</span><span class="dr-what"><code>${h(e.code)}</code> <span class="dim">${h(e.component)} · ${e.count > 1 ? `${e.count} times, ` : ""}last ${timeAgo(e.last)}</span><span class="dim">${h(e.message)}</span>${e.fix ? `<span class="dr-fix">${h(e.fix)}</span>` : ""}</span></li>`)
          .join("")
      : `<li class="dr-ok"><span class="dr-mark">✓</span><span class="dr-what">Nothing went wrong in the last day<span class="dim">xenonet logs shows what XenoNet keeps</span></span></li>`;
  }

  async function copyDiagnostics(btn) {
    btn.disabled = true;
    try {
      const r = await fetch("/api/diagnose");
      const data = await r.json();
      if (!r.ok) throw new Error(data.error ?? r.statusText);
      await navigator.clipboard.writeText(data.markdown);
      sfx("select");
      toast("Diagnostics copied: no transcripts, names or keys in them");
    } catch (err) {
      sfx("error");
      toast(`Couldn't copy them: ${err.message}`, true);
    } finally {
      btn.disabled = false;
    }
  }

  // Choosing who looks: Codex or Claude Code, and its model.
  function askView() {
    asking ??= lastAsk();
    panel.querySelector(".m-sub").textContent = "A Codex or Claude Code session on this computer looks at it with you";
    const pick = (app, label) => `<button class="toggle ${asking.app === app ? "on" : ""}" data-dr-app="${app}" aria-pressed="${asking.app === app}">${label}</button>`;
    panel.querySelector(".m-body").innerHTML = `
      <p class="pj-note">XenoNet writes what it can tell (the doctor's findings and its errors by code, with no transcripts, names or keys) to a folder of its own, and starts a session there that reads it, with XenoNet's own code, and talks it through with you. It works read-only and asks before changing anything. It never files an issue, opens a pull request or sends anything: if it finds a bug in XenoNet, it writes issue-draft.md in that folder, for you to send if you choose.</p>
      <div class="ac"><div class="ac-form">
        <div class="ac-field"><span class="ac-k">app</span><div class="ac-caps">${pick("codex", "Codex")}${pick("claude", "Claude Code")}</div></div>
        <label class="ac-field"><span class="ac-k">model</span><input class="ac-input" data-dr-model maxlength="80" placeholder="its default, or a model of yours" spellcheck="false" autocomplete="off" value="${h(asking.model)}" /></label>
      </div>
      <div data-dr-command hidden></div></div>
      <div class="m-foot"><button class="act" data-dr-back>Back</button><span class="grow"></span><button class="act primary" data-dr-start>${terminal ? "Start it in a terminal" : "Show the command"}</button></div>`;
  }

  async function startAsk(btn) {
    asking.model = panel.querySelector("[data-dr-model]")?.value.trim() ?? "";
    try {
      localStorage.setItem(ASK_KEY, JSON.stringify(asking));
    } catch {}
    btn.disabled = true;
    try {
      const r = await fetch("/api/diagnose/ask", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ app: asking.app, model: asking.model || null }) });
      const data = await r.json();
      if (!r.ok) throw new Error(data.error ?? r.statusText);
      if (terminal) {
        await terminal(data.dir, data.command);
        sfx("success");
        modals.close();
        return;
      }
      const box = panel.querySelector("[data-dr-command]");
      box.hidden = false;
      box.innerHTML = `<p class="pj-note">Run this in a terminal, from ${h(data.folder)}:</p><code class="ac-token">${h(data.command)}</code>`;
      sfx("success");
    } catch (err) {
      sfx("error");
      toast(`Couldn't start it: ${err.message}`, true);
    } finally {
      btn.disabled = false;
    }
  }

  async function open() {
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">✚ doctor</div><h2>XenoNet's setup</h2><div class="m-sub">Checking…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"><div class="m-empty">Checking sign-ins, folders and connections…</div></div>`, { wide: true, cls: "dr" });
    panel.addEventListener("click", async (e) => {
      const btn = e.target.closest("button");
      if (!btn || btn.disabled) return;
      const d = btn.dataset;
      if (d.drCopy !== undefined) return copyDiagnostics(btn);
      if (d.drAsk !== undefined) return askView();
      if (d.drBack !== undefined) return last && render(last);
      if (d.drApp) {
        asking.model = panel.querySelector("[data-dr-model]")?.value.trim() ?? asking.model;
        asking.app = d.drApp;
        sfx("toggle");
        return askView();
      }
      if (d.drStart !== undefined) return startAsk(btn);
      if (d.drRun === undefined) return;
      btn.disabled = true;
      btn.textContent = "Checking…";
      try {
        render(await load("POST"));
        sfx("success");
      } catch (err) {
        sfx("error");
        toast(`The doctor couldn't run: ${err.message}`, true);
        btn.disabled = false;
        btn.textContent = "Check again";
      }
    });
    try {
      render(await load("GET"));
    } catch (err) {
      panel.querySelector(".m-body").innerHTML = `<div class="m-empty">${h(err.message)}</div>`;
    }
  }

  async function load(method) {
    const res = await fetch("/api/doctor", { method });
    const r = await res.json();
    if (!res.ok) throw new Error(r.error ?? res.statusText);
    onResult(r);
    return r;
  }

  return { open, load };
}
