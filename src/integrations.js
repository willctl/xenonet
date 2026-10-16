import { escapeHtml as h, tildePath } from "./format.js";
import "./integrations.css";

// Sending a decision or launching a job stays in the owner's window. The
// console driver exposes browsing and form preparation, never execution.
export function createIntegrationsPanel({ modals, calls, savePatch }) {
  let panel = null, tab = "routing", data = null, selected = null, config = null, ticket = null, revision = 0, busy = false, writable = false;
  let filters = { q: "", status: "", sort: "newest", page: 1 };
  let review = null, reviewFile = null, reviewMode = false;
  const titles = { routing: "Route task", sandbox: "Launch sandbox", pi: "Review Pi runs" };
  const el = (s) => panel?.querySelector(s);
  const message = (s) => { if (el("[data-in-message]")) el("[data-in-message]").textContent = s; };
  const state = () => ({ open: !!panel, tab, busy, writable, preview: !!ticket, filters: { ...filters }, selected, total: data?.total ?? 0, matched: data?.matched ?? 0, page: data?.page ?? 1, pages: data?.pages ?? 0, records: data?.records ?? [], review: review ? { available: review.available, reason: review.reason, baseline: review.baseline && { id: review.baseline.id, digest: review.baseline.digest, bytes: review.baseline.bytes, files: review.baseline.files.length, excluded: review.baseline.excluded }, result: review.result && { revision: review.result.revision, at: review.result.at, complete: review.result.complete, sourceState: review.result.sourceState, execution: review.result.execution, patchHash: review.result.patchHash, problems: review.result.problems, omissions: review.result.omissions, files: review.result.changes.map(({ diff, ...f }) => f) }, file: reviewFile } : null, error: el("[data-in-message]")?.textContent ?? "" });
  async function write(route, body) {
    if (!writable) throw new Error("Use XenoNet's own window for this action.");
    const own = panel;
    const r = await calls.call("POST", `/api/integrations${route}`, body);
    if (own !== panel) throw new Error("The panel closed. Refresh to see the operation's result.");
    if (!r.ok) throw new Error(r.error || "XenoNet did not answer.");
    return r.data;
  }
  async function get(route) {
    const r = await fetch(`/api/integrations${route}`);
    const body = await r.json();
    if (!r.ok) throw new Error(body.error || "XenoNet did not answer.");
    return body;
  }
  function lock() {
    panel?.querySelectorAll("[data-in-write]").forEach((b) => { b.disabled = busy || !writable; b.setAttribute("data-yours", ""); });
    if (el("[data-in-confirm]")) el("[data-in-confirm]").disabled = busy || !writable || !ticket;
  }
  async function work(fn) {
    if (busy) return;
    const own = panel;
    busy = true; lock(); message("");
    try { await fn(); } catch (e) { if (panel === own) message(e.message); }
    finally { if (panel === own) { busy = false; lock(); } }
  }
  function field(name, label, value = "", type = "text") { return `<label>${label}<input name="${name}" type="${type}" value="${h(value)}" data-in-draft autocomplete="off"></label>`; }
  function form() {
    const own = writable ? "" : "<p>Use XenoNet's own window to send decisions, launch sandboxes or read a new source.</p>";
    if (tab === "routing") return `${own}<p>Model: ${h(config.decision.model)} · ${config.decision.key === "missing" ? "No key stored. Set the OpenRouter key for decision models from the command palette." : "Decision provider ready."} Suggestions never start an agent.</p><label>Task<textarea name="request" maxlength="2000" data-in-draft placeholder="Describe the task without credentials or private transcript excerpts."></textarea></label><button class="act" data-in-preview data-in-write>Preview what will be sent</button><button class="act primary" data-in-confirm data-in-write disabled>Send to decision model</button>`;
    if (tab === "sandbox") return `${own}<p>Runs in an existing OpenShell gateway. Windows uses experimental WSL support. The image must contain the selected agent and providers must already be configured.</p><div class="in-fields"><label>Execution environment<select name="backend" data-in-draft><option value="wsl" ${config.platform === "win32" ? "selected" : ""}>WSL</option><option value="native" ${config.platform !== "win32" ? "selected" : ""}>Native Linux / macOS CLI</option></select></label>${field("distro", "WSL distribution", config.profiles?.[0]?.distro ?? "")}<label>Agent<select name="agent" data-in-draft><option value="pi">Pi</option><option value="codex">Codex</option><option value="claude">Claude Code</option></select></label>${field("image", "Container image", "xenonet-pi:1.2.0")}${field("provider", "Existing provider (optional)")}${field("source", "Project folder (optional)")}${field("policy", "Policy YAML file (optional)")}</div><label class="in-check"><input type="checkbox" name="upload" data-in-draft> Allow the selected project to be uploaded to the gateway</label><label class="in-check"><input type="checkbox" name="review" data-in-draft> Freeze a text-only Git snapshot for change review and patch export</label><p>Text review needs Node at /usr/local/bin/node in the image. It keeps the sandbox running for retrieval until you delete it. Review the upload manifest before launching.</p><label>Task<textarea name="request" maxlength="2000" data-in-draft></textarea></label><button class="act" data-in-check data-in-write>Check gateway</button><button class="act" data-in-preview data-in-write>Preview launch</button><button class="act primary" data-in-confirm data-in-write disabled>Launch reviewed sandbox</button>`;
    return `${own}<p>Read-only metadata from Native Pi sessions and pi-harness JSON run exports. Prompts, tool arguments and tool output are omitted. Pending results are not treated as proof of a stall.</p><p>Sessions: ${h(tildePath(config.piRoot))} · ${config.piOn ? "reading enabled" : "reading off"}</p><button class="act" data-in-enable data-in-write>${config.piOn ? "Stop reading Pi sessions" : "Read Pi sessions"}</button><label>Import a pi-harness JSON run export<input type="file" accept=".json,application/json" data-in-import data-in-write></label>`;
  }
  function renderDetail() {
    const r = data?.records.find((r) => r.id === selected);
    panel?.querySelectorAll("[data-in-id]").forEach((button) => button.classList.toggle("selected", button.dataset.inId === selected));
    const detail = el("[data-in-detail]");
    if (!detail) return;
    if (!r) { detail.textContent = "Select a record to review it."; return; }
    const row = (label, value) => value === null || value === undefined || value === "" ? "" : `<dt>${h(label)}</dt><dd>${h(value)}</dd>`;
    const common = `${row("Status", r.status)}${row("Model", r.model)}${row("Provider", r.provider)}${row("Cost", r.cost == null ? null : `$${r.cost.toFixed(5)}`)}${row("Decision time", r.ms == null ? null : `${r.ms} ms`)}`;
    const specific = r.kind === "sandbox" ? `${row("Sandbox", r.name)}${row("Agent", r.agent)}${row("Image", r.image)}${row("Project uploaded", r.source ? tildePath(r.source) : "None")}${row("Execution environment", r.profile?.backend === "wsl" ? `WSL · ${r.profile.distro}` : "Native CLI")}` : `${row("Run", r.externalId)}${row("Tokens", r.tokens)}${row("Tool calls", r.tools)}${row("Failed results", r.failures)}${row("Pending results", r.pending)}${row("Checkpoints", r.checkpoints)}${row("Artifacts", r.artifacts)}${row("Project", r.cwd ? tildePath(r.cwd) : null)}`;
    const scores = r.kind === "decision" ? `<div class="in-scores">${r.candidates.map((c) => `<div><strong>${h(c.name)}</strong><span>${c.probability == null ? "No score" : `${(c.probability * 100).toFixed(1)}%`}</span><progress max="1" value="${Number(c.probability) || 0}" aria-label="${h(c.name)} probability"></progress><small>${c.filtered ? `Excluded: ${h(c.reason)}` : c.id === r.selected ? "Suggested · awaits your choice" : "Eligible"}</small></div>`).join("")}</div>` : "";
    detail.innerHTML = `<h3>${h(r.kind === "decision" ? "Routing decision" : r.kind === "sandbox" ? "Sandbox job" : "Pi run")}</h3><dl>${common}${r.kind === "decision" ? row("Suggestion", r.selected || "No decision") : specific}${r.kind === "sandbox" ? row("Execution", "Unknown · gateway phase does not establish task completion") + row("Launch steps completed", r.launchSteps ? `${r.completedSteps} / ${r.launchSteps}` : null) + row("Last retrieval attempt", r.retrieval) : ""}</dl>${scores}${r.reason ? `<p>${h(r.reason)}</p>` : ""}${r.note ? `<p>${h(r.note)}</p>` : ""}${r.kind === "decision" ? `<label>Recorded outcome<select data-in-outcome data-in-write>${["unreviewed", "accepted", "rejected", "useful", "unhelpful"].map((v) => `<option ${r.outcome === v ? "selected" : ""}>${v}</option>`).join("")}</select></label><p>Scores are provider evidence. No agent has been started by this decision.</p>` : r.kind === "sandbox" ? `${r.status !== "deleted" && r.status !== "not-created" ? '<button class="act" data-in-inspect data-in-write>Inspect status, policy and logs</button><button class="act" data-in-delete-preview data-in-write>Review deletion</button><div data-in-inspection></div>' : ""}${r.reviewId ? `<button class="act" data-in-review>Open local review</button>${r.status !== "deleted" && r.status !== "not-created" ? '<button class="act" data-in-retrieve data-in-write>Retrieve text changes</button>' : ""}<button class="act" data-in-forget-preview data-in-write>Forget local review…</button><p>The local baseline and retrieved changes remain here after sandbox deletion until you forget them.</p><section data-in-review-body></section>` : "<p>This job has no frozen text baseline for review.</p>"}` : ""}`;
    if (r.kind === "sandbox" && review?.record === r.id) renderReview();
    lock();
  }
  function renderReview() {
    const body = el("[data-in-review-body]"); if (!body) return;
    if (!review?.available) { body.textContent = review?.reason ?? "Open or retrieve this job's local review."; return; }
    const { baseline, result } = review;
    if (!result) { body.textContent = `Baseline ${baseline.digest.slice(0, 12)} · ${baseline.files.length} text files · ${baseline.bytes} bytes. No changes retrieved yet.`; return; }
    const selectedFile = result.changes.find((f) => f.path === reviewFile) ?? result.changes[0]; reviewFile = selectedFile?.path ?? null;
    body.innerHTML = `<h4>Retrieved text snapshot</h4><p>${h(new Date(result.at).toLocaleString())} · baseline <code>${h(baseline.digest.slice(0, 12))}</code></p><p>Source at retrieval: <strong>${h(result.sourceState)}</strong>. Execution: unknown. File modes are outside this review.</p>${result.sourceState !== "unchanged" ? "<p>The patch targets the frozen upload, which may differ from your current project.</p>" : ""}<p>${result.complete ? result.changes.length ? `${result.changes.length} changed text files` : "No text changes in the selected scope." : "Retrieval is incomplete. Export is unavailable."}</p>${result.problems.length ? `<ul>${result.problems.map((p) => `<li>${h(p.path)}: ${h(p.reason)}</li>`).join("")}</ul>` : ""}<details><summary>Excluded scope (${baseline.excluded.length} at upload, ${result.omissions.length} at retrieval)</summary><ul>${[...baseline.excluded, ...result.omissions].map((p) => `<li>${h(p.path)}: ${h(p.reason)}</li>`).join("")}</ul><p>Ignored files and .git are not uploaded. Sensitive-name rules are a precaution; review the manifest yourself.</p></details><div class="in-review-files" aria-label="Changed text files">${result.changes.map((f) => `<button class="act" data-in-review-file="${h(f.path)}" aria-pressed="${f.path === reviewFile}">${h(f.path)} · ${h(f.status)}</button>`).join("")}</div><pre data-in-diff aria-label="Read-only text diff">${h(selectedFile?.diff ?? "")}</pre><p>Patch SHA-256: <code>${h(result.patchHash)}</code></p>${result.complete && result.changes.length ? '<button class="act primary" data-in-export data-in-write>Export reviewed patch…</button>' : ""}`;
    lock();
  }
  async function openReview() {
    const record = selected, current = await get(`/sandbox/review?${new URLSearchParams({ id: record })}`);
    if (selected !== record) return;
    review = { ...current, record }; reviewFile = null; renderReview();
  }
  async function refresh() {
    const current = ++revision, own = panel;
    const result = await get(`${tab === "pi" ? "/pi" : "/history"}?${new URLSearchParams({ ...filters, ...(tab === "pi" ? {} : { kind: tab === "sandbox" ? "sandbox" : "decision" }) })}`);
    if (current !== revision || own !== panel || !panel) return;
    data = result;
    el("[data-in-list]").innerHTML = result.records.map((r) => `<button class="in-record ${r.id === selected ? "selected" : ""}" data-in-id="${h(r.id)}"><strong>${h(r.selected || r.name || r.externalId || r.kind)}</strong><span>${h(r.status)}${r.outcome ? ` · ${h(r.outcome)}` : ""}</span><small>${h(new Date(r.at).toLocaleString())}</small></button>`).join("") || "<p>No records match these filters.</p>";
    el("[data-in-count]").textContent = `${result.matched} of ${result.total} records${result.warnings?.length ? ` · ${result.warnings.join(" ")}` : ""}`;
    el("[data-in-page]").textContent = result.pages ? `Page ${result.page} of ${result.pages}` : "";
    el("[data-in-prev]").disabled = result.page <= 1;
    el("[data-in-next]").disabled = result.page >= result.pages;
    renderDetail();
  }
  function draft() {
    return Object.fromEntries([...panel.querySelectorAll("[data-in-draft]")].map((input) => [input.name, input.type === "checkbox" ? input.checked : input.value]));
  }
  function invalidate() { ticket = null; if (el("[data-in-preview-body]")) el("[data-in-preview-body]").textContent = ""; lock(); }
  async function preview() {
    const before = JSON.stringify(draft());
    const result = await write(tab === "routing" ? "/route/preview" : "/sandbox/preview", draft());
    if (before !== JSON.stringify(draft())) { message("The form changed. Preview again."); return; }
    ticket = result.ticket;
    el("[data-in-preview-body]").textContent = `${result.note}\n\n${JSON.stringify(result.outgoing ?? result.commands ?? result.command, null, 2)}${result.baseline ? `\n\nFrozen upload manifest:\n${JSON.stringify(result.baseline, null, 2)}` : ""}${result.policy ? `\n\nPolicy file:\n${result.policy}` : ""}`;
  }
  async function open(mode = "routing") {
    reviewMode = mode === "results"; tab = reviewMode ? "sandbox" : mode in titles ? mode : "routing"; filters = { q: "", status: "", sort: "newest", page: 1 }; data = null; selected = null; ticket = null; busy = false; review = null; reviewFile = null;
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">Agent workflows</div><h2>${reviewMode ? "Review sandbox changes" : titles[tab]}</h2><div class="m-sub">Decisions, execution and recorded outcomes.</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body in-body"><nav>${Object.entries(titles).map(([id, label]) => `<button class="act" data-in-tab="${id}" ${id === tab ? 'aria-current="page"' : ""}>${label}</button>`).join("")}</nav><p data-in-message role="status" aria-live="polite"></p><section data-in-form></section><pre data-in-preview-body aria-live="polite"></pre><div class="in-filters"><label>Search<input type="search" name="q" data-in-filter></label><label>Status<input name="status" data-in-filter placeholder="All statuses"></label><label>Sort<select name="sort" data-in-filter><option value="newest">Newest first</option><option value="oldest">Oldest first</option></select></label><button class="act" data-in-refresh>Refresh</button></div><div data-in-count></div><div class="in-columns"><section data-in-list aria-label="Integration records"></section><section data-in-detail aria-label="Record details"></section></div></div><footer class="m-foot"><span data-in-page></span><span class="grow"></span><button class="act" data-in-prev>Previous</button><button class="act" data-in-next>Next</button></footer>`, { wide: true, cls: "in-panel", accent: "var(--gold)", onClose: () => { revision++; panel = null; ticket = null; } });
    const own = panel;
    try {
      [config, writable] = await Promise.all([get(""), calls.ready()]);
      if (own !== panel) return state();
      el("[data-in-form]").innerHTML = reviewMode ? `<details><summary>Launch another sandbox</summary>${form()}</details>` : form(); panel.querySelectorAll("[data-in-write]").forEach((e) => e.setAttribute("data-yours", "")); lock(); await refresh();
    } catch (e) { if (own === panel) message(e.message); }
    if (own !== panel) return state();
    panel.addEventListener("input", (e) => {
      if (e.target.matches("[data-in-draft]")) invalidate();
      if (e.target.matches("[data-in-filter]")) { filters[e.target.name] = e.target.value; filters.page = 1; refresh().catch((e) => message(e.message)); }
    });
    panel.addEventListener("change", (e) => {
      if (e.target.matches("[data-in-outcome]")) work(async () => { await write("/route/outcome", { id: selected, outcome: e.target.value }); await refresh(); });
      if (e.target.matches("[data-in-import]")) work(async () => {
        const file = e.target.files?.[0]; if (!file) return;
        if (file.size > 1024 * 1024) throw new Error("Choose an export under 1 MiB.");
        let value; try { value = JSON.parse(await file.text()); } catch { throw new Error("This file is not a valid JSON run export."); }
        await write("/pi/import", value); await refresh(); message("Imported metadata only.");
      });
    });
    panel.addEventListener("click", (e) => {
      const target = e.target.closest("button"); if (!target) return;
      if (target.dataset.inTab) { if (!busy) open(target.dataset.inTab); return; }
      if (target.dataset.inId) { if (!busy) { selected = target.dataset.inId; review = null; renderDetail(); } return; }
      if (target.hasAttribute("data-in-review-file")) { reviewFile = target.dataset.inReviewFile; renderReview(); return; }
      work(async () => {
        if (target.matches("[data-in-preview]")) await preview();
        else if (target.matches("[data-in-confirm]")) {
          const reviewed = ticket; invalidate();
          const result = await write(tab === "routing" ? "/route/send" : "/sandbox/launch", { ticket: reviewed });
          selected = result.id; await refresh(); el("[data-in-detail]")?.scrollIntoView({ block: "center" }); message(tab === "routing" ? "Suggestion recorded. No agent started." : "Launch result recorded. Inspect the gateway for current status.");
        } else if (target.matches("[data-in-check]")) { const result = await write("/sandbox/check", draft()); message(result.ready ? `${result.version} · gateway ready${result.experimental ? " · experimental WSL support" : ""}` : result.reason); }
        else if (target.matches("[data-in-enable]")) { await write("/pi/enable", { on: !config.piOn }); config = await get(""); el("[data-in-form]").innerHTML = form(); panel.querySelectorAll("[data-in-write]").forEach((e) => e.setAttribute("data-yours", "")); await refresh(); }
        else if (target.matches("[data-in-inspect]")) { const result = await write("/sandbox/inspect", { id: selected }); el("[data-in-inspection]").innerHTML = `<h4>Gateway status: ${h(result.sandbox.phase)}</h4><p>${h(result.audit)}</p><h4>Effective policy</h4><pre>${h(result.policy)}</pre><h4>Recent logs</h4><pre>${h(result.logs || "No log entries returned.")}</pre>`; }
        else if (target.matches("[data-in-review]")) await openReview();
        else if (target.matches("[data-in-retrieve]")) { const record = selected; const result = await write("/sandbox/retrieve", { id: record }); review = { ...result, record }; reviewFile = null; await refresh(); message("Retrieved a text snapshot. Execution status remains unknown."); }
        else if (target.matches("[data-in-export]")) { if (!savePatch) throw new Error("Patch saving requires XenoNet Desktop."); const result = await write("/sandbox/export", { id: selected, revision: review?.result?.revision }); const saved = await savePatch(result); message(saved ? `Saved ${saved.file}. The project was not changed.` : "Patch export canceled."); }
        else if (target.matches("[data-in-forget-preview]")) { target.textContent = "Delete the local baseline and retrieved changes"; target.removeAttribute("data-in-forget-preview"); target.setAttribute("data-in-forget", ""); }
        else if (target.matches("[data-in-forget]")) { await write("/sandbox/forget-review", { id: selected }); review = null; await refresh(); message("Local review forgotten. Sandbox files are unchanged."); }
        else if (target.matches("[data-in-delete-preview]")) { target.textContent = "Delete this sandbox and its files"; target.removeAttribute("data-in-delete-preview"); target.setAttribute("data-in-delete", ""); }
        else if (target.matches("[data-in-delete]")) { await write("/sandbox/delete", { id: selected }); await refresh(); }
        else { if (target.matches("[data-in-prev]")) filters.page--; if (target.matches("[data-in-next]")) filters.page++; await refresh(); }
      });
    });
    return state();
  }
  return { open, get state() { return state(); }, async action(action = "open", value) {
    if (action === "open") return open(value);
    if (!panel) await open();
    if (action === "filter") { filters = { ...filters, ...value, page: 1 }; panel.querySelectorAll("[data-in-filter]").forEach((i) => { i.value = filters[i.name]; }); await refresh(); }
    else if (action === "select") { selected = value; review = null; reviewFile = null; renderDetail(); }
    else if (action === "review") await openReview();
    else if (action === "file") { if (!review?.result?.changes.some((f) => f.path === value)) throw new Error("No such reviewed text file."); reviewFile = value; renderReview(); }
    else if (action === "page") { filters.page = Number(value) || 1; await refresh(); }
    else if (action === "refresh") await refresh();
    else if (action === "draft") { for (const [name, val] of Object.entries(value ?? {})) { const input = [...panel.querySelectorAll("[data-in-draft]")].find((i) => i.name === name); if (input) { if (input.type === "checkbox") input.checked = val === true; else input.value = String(val); } } invalidate(); }
    else throw new Error(`Unknown integration browser action: ${action}`);
    return state();
  } };
}
