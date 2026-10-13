import { escapeHtml as h } from "./format.js";
import { modelOf, categoryOf } from "../core/refusals/query.js";
import "./refusals.css";

const labels = { refusal: "Refusal", "refusal-recovered": "Recovered", reroute: "Reroute", "possible-refusal": "Possible refusal" };
const defaults = () => ({ q: "", provider: "", kind: "", model: "", category: "", context: "", from: "", to: "", sort: "newest" });
const stamp = (at) => new Date(at).toLocaleString();
const pair = (label, value) => `<dt>${h(label)}</dt><dd>${h(value || "Unknown")}</dd>`;

export function refusalDetail(r) {
  return `<h3>${h(labels[r.kind] ?? r.kind)}</h3><div class="rf-id">${h(r.id)}</div>
    <dl>${pair("Time", stamp(r.at))}${pair("Provider", r.provider)}${pair("App", r.app)}${pair("Model", modelOf(r))}${pair("Category", categoryOf(r))}
    ${r.signal?.fallbackModel ? pair("Fallback model", r.signal.fallbackModel) : ""}${pair("Session", r.session?.id)}${pair("Folder", r.session?.cwd)}
    ${r.signal?.requestId ? pair("Request ID", r.signal.requestId) : ""}${pair("Transcript", r.source?.file)}</dl>
    ${r.kind === "possible-refusal" ? '<p class="rf-note">This provider signal does not confirm a refusal.</p>' : ""}
    ${r.kind === "reroute" ? '<p class="rf-note">The provider routed the request to another model.</p>' : ""}
    <h4>Provider signal</h4><pre>${h(r.signal?.text || r.signal?.explanation || "No message recorded.")}</pre>
    <h4>Saved context</h4><p class="rf-note">${r.context ? `${h(r.context.lines ?? "Unknown")} transcript lines · ${h(r.context.bytes ?? "Unknown")} bytes${r.context.truncated ? " · Trimmed" : ""}` : "No context saved with this record."}</p>
    ${r.missing?.length ? `<p class="rf-note">Not recorded: ${h(r.missing.join("; "))}.</p>` : ""}
    ${r.context ? '<button class="act" data-rf-request>View saved request</button><p class="rf-note">Saved requests may contain private prompts and tool output.</p><div data-rf-request-body></div>' : ""}
    <details><summary>Record JSON</summary><pre>${h(JSON.stringify(r, null, 2))}</pre></details>`;
}

