// Modal panels: collision diffs, handoff briefs, "while you were away", the
// day's log, how the projects folder is read, and your identities. One modal
// is open at a time; Esc closes it.
import { escapeHtml as h, timeAgo, stamp, renderText, tildePath } from "./format.js";
import { APP_LABEL, APP_DESKTOP, colorVar, categoryVar } from "./theme.js";
import { compact } from "./telemetry.js";

const root = () => document.getElementById("modal");

export function createModals({ sfx, toast, select, onPoster, identities = null, onChangeRoot = null, onHandoff = null }) {
  let onClose = null;
  // A close hides and empties the modal once it has faded out. A panel
  // opened before then cancels that, or it would be emptied too.
  let hiding = null;

  // `closing` runs once the panel goes, closed or replaced by another.
  function open(html, { accent = "var(--cyan)", wide = false, cls = "", onClose: closing = null } = {}) {
    const el = root();
    clearTimeout(hiding);
    const was = onClose;
    onClose = null;
    was?.();
    onClose = closing;
    el.innerHTML = `<div class="m-backdrop" data-mclose></div><section class="m-panel ${wide ? "wide" : ""} ${cls}" style="--accent:${accent}">${html}</section>`;
    el.hidden = false;
    requestAnimationFrame(() => el.classList.add("open"));
    sfx("open");
    return el.querySelector(".m-panel");
  }

  function close() {
    const el = root();
    if (el.hidden) return;
    el.classList.remove("open");
    hiding = setTimeout(() => {
      el.hidden = true;
      el.innerHTML = "";
    }, 200);
    sfx("close");
    onClose?.();
    onClose = null;
  }

  const head = (chip, title, sub = "") => `
    <header class="m-head">
      <div><div class="m-chip">${chip}</div><h2>${title}</h2>${sub ? `<div class="m-sub">${sub}</div>` : ""}</div>
      <button class="act" data-mclose>Close ✕</button>
    </header>`;

  root().addEventListener("click", (e) => {
    // An identity filters the map to it (see identities.js); the map's behind this.
    if (e.target.closest("[data-mclose], [data-identity]")) close();
    const go = e.target.closest("[data-mgoto]");
    if (go) {
      close();
      select(go.dataset.mgoto);
    }
  });
  addEventListener(
    "keydown",
    (e) => {
      if (e.key === "Escape" && !root().hidden) {
        e.stopImmediatePropagation();
        close();
      }
    },
    true,
  );

  // ---------- collision diff ----------

  function diffHtml(diff) {
    return `<pre class="diff">${String(diff ?? "")
      .split("\n")
      .map((l) => {
        const cls = /^(\+(?!\+\+)|\+ )/.test(l) ? "add" : /^(-(?!--)|- )/.test(l) ? "del" : /^@@/.test(l) ? "hunk" : "";
        return `<span class="${cls}">${h(l) || " "}</span>`;
      })
      .join("\n")}</pre>`;
  }

  async function collision(a, b) {
    const panel = open(`${head("⚠ collision", "Two sessions edited the same file", "Loading the diffs…")}<div class="m-body"></div>`, { accent: colorVar("collision"), wide: true });
    const data = await fetch(`/api/collision?${new URLSearchParams({ a, b })}`).then((r) => r.json());
    const side = (s, edits) => `
      <div class="col">
        <div class="col-head"><i class="sw ${h(s.id.split(":")[0])}"></i><a href="#" data-mgoto="${h(s.id)}">${h(s.title)}</a>${s.branch ? `<code class="branch">${h(s.branch)}</code>` : ""}</div>
        ${(edits ?? []).map((e) => `<div class="edit-meta">${h(e.kind)} · ${timeAgo(e.ts)}</div>${diffHtml(e.diff)}`).join("")}
      </div>`;
    const sameBranch = data.a.branch && data.a.branch === data.b.branch;
    panel.querySelector(".m-sub").textContent = `${data.files.length} shared file${data.files.length === 1 ? "" : "s"} · ${sameBranch ? `both on ${data.a.branch}` : `${data.a.branch ?? "?"} vs ${data.b.branch ?? "?"}`}`;
    panel.querySelector(".m-body").innerHTML = data.files.length
      ? data.files.map((f) => `<div class="file-row"><div class="file-name">${h(tildePath(f.file))}</div><div class="cols">${side(data.a, f.a)}${side(data.b, f.b)}</div></div>`).join("")
      : `<div class="m-empty">XenoNet keeps edits for 10 minutes, and these are older.</div>`;
  }

  // ---------- handoff ----------

  async function handoff(node, target = node.app === "claude" ? "codex" : "claude") {
    const panel = open(
      `${head("⇄ handoff", h(node.title), `Continue this ${APP_LABEL[node.app]} session in a new one`)}
      <div class="m-body">
        <div class="seg handoff-target">
          <button data-target="claude" class="${target === "claude" ? "on" : ""}"><i class="sw claude"></i>Claude Code</button>
          <button data-target="codex" class="${target === "codex" ? "on" : ""}"><i class="sw codex"></i>Codex</button>
        </div>
        <textarea class="brief" spellcheck="false">Writing the brief…</textarea>
        <div class="m-foot"><span class="count"></span><span class="grow"></span><button class="act" data-copy-brief>Copy</button><button class="act primary" data-send>Send</button></div>
      </div>`,
      { accent: colorVar(target) },
    );
    const ta = panel.querySelector(".brief");
    const count = panel.querySelector(".count");
    const send = panel.querySelector("[data-send]");
    const load = async () => {
      ta.value = "Writing the brief…";
      const r = await fetch(`/api/handoff?${new URLSearchParams({ nodeId: node.id, target })}`).then((x) => x.json());
      ta.value = r.brief ?? r.error ?? "";
      update();
    };
    const update = () => {
      count.textContent = `${ta.value.length} / 6000`;
      send.textContent = `Send to ${APP_DESKTOP[target]}`;
      send.className = `act primary ${target}`;
      panel.style.setProperty("--accent", colorVar(target));
    };
    ta.addEventListener("input", update);
    panel.addEventListener("click", async (e) => {
      const t = e.target.closest("[data-target]");
      if (t) {
        target = t.dataset.target;
        panel.querySelectorAll("[data-target]").forEach((b) => b.classList.toggle("on", b === t));
        sfx("toggle");
        await load();
      }
      if (e.target.closest("[data-copy-brief]")) {
        await navigator.clipboard?.writeText(ta.value);
        toast("Brief copied");
        sfx("select");
      }
      if (e.target.closest("[data-send]")) {
        const res = await fetch("/api/handoff", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ nodeId: node.id, target, brief: ta.value }) });
        if (res.ok) {
          sfx("success");
          toast(`Sent to ${APP_DESKTOP[target]}`);
          onHandoff?.(node, target);
          close();
        } else {
          sfx("error");
          toast("Couldn't send the brief", true);
        }
      }
    });
    await load();
  }

  // ---------- digests ----------

  function sessionRows(sessions) {
    return sessions
      .slice(0, 10)
      .map((s) => {
        const tools = Object.entries(s.tools)
          .sort((a, b) => b[1] - a[1])
          .slice(0, 4)
          .map(([k, v]) => `<span class="t" style="--c:${categoryVar(k)}">${h(k)} ${h(v)}</span>`)
          .join("");
        return `<li data-mgoto="${h(s.id)}">
          <i class="sw ${h(s.app)}"></i>
          <div class="d-main">
            <div class="d-title">${h(s.title)}${s.needsYou ? `<span class="tag need">${s.needsYou.kind === "review" ? "ready for review" : "waiting for you"}</span>` : ""}</div>
            <div class="d-meta">${h(s.turns)} turn${s.turns === 1 ? "" : "s"} · ${h(s.toolCalls)} tool calls${s.errors ? ` · <b class="bad">${h(s.errors)} errors</b>` : ""}${s.files.length ? ` · ${h(s.files.slice(0, 4).join(", "))}${s.files.length > 4 ? ` +${s.files.length - 4}` : ""}` : ""}</div>
            <div class="d-tools">${tools}</div>
          </div>
        </li>`;
      })
      .join("");
  }

  function totalsHtml(t, extra = []) {
    const cell = (v, k) => `<div><span>${h(v)}</span><small>${k}</small></div>`;
    return `<div class="d-totals">${cell(t.sessions, "sessions")}${cell(t.turns, "turns")}${cell(compact(t.toolCalls), "tool calls")}${cell(t.files, "files edited")}${cell(t.errors, "errors")}${extra.join("")}</div>`;
  }

  function narrateBlock() {
    return `<div class="narrate"><button class="act" data-narrate>✦ Summarize with Codex</button><div class="narration" hidden></div></div>`;
  }

  function wireNarrate(panel, body) {
    panel.querySelector("[data-narrate]")?.addEventListener("click", async (e) => {
      const btn = e.currentTarget;
      const out = panel.querySelector(".narration");
      btn.disabled = true;
      btn.textContent = "Codex is writing…";
      out.hidden = false;
      out.classList.add("loading");
      out.textContent = "";
      const res = await fetch("/api/narrate", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json()).catch((err) => ({ error: String(err) }));
      out.classList.remove("loading");
      out.innerHTML = res.text ? renderText(res.text) : `<span class="bad">${h(res.error ?? "Codex didn't answer")}</span>`;
      btn.textContent = "✦ Summarize again";
      btn.disabled = false;
      sfx(res.text ? "success" : "error");
    });
  }

  async function away(since, { label } = {}) {
    const d = await fetch(`/api/digest?since=${since}`).then((r) => r.json());
    const mins = Math.round((Date.now() - since) / 60e3);
    const dur = mins >= 90 ? `${Math.floor(mins / 60)}h ${mins % 60}m` : `${mins}m`;
    const waiting = d.sessions.filter((s) => s.needsYou && s.needsYou.kind !== "review");
    const panel = open(
      `${head("◷ while you were away", label ?? `The last ${dur}`, `from ${stamp(since)} to now`)}
      <div class="m-body">
        ${totalsHtml(d.totals)}
        ${waiting.length ? `<div class="d-k need">needs you first</div><ol class="d-list">${sessionRows(waiting)}</ol>` : ""}
        <div class="d-k">what your sessions did</div>
        <ol class="d-list">${sessionRows(d.sessions.filter((s) => !waiting.includes(s))) || `<li class="m-empty">Nothing ran.</li>`}</ol>
        ${narrateBlock()}
      </div>`,
      { accent: colorVar("review") },
    );
    wireNarrate(panel, { kind: "away", since });
    return d;
  }

  async function dayLog() {
    const d = await fetch("/api/daylog").then((r) => r.json());
    const max = Math.max(1, ...d.hours);
    const bars = d.hours.map((v, i) => `<i style="height:${Math.round((v / max) * 100)}%" class="${i === d.busiestHour ? "peak" : ""}" title="${i}:00 · ${h(v)} tool calls"></i>`).join("");
    const span = d.firstAt ? `${new Date(d.firstAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} to ${new Date(d.lastAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "no activity yet";
    const panel = open(
      `${head("☾ day's log", h(d.date), span)}
      <div class="m-body">
        ${totalsHtml(d.totals, [`<div><span>${compact(d.tokens)}</span><small>tokens</small></div>`])}
        <div class="d-k">activity by hour${d.busiestHour != null ? ` · busiest ${d.busiestHour}:00` : ""}</div>
        <div class="hours">${bars}</div><div class="hours-axis"><span>0</span><span>6</span><span>12</span><span>18</span><span>24</span></div>
        ${d.topProjects.length ? `<div class="d-k">busiest projects</div><div class="top-projects">${d.topProjects.map((p) => `<a href="#" data-mgoto="${h(p.id)}">${h(p.name)} <small>${h(p.toolCalls)}</small></a>`).join("")}</div>` : ""}
        ${d.uncommitted.length ? `<div class="d-k ship">uncommitted at end of day</div><div class="top-projects">${d.uncommitted.map((r) => `<span class="ship">${h(r.name)} <small>Δ${h(r.dirty)}${r.ahead ? ` ↑${h(r.ahead)}` : ""}</small></span>`).join("")}</div>` : ""}
        ${d.waiting ? `<div class="d-k need">${h(d.waiting)} session${d.waiting === 1 ? "" : "s"} still waiting for you</div>` : ""}
        <div class="d-k">sessions</div>
        <ol class="d-list">${sessionRows(d.sessions)}</ol>
        <div class="m-foot">${narrateBlock()}<span class="grow"></span><button class="act" data-poster>▣ Make a poster</button></div>
      </div>`,
      { accent: colorVar("wiki"), wide: true },
    );
    wireNarrate(panel, { kind: "day" });
    panel.querySelector("[data-poster]")?.addEventListener("click", () => {
      close();
      onPoster?.();
    });
    return d;
  }

  // ---------- how the projects folder is read ----------
  //
  // The last scan, your rules (each with an undo), and the optional Codex
  // pass: first it shows everything Codex would get, and only a click on
  // Send sends it.

  const AS_LABEL = { project: "one project", group: "a group of projects", ignore: "left out" };
  const api = (url, method = "GET", body) => fetch(url, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined }).then(async (r) => ({ ok: r.ok, ...(await r.json().catch(() => ({}))) }));

  async function projects({ showOutline = false } = {}) {
    const panel = open(`${head("▣ projects folder", "How XenoNet finds your projects", "Loading…")}<div class="m-body"></div>`, { accent: colorVar("project"), wide: true });
    const body = panel.querySelector(".m-body");
    let d = await api("/api/projects");
    let outline = null;

    const outlineHtml = (o) => `
      <div class="d-k">what Codex${o.decider?.key === "stored" ? " or the decision model" : ""} would get · ${o.outline.length} folders</div>
      <p class="pj-note">Each folder's path inside your projects folder, the well-known files in it, how many files and folders it holds, its commonest file types, how many sessions ran there, and XenoNet's own call. No other file names, nothing from inside a file, no transcripts.</p>
      <div class="pj-scroll"><table class="pj-table"><thead><tr><th>folder</th><th>XenoNet's call</th><th>well-known files</th><th>files · folders</th><th>types</th><th>sessions</th></tr></thead><tbody>
        ${o.outline.map((f) => `<tr><td><code>${h(f.path)}</code></td><td>${h(f.xenonet)}</td><td>${h(f.markers.join(" "))}</td><td>${f.files == null ? "" : `${f.files} · ${f.folders}`}</td><td>${h(Object.entries(f.types).map(([k, n]) => `${k}:${n}`).join(" "))}</td><td>${f.sessions || ""}</td></tr>`).join("")}
      </tbody></table></div>
      <div class="m-foot"><button class="act primary codex" data-pj-send>Send to Codex</button>${o.decider?.key === "stored" ? `<button class="act" data-pj-send data-pj-via="decider" title="One choice a folder for ${h(o.decider.model)} on OpenRouter, with zero data retention asked; it keeps only the picks it's sure of">Send to ${h(String(o.decider.model).replace(/^~?[^/]+\//, "").replace(/-latest$/, ""))}</button>` : ""}<button class="act" data-pj-hide>Don't send</button><span class="grow"></span><span class="pj-foot">${o.demo ? "Demo mode: a made-up answer, nothing is sent." : `Codex runs codex exec, read-only and ephemeral, like Summarize with Codex.${o.decider?.key === "stored" ? " The decision model gets the same, through OpenRouter." : ""}`}</span></div>`;

    function render() {
      const sc = d.scan;
      panel.querySelector(".m-sub").textContent = sc?.ok ? `${tildePath(d.root)} · ${sc.projects} projects in ${sc.groups} group${sc.groups === 1 ? "" : "s"} · ${sc.dirsRead} folders read in ${sc.ms} ms · ${sc.depth} levels deep · scanned ${timeAgo(sc.at)}` : "The projects folder couldn't be read";
      const c = d.codex;
      body.innerHTML = `
        ${sc?.truncated ? `<p class="pj-note bad">The scan stopped at its ${sc.maxDirs}-folder limit, so folders further down may be missing. Raise XENONET_PROJECTS_MAX_DIRS, or leave big folders out.</p>` : ""}
        <p class="pj-note">A folder with its own repo is a project. A folder that holds projects is a group, drawn as a hollow frame with a <code>/</code> after its name. A top-level folder, a workspace, or a folder in a group where a session ran is a project too. Your rules come first, then Codex's picks.</p>
        <div class="d-k">your rules</div>
        ${d.rules.length ? `<ol class="d-list pj-rules">${d.rules.map((r) => `<li><code>${h(tildePath(r.path))}</code><span>${h(AS_LABEL[r.as] ?? r.as)}</span><span class="dim">${r.inRoot === false ? "outside this projects folder" : timeAgo(r.at)}</span><button class="act" data-pj-undo="${h(r.path)}">Undo</button></li>`).join("")}</ol>` : `<div class="m-empty">None yet. Select a project, a group or a session, then use the palette to make a folder one project, look inside it, or leave it out.</div>`}
        <div class="d-k">Codex's picks for this folder</div>
        ${
          c
            ? `<p class="pj-note">Asked ${timeAgo(c.ranAt)}, about ${c.sent} folders. ${c.picks.length ? `${c.picks.length} pick${c.picks.length === 1 ? "" : "s"}:` : "It agreed with XenoNet everywhere."}${c.stale ? ` <span class="bad">The folders have changed since. Ask again to update.</span>` : ""}</p>
               ${c.picks.length ? `<ol class="d-list pj-rules">${c.picks.map((p) => `<li><code>${h(p.path)}</code><span>${h(AS_LABEL[p.as])}</span><span class="dim">${h(p.reason)}</span></li>`).join("")}</ol>` : ""}`
            : `<p class="pj-note">Not asked. For folders with no repo and no sessions, XenoNet can only guess. Codex can look at an outline of them and say which are really one project, or a group of separate ones. Its picks never hide anything, and your rules win over them.</p>`
        }
        <div class="pj-outline">${outline ? outlineHtml(outline) : ""}</div>
        <div class="m-foot">
          ${outline ? "" : `<button class="act" data-pj-outline>${c ? "Ask Codex again…" : "Ask Codex which folders are projects…"}</button>`}
          ${c ? `<button class="act" data-pj-forget>Forget Codex's picks</button>` : ""}
          <span class="grow"></span>
          ${onChangeRoot ? `<button class="act" data-pj-root>Change the folder…</button>` : ""}
          ${d.rules.some((r) => r.inRoot !== false) ? `<button class="act" data-pj-clear title="Your rules for folders in ${h(tildePath(d.root))}; other folders keep theirs">Clear your rules</button>` : ""}
          ${d.rules.some((r) => r.inRoot === false) ? `<button class="act" data-pj-clear-all title="Your rules for every projects folder, this one and the others">Clear rules for every folder</button>` : ""}
          <button class="act" data-pj-rescan>Detect projects again</button>
        </div>`;
    }

    async function act(work, done) {
      const res = await work();
      if (!res.ok) {
        sfx("error");
        toast(res.error ?? "That didn't work", true);
        return res;
      }
      d = res.scan ? res : d;
      sfx("success");
      if (done) toast(typeof done === "function" ? done(res) : done);
      render();
      return res;
    }

    body.addEventListener("click", async (e) => {
      const t = e.target.closest("button");
      if (!t || t.disabled) return;
      if (t.dataset.pjUndo) await act(() => api("/api/projects/rule", "POST", { path: t.dataset.pjUndo, as: null }), "Rule removed");
      // Only this folder's; other folders keep theirs unless asked.
      else if (t.matches("[data-pj-clear]")) await act(() => api("/api/projects/rules", "DELETE"), "Your rules for this folder are cleared");
      else if (t.matches("[data-pj-clear-all]")) await act(() => api("/api/projects/rules?all=1", "DELETE"), "Your rules for every folder are cleared");
      else if (t.matches("[data-pj-rescan]")) await act(() => api("/api/projects/rescan", "POST"), (r) => `Found ${r.scan.projects} projects in ${r.scan.groups} group${r.scan.groups === 1 ? "" : "s"}`);
      else if (t.matches("[data-pj-forget]")) await act(() => api("/api/projects/codex", "DELETE"), "Codex's picks are forgotten");
      // In place of this panel (see root.js).
      else if (t.matches("[data-pj-root]")) onChangeRoot();
      else if (t.matches("[data-pj-hide]")) {
        outline = null;
        render();
      } else if (t.matches("[data-pj-outline]")) {
        t.disabled = true;
        outline = await api("/api/projects/codex");
        render();
      } else if (t.matches("[data-pj-send]")) {
        const via = t.dataset.pjVia ?? null;
        for (const b of body.querySelectorAll("[data-pj-send], [data-pj-hide]")) b.disabled = true;
        t.textContent = via ? "The decision model is sorting the folders…" : "Codex is sorting the folders…";
        const who = via ? "The decision model" : "Codex";
        const res = await act(() => api("/api/projects/codex", "POST", { fingerprint: outline.fingerprint, via }), (r) => `${who} made ${r.codex.picks.length} pick${r.codex.picks.length === 1 ? "" : "s"}`);
        if (res.ok) outline = null;
        render();
      }
    });

    if (showOutline && d.scan) outline = await api("/api/projects/codex");
    render();
    return d;
  }

  // ---------- your identities ----------
  //
  // Every identity XenoNet found, where it read it and what it reaches. Here
  // you name them (the same name makes several one), hide ones you don't
  // care about, and undo pins. While identities are hidden, so is everything
  // that would show one, and naming waits.

  const SECTION = { claude: "Claude accounts", chatgpt: "ChatGPT accounts · Codex", openai: "OpenAI API", github: "GitHub accounts", git: "git identities" };
  let renderIdentities = null;

  async function identitiesWindow() {
    const panel = open(`${head("◈ identities", "Who your work runs as", "Loading…")}<div class="m-body"></div>`, { accent: "var(--ring-0)", wide: true });
    const body = panel.querySelector(".m-body");
    let d = await api("/api/identities");
    const ids = identities;

    function render() {
      const m = ids.masked;
      const known = new Map(d.identities.map((i) => [i.id, i]));
      const name = (id) => (m || !known.get(id) ? ids.label(id) : (known.get(id).name ?? ids.handleOf(known.get(id))));
      panel.querySelector(".m-sub").textContent = d.enabled ? `${d.identities.length} found · read ${timeAgo(d.at)} · nothing here leaves this computer${m ? " · hidden" : ""}` : "Off: XENONET_IDENTITIES=off";
      const row = (i) => `
        <li class="${i.hidden ? "off" : ""}">
          <span class="id-who">${ids.glyph(i)}<span><span class="id-handle">${h(m ? ids.label(i.id) : ids.handleOf(i))}</span><span class="dim">${h(ids.detailOf(i))}</span></span></span>
          <span class="dim">${h(i.sources.join(" · "))}</span>
          <span class="dim">${h(i.sessions)} session${i.sessions === 1 ? "" : "s"} · ${h(i.projects)} project${i.projects === 1 ? "" : "s"}${i.firstSeen ? ` · first seen ${timeAgo(i.firstSeen)}` : ""}</span>
          <input class="id-name-input" data-id-name="${h(i.id)}" value="${m ? "" : h(i.name ?? "")}" placeholder="${m ? "hidden" : "name it"}" maxlength="40" spellcheck="false" ${m ? "disabled" : ""} title="Identities with the same name show as one, and their projects stand on a ring in its color" />
          <button class="act" data-id-hide="${h(i.id)}" title="${i.hidden ? "Give it a chip under Filters again" : "No chip under Filters and no ring on the map; its links stay"}">${i.hidden ? "Show" : "Hide"}</button>
        </li>`;
      const kinds = Object.keys(SECTION).filter((k) => d.identities.some((i) => i.kind === k));
      const warnings = ids.allMismatches();
      const spans = Object.entries(d.signins).filter(([, list]) => list.length);
      body.innerHTML = `
        <p class="pj-note">A session's account comes from the app's own record where it keeps one: Claude desktop's session folders and Remote Control transcripts. Otherwise it's whoever was signed in when the session last ran, from Codex's sign-in time and the sign-ins XenoNet has seen. A project's come from its origin remote and its git config, includeIf rules and all. Click one to show only its work.</p>
        ${warnings.length ? `<div class="d-k need">worth a look</div><ol class="d-list pj-rules id-warns">${warnings.map((w) => `<li data-mgoto="${h(w.id)}"><span>${h(w.name)}</span><span class="id-warn-t">⚠ ${h(w.texts.join("; "))}</span></li>`).join("")}</ol>` : ""}
        ${kinds.map((k) => `<div class="d-k">${SECTION[k]}</div><ol class="d-list id-list">${d.identities.filter((i) => i.kind === k).map(row).join("")}</ol>`).join("") || `<div class="m-empty">${d.enabled ? "None found yet." : "XenoNet isn't reading identities. Unset XENONET_IDENTITIES to turn them on."}</div>`}
        <div class="d-k">your pins</div>
        ${d.pins.length ? `<ol class="d-list pj-rules">${d.pins.map((p) => `<li><span>${h(p.title ?? "a session or project that's gone")}</span><span>→ ${h(name(p.identity))}</span><span class="dim">${timeAgo(p.at)}</span><button class="act" data-id-unpin="${h(p.node)}">Undo</button></li>`).join("")}</ol>` : `<div class="m-empty">None. Select a session or a project, then use the palette to say which account it ran as or belongs to.</div>`}
        ${spans.length ? `<div class="d-k">sign-ins XenoNet has seen</div><ol class="d-list pj-rules id-signins">${spans.map(([app, list]) => list.slice().reverse().map((sp, k) => `<li><span>${h(app === "claude" ? "Claude" : "ChatGPT")}</span><span>${h(name(sp.id))}</span><span class="dim">${k === 0 ? `since ${timeAgo(sp.from)}` : `${timeAgo(sp.from)} to ${timeAgo(sp.to)}`}</span><span></span></li>`).join("")).join("")}</ol>` : ""}
        <div class="m-foot">
          <button class="act" data-id-mask>${m ? "Show identities" : "Hide identities"}</button>
          <span class="grow"></span>
          <span class="pj-foot">Names and pins stay in XenoNet's state.json</span>
          <button class="act" data-id-rescan>Detect identities again</button>
        </div>`;
    }

    async function act(work, done) {
      const res = await work();
      if (!res.ok) {
        sfx("error");
        toast(res.error ?? "That didn't work", true);
        return;
      }
      d = res;
      sfx("success");
      if (done) toast(done);
      render();
    }
    const saveName = (input) => {
      const value = input.value.trim();
      if (value === (d.identities.find((i) => i.id === input.dataset.idName)?.name ?? "")) return;
      act(() => api("/api/identities/name", "POST", { id: input.dataset.idName, name: value || null }), value ? `Named ${value}` : "Name removed");
    };
    body.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && e.target.matches("[data-id-name]")) e.target.blur();
    });
    body.addEventListener("focusout", (e) => e.target.matches?.("[data-id-name]") && saveName(e.target));
    body.addEventListener("click", async (e) => {
      const t = e.target.closest("button");
      if (!t || t.disabled) return;
      if (t.dataset.idHide) {
        const hidden = !d.identities.find((i) => i.id === t.dataset.idHide)?.hidden;
        await act(() => api("/api/identities/hidden", "POST", { id: t.dataset.idHide, hidden }), hidden ? "Hidden from the filters and the map" : "Shown again");
      } else if (t.dataset.idUnpin) await act(() => api("/api/identities/pin", "POST", { nodeId: t.dataset.idUnpin, identity: null }), "Pin removed");
      else if (t.matches("[data-id-rescan]")) {
        t.disabled = true;
        await act(() => api("/api/identities/rescan", "POST"), "Read every identity again");
      } else if (t.matches("[data-id-mask]")) ids.toggleMask();
    });

    renderIdentities = render;
    onClose = () => (renderIdentities = null);
    render();
    return d;
  }

  return {
    open,
    close,
    collision,
    handoff,
    away,
    dayLog,
    projects,
    identities: identitiesWindow,
    // Identities masked or renamed: the window again, if it's open.
    refreshIdentities: () => !document.activeElement?.matches?.("[data-id-name]") && renderIdentities?.(),
    get isOpen() {
      return !root().hidden;
    },
  };
}
