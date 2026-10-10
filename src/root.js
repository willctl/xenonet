// The projects folder XenoNet reads, from the console (core/workspace.js):
// the folder in use and where that comes from; another one, picked with
// the desktop app's folder picker or typed; what XenoNet would find there,
// as you go; and Use this folder, which rescans and redraws the map.
// "Change the projects folder…" in the palette opens it, and the driver's
// root(), checkRoot() and setRoot() go through here too.
import "./root.css";
import { escapeHtml as h, timeAgo, tildePath } from "./format.js";
import { note } from "./oplog.js";

const FROM = { env: "set by XENONET_PROJECTS_DIR", chosen: "chosen in XenoNet", default: "the default" };
const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
const found = (sc) => `${plural(sc.projects, "project")} in ${plural(sc.groups, "group")}`;

async function call(url, method = "GET", body) {
  const res = await fetch(url, { method, ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }) });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error ?? res.statusText);
  return data;
}

export function createRootPanel({ modals, sfx, toast, desktop = null }) {
  // The folder in use, where that comes from, and its last scan.
  const get = () => call("/api/workspace/root");
  // What XenoNet would find in a folder (a full path, or one starting with ~),
  // without reading it for the map.
  const check = (path) => call("/api/workspace/check", "POST", { path });
  // Reads another folder (null: back to the default), and answers once the
  // new map has gone out. It's a step in the op log (see oplog.js).
  function use(path) {
    note("setRoot", path ?? null);
    return path == null ? call("/api/workspace/root", "DELETE") : call("/api/workspace/root", "POST", { path });
  }

  async function open() {
    const panel = modals.open(
      `<header class="m-head"><div><div class="m-chip">▣ projects folder</div><h2>Change the projects folder</h2><div class="m-sub">Loading…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"></div>`,
      { cls: "rt" },
    );
    const body = panel.querySelector(".m-body");
    let info;
    try {
      info = await get();
    } catch (err) {
      body.innerHTML = `<div class="m-empty">${h(err.message)}</div>`;
      return;
    }
    const sc = info.scan;
    panel.querySelector(".m-sub").textContent = `${tildePath(info.path)} · ${FROM[info.source] ?? info.source}`;
    const now = `
      <div class="d-k">the folder XenoNet reads</div>
      <div class="rt-now"><code>${h(tildePath(info.path))}</code><span class="dim">${h(FROM[info.source] ?? info.source)}</span></div>
      <p class="rt-scan">${sc?.ok ? `${found(sc)} · scanned ${timeAgo(sc.at)}` : sc ? `<span class="bad">XenoNet can't read it.</span> Choose another, or go back to ${h(tildePath(info.default))}.` : "Not scanned yet."}</p>`;
    if (info.source === "env") {
      body.innerHTML = `${now}
        <p class="pj-note">XENONET_PROJECTS_DIR is set where XenoNet started, so it decides which folder XenoNet reads. To choose one here, unset it (in your shell's profile, say) and restart XenoNet.</p>`;
      return;
    }
    body.innerHTML = `${now}
      <div class="d-k">another folder</div>
      <div class="rt-pick">
        <input class="rt-field" data-root-field placeholder="~/code, or the folder's full path" spellcheck="false" autocomplete="off" />
        ${desktop?.chooseFolder ? `<button class="act" data-root-choose>Choose…</button>` : ""}
      </div>
      <div class="rt-answer" data-root-answer></div>
      <p class="pj-note">Your layout, your rules and Codex's picks stay with each folder, so coming back to one brings them back. Sessions carry on, and the ones outside the new folder show as folders outside it.</p>
      <div class="m-foot">
        <button class="act primary" data-root-use disabled>Use this folder</button>
        ${info.source === "chosen" ? `<button class="act" data-root-reset>Back to ${h(tildePath(info.default))}</button>` : ""}
        <span class="grow"></span>
        <span class="pj-foot">Kept in XenoNet's state.json</span>
      </div>`;

    const field = body.querySelector("[data-root-field]");
    const answer = body.querySelector("[data-root-answer]");
    const useButton = body.querySelector("[data-root-use]");
    // Each edit makes the answer in flight stale; `ready` is the folder the
    // last good answer was for, the one Use this folder reads.
    let seq = 0;
    let timer = null;
    let ready = null;

    function show(r, value) {
      ready = r.ok && !r.current ? value : null;
      useButton.disabled = !ready;
      if (!r.ok) answer.innerHTML = `<div class="bad">✗ ${h(r.error)}</div>`;
      else {
        const what = r.current ? "That's the folder XenoNet reads now." : r.scan ? `${found(r.scan)} <span class="dim">· ${plural(r.scan.dirsRead, "folder")} read in ${r.scan.ms} ms</span>` : "XenoNet can read it.";
        answer.innerHTML = `<div class="rt-ok">✓ ${what}</div>${r.warnings.map((w) => `<div class="rt-warn">! ${h(w)}</div>`).join("")}`;
      }
    }

    async function checkNow() {
      const value = field.value.trim();
      const mine = seq;
      if (!value) return;
      answer.innerHTML = `<div class="dim">Checking…</div>`;
      const r = await check(value).catch((err) => ({ ok: false, error: err.message, warnings: [] }));
      if (mine === seq) show(r, value);
    }

    field.addEventListener("input", () => {
      seq++;
      ready = null;
      useButton.disabled = true;
      answer.innerHTML = field.value.trim() ? `<div class="dim">Checking…</div>` : "";
      clearTimeout(timer);
      timer = setTimeout(checkNow, 250);
    });
    field.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !useButton.disabled) useButton.click();
    });
    body.querySelector("[data-root-choose]")?.addEventListener("click", async () => {
      const dir = await desktop.chooseFolder({ defaultPath: info.path }).catch(() => null);
      if (!dir) return;
      field.value = tildePath(dir);
      seq++;
      clearTimeout(timer);
      checkNow();
    });

    async function go(path, button) {
      const label = button.textContent;
      for (const b of body.querySelectorAll("button")) b.disabled = true;
      field.disabled = true;
      button.textContent = path == null ? "Going back…" : "Reading it…";
      try {
        const r = await use(path);
        sfx("success");
        toast(r.changed ? `XenoNet reads ${tildePath(r.path)} now${r.scan?.ok ? `: ${found(r.scan)}` : ""}` : `XenoNet reads ${tildePath(r.path)} already`);
        // Unless you've closed it meanwhile, and maybe opened another.
        if (panel.isConnected) modals.close();
      } catch (err) {
        sfx("error");
        toast(err.message, true);
        for (const b of body.querySelectorAll("button")) b.disabled = false;
        field.disabled = false;
        useButton.disabled = !ready;
        button.textContent = label;
      }
    }
    useButton.addEventListener("click", () => ready && go(ready, useButton));
    body.querySelector("[data-root-reset]")?.addEventListener("click", (e) => go(null, e.currentTarget));
    field.focus();
  }

  return { open, get, check, use };
}