export function createRefusalsPanel({ modals }) {
  let panel = null, filters = defaults(), page = 1, data = null, selected = null, error = null;
  let controller = null, requestController = null, timer = null, revision = 0;
  const el = (selector) => panel?.querySelector(selector);
  const state = () => ({ open: !!panel, filters: { ...filters }, page, total: data?.total ?? 0, matched: data?.matched ?? 0, records: data?.records.map((r) => ({ id: r.id, provider: r.provider, kind: r.kind, model: modelOf(r), category: categoryOf(r), at: r.at, context: !!r.context })) ?? [], selected, error });
  function closed() {
    clearTimeout(timer); controller?.abort(); requestController?.abort(); revision++;
    panel = null;
  }
  async function read(url, signal) {
    const r = await fetch(url, { signal });
    const body = await r.json();
    if (!r.ok) throw new Error(body.error || (r.status === 403 ? "Open this panel from your own XenoNet window." : "Could not read refusal records."));
    return body;
  }
  function select(id) {
    requestController?.abort();
    const r = data?.records.find((r) => r.id === id);
    if (id && !r) throw new Error("This record is not on the current page.");
    selected = r?.id ?? null;
    el("[data-rf-detail]").innerHTML = r ? refusalDetail(r) : '<p class="rf-note">Select a record to inspect its signal and saved-context metadata.</p>';
    el("[data-rf-detail]").scrollTop = 0;
    panel.querySelectorAll("[data-rf-id]").forEach((row) => row.setAttribute("aria-pressed", String(row.dataset.rfId === selected)));
  }
  async function request() {
    const id = selected, host = el("[data-rf-request-body]");
    if (!host) throw new Error("This record has no saved context.");
    requestController?.abort(); requestController = new AbortController();
    const signal = requestController.signal;
    host.textContent = "Loading saved request…";
    try {
      const r = await read(`/api/refusals/${encodeURIComponent(id)}/request`, signal);
      if (!signal.aborted && selected === id && panel) host.innerHTML = `<details open><summary>Saved request JSON</summary><pre>${h(JSON.stringify(r.request, null, 2))}</pre></details>`;
    } catch (err) {
      if (!signal.aborted) host.textContent = err.message;
    }
  }
  function facets() {
    for (const key of ["provider", "kind", "model", "category"]) {
      const input = el(`[name="${key}"]`);
      const values = [...new Set([...(data.facets?.[key] ?? []), ...(filters[key] ? [filters[key]] : [])])];
      input.innerHTML = `<option value="">All ${{ kind: "outcomes", category: "categories", provider: "providers", model: "models" }[key]}</option>` + values.map((v) => `<option value="${h(v)}">${h(key === "kind" ? labels[v] ?? v : v)}</option>`).join("");
      input.value = filters[key];
    }
  }
  async function load() {
    clearTimeout(timer); controller?.abort(); requestController?.abort();
    controller = new AbortController(); const signal = controller.signal, current = ++revision;
    error = null; el("[data-rf-status]").textContent = "Loading records…";
    el("[data-rf-results]").setAttribute("aria-busy", "true");
    const q = new URLSearchParams({ limit: "50", offset: String((page - 1) * 50) });
    for (const key of ["q", "provider", "kind", "model", "category", "context", "sort"]) if (filters[key]) q.set(key, filters[key]);
    if (filters.from) q.set("since", String(new Date(`${filters.from}T00:00:00`).getTime()));
    if (filters.to) q.set("until", String(new Date(`${filters.to}T23:59:59.999`).getTime()));
    try {
      const result = await read(`/api/refusals?${q}`, signal);
      if (signal.aborted || current !== revision || !panel) return;
      data = result;
      if (page > 1 && !data.records.length) { page = Math.max(1, Math.ceil(data.matched / 50)); return load(); }
      facets();
      el("[data-rf-status]").textContent = `${data.matched} of ${data.total} records · Context recording ${data.context ? "on" : "off"}`;
      el("[data-rf-results]").innerHTML = data.records.length ? data.records.map((r) => `<button class="rf-row" data-rf-id="${h(r.id)}" aria-pressed="false"><span class="rf-row-top"><strong>${h(r.provider)}</strong><span class="rf-kind">${h(labels[r.kind] ?? r.kind)}</span></span><span>${h(modelOf(r) || "Unknown model")} · ${h(categoryOf(r) || "No category")}</span><span class="rf-row-bottom"><time datetime="${h(new Date(r.at).toISOString())}">${h(stamp(r.at))}</time><span>${r.context ? "Context saved" : "No context"}</span></span></button>`).join("") : `<p class="rf-note">${data.total ? "No records match these filters. Clear filters to see the collection." : "No provider refusal records have been saved yet."}</p>`;
      el("[data-rf-page]").textContent = `Page ${page} of ${Math.max(1, Math.ceil(data.matched / 50))}`;
      el("[data-rf-prev]").disabled = page <= 1;
      el("[data-rf-next]").disabled = page * 50 >= data.matched;
      select(data.records.some((r) => r.id === selected) ? selected : null);
    } catch (err) {
      if (signal.aborted || current !== revision || !panel) return;
      error = err.message; data = null; selected = null;
      el("[data-rf-status]").textContent = "Could not load records";
      el("[data-rf-results]").textContent = error;
      el("[data-rf-detail]").textContent = "Refresh to try again.";
      el("[data-rf-prev]").disabled = el("[data-rf-next]").disabled = true;
      el("[data-rf-page]").textContent = "";
    } finally {
      if (current === revision) el("[data-rf-results]")?.setAttribute("aria-busy", "false");
    }
  }
  async function filter(value = {}) {
    for (const [key, v] of Object.entries(value)) if (key in filters) filters[key] = String(v ?? "");
    panel.querySelectorAll("[data-rf-filter]").forEach((input) => { input.value = filters[input.name]; });
    page = 1; selected = null; return load();
  }
  async function open() {
    filters = defaults(); page = 1; data = null; selected = null;
    const dropdown = (key, label, options = "") => `<label>${label}<select name="${key}" aria-label="${label}" data-rf-filter>${options || '<option value="">All</option>'}</select></label>`;
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">Refusal records</div><h2>Provider refusals</h2><div class="m-sub">Browse saved provider signals and request metadata.</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body rf-body">
      <div class="rf-filters"><label class="rf-search">Search<input name="q" type="search" data-rf-filter placeholder="Model, signal, session or folder" autocomplete="off"></label>
      ${dropdown("provider", "Provider")}${dropdown("kind", "Outcome")}${dropdown("model", "Model")}${dropdown("category", "Category")}
      ${dropdown("context", "Saved context", '<option value="">All records</option><option value="with">With context</option><option value="without">Without context</option>')}
      <label>From<input type="date" name="from" data-rf-filter></label><label>Through<input type="date" name="to" data-rf-filter></label>
      ${dropdown("sort", "Sort", '<option value="newest">Newest first</option><option value="oldest">Oldest first</option><option value="provider">Provider</option><option value="model">Model</option><option value="category">Category</option><option value="kind">Outcome</option>')}</div>
      <div class="rf-toolbar"><span data-rf-status role="status" aria-live="polite"></span><button class="act" data-rf-clear>Clear filters</button><button class="act" data-rf-refresh>Refresh</button></div>
      <div class="rf-columns"><div data-rf-results class="rf-results" aria-label="Refusal records"></div><section data-rf-detail class="rf-detail" aria-label="Record details"></section></div>
      </div><footer class="m-foot"><span data-rf-page></span><span class="grow"></span><button class="act" data-rf-prev disabled>Previous</button><button class="act" data-rf-next disabled>Next</button></footer>`, { wide: true, cls: "rf-panel", accent: "var(--gold)", onClose: closed });
    panel.addEventListener("input", (e) => {
      if (!e.target.matches("[data-rf-filter]")) return;
      filters[e.target.name] = e.target.value; page = 1; selected = null;
      clearTimeout(timer);
      // Invalidate in-flight results immediately, not after the debounce.
      controller?.abort(); requestController?.abort(); revision++;
      timer = setTimeout(() => { if (panel) load(); }, e.target.name === "q" ? 220 : 0);
    });
    panel.addEventListener("click", (e) => {
      const row = e.target.closest("[data-rf-id]");
      if (row) select(row.dataset.rfId);
      else if (e.target.closest("[data-rf-clear]")) filter(defaults());
      else if (e.target.closest("[data-rf-refresh]")) load();
      else if (e.target.closest("[data-rf-prev]")) { page--; load(); }
      else if (e.target.closest("[data-rf-next]")) { page++; load(); }
      else if (e.target.closest("[data-rf-request]")) request();
    });
    el('[name="q"]').focus();
    await load();
    return state();
  }
  return { open, get state() { return state(); }, async action(action, value) {
    if (action === "open") return open();
    if (!panel) await open();
    if (action === "filter") await filter(value);
    else if (action === "select") select(value);
    else if (action === "page") { page = Math.min(Math.max(1, Math.floor(Number(value) || 1)), Math.max(1, Math.ceil((data?.matched ?? 0) / 50))); await load(); }
    else if (action === "refresh") await load();
    else if (action === "request") await request();
    else throw new Error(`Unknown refusal browser action: ${action}`);
    return state();
  } };
}
