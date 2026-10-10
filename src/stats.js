// A session's stats, large: what it would cost at API prices, which models it
// used and when, where its tokens went, how long its turns took, how full its
// context got, and its tools (core/stats.js, through
// GET /api/session/:id/stats). Opened from the inspector (Stats), the palette
// (Mod+I on a session) and the driver.
//
// Charts follow the dataviz rules: one axis per chart, thin marks with 2px
// gaps, a legend for two series or more, values first in tooltips, and a
// table view for every chart. Colors come from the --st-* variables in
// style.css, validated on the panel's surface.
import { escapeHtml as h, shortPath, stamp } from "./format.js";
import { money, tokens, duration, share } from "../core/units.js";

const MODEL_SLOTS = 5;
const KINDS = [
  ["cacheRead", "read from cache"],
  ["input", "input"],
  ["cacheWrite", "written to cache"],
  ["output", "output"],
];
const APP_NAME = { claude: "Claude Code", codex: "Codex" };
const clockOf = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

// Round ticks from 0 to at least `max`: steps of 1, 2, 2.5 or 5 × 10^n.
// A remote session's stats come from its host as they came, so a max too
// small to step through (a step of 0, which would never end) or too large
// to count gets the ticks of nothing, and there are never more than a few.
function ticks(max, count = 4) {
  if (!(max > 0) || !Number.isFinite(max)) return [0, 1];
  const raw = max / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw);
  if (!(step > 0) || !Number.isFinite(step)) return [0, 1];
  const out = [];
  for (let v = 0; v < max + step * 0.999 && out.length <= count + 2; v += step) out.push(+v.toPrecision(12));
  return out;
}

// A column with a 4px rounded top and a square base.
function column(x, y, w, hgt, fill, round = true) {
  const r = round ? Math.min(4, w / 2, hgt) : 0;
  return `<path d="M${x},${y + hgt}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + hgt}Z" style="fill:${fill}"/>`;
}
// A bar growing right, with a 4px rounded end.
function bar(x, y, w, hgt, fill, round = true) {
  const r = round ? Math.min(4, hgt / 2, w) : 0;
  return `<path d="M${x},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + hgt - r}Q${x + w},${y + hgt} ${x + w - r},${y + hgt}H${x}Z" style="fill:${fill}"/>`;
}

export function createStatsPanel({ modals, sfx, toast }) {
  let panel = null;
  let data = null;
  let node = null;
  let timer = null;
  let tips = new Map();
  const tables = new Set();

  const modelColor = (m) => {
    const i = data.models.findIndex((x) => x.model === m);
    return i >= 0 && i < MODEL_SLOTS ? `var(--st-m${i + 1})` : "var(--st-other)";
  };
  const modelName = (m) => {
    const i = data.models.findIndex((x) => x.model === m);
    return i >= 0 && i < MODEL_SLOTS ? m : "other models";
  };

  // ---------- tooltip ----------

  // Rows of [text, strong?, keyColor?], built with textContent: model names
  // and prompts come from transcripts.
  function showTip(e, rows) {
    const tip = panel.querySelector(".st-tip");
    tip.replaceChildren(
      ...rows.map(([text, strong, key]) => {
        const row = document.createElement("div");
        row.className = strong ? "st-tip-v" : "st-tip-l";
        if (key) {
          const k = document.createElement("i");
          k.style.background = key;
          row.append(k);
        }
        row.append(document.createTextNode(text));
        return row;
      }),
    );
    tip.hidden = false;
    const box = panel.getBoundingClientRect();
    const r = e.target.getBoundingClientRect?.() ?? { left: e.clientX, top: e.clientY, width: 0 };
    const x = (e.clientX ?? r.left + r.width / 2) - box.left;
    const y = (e.clientY ?? r.top) - box.top;
    const w = tip.offsetWidth, ht = tip.offsetHeight;
    tip.style.left = `${Math.max(8, Math.min(box.width - w - 8, x + 14))}px`;
    tip.style.top = `${Math.max(8, y - ht - 12 < 8 ? y + 18 : y - ht - 12)}px`;
  }
  const hideTip = () => {
    const tip = panel?.querySelector(".st-tip");
    if (tip) tip.hidden = true;
  };

  // ---------- the charts ----------

  function turnsChart(W) {
    const turns = data.turns;
    const H = 230, top = 12, bottom = 36, left = 56, right = 12;
    const plotW = W - left - right, plotH = H - top - bottom;
    const max = Math.max(...turns.map((t) => t.cost), 0);
    const ys = ticks(max || 0.01);
    const top$ = ys.at(-1);
    const band = plotW / Math.max(1, turns.length);
    const bw = Math.max(1.5, Math.min(24, band * 0.72));
    const y = (v) => top + plotH - (v / top$) * plotH;
    const order = data.models.map((m) => m.model);
    let out = ys.map((v) => `<line class="st-gl" x1="${left}" x2="${W - right}" y1="${y(v)}" y2="${y(v)}"/><text class="st-ax" x="${left - 8}" y="${y(v) + 4}" text-anchor="end">${h(money(v))}</text>`).join("");
    // Times under some columns, at least 84px apart.
    const every = Math.max(1, Math.ceil(84 / band));
    turns.forEach((t, i) => {
      const x = left + i * band + (band - bw) / 2;
      let base = top + plotH;
      const parts = Object.entries(t.byModel).filter(([, v]) => v.cost > 0).sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0]));
      parts.forEach(([m, v], j) => {
        const hgt = (v.cost / top$) * plotH;
        const gap = j < parts.length - 1 && hgt > 3 ? 2 : 0;
        out += column(x, base - hgt, bw, Math.max(0.5, hgt - gap), modelColor(m), j === parts.length - 1);
        base -= hgt;
      });
      const id = `turn:${i}`;
      tips.set(id, () => {
        const rows = [[money(t.cost), true], [`Turn ${t.n} · ${clockOf(t.start)} · ${duration(t.end - t.start)}`]];
        for (const [m, v] of parts) rows.push([`${modelName(m)} ${money(v.cost)}`, false, modelColor(m)]);
        rows.push([`${tokens(t.tokens)} tokens · ${t.tools} tool calls${t.errors ? `, ${t.errors} failed` : ""}`]);
        if (data.compactions.some((c) => c >= t.start && c <= t.end)) rows.push(["Context compacted during this turn"]);
        if (t.prompt) rows.push([`"${t.prompt}"`]);
        return rows;
      });
      out += `<rect class="st-hit" data-tip="${id}" x="${left + i * band}" y="${top}" width="${band}" height="${plotH}"${turns.length <= 60 ? ' tabindex="0"' : ""}/>`;
      if (i % every === 0) out += `<text class="st-ax" x="${left + i * band + band / 2}" y="${H - bottom + 18}" text-anchor="middle">${h(turns.length > 1 ? `${t.n}` : clockOf(t.start))}</text>`;
    });
    out += `<line class="st-axis" x1="${left}" x2="${W - right}" y1="${top + plotH}" y2="${top + plotH}"/>`;
    out += `<text class="st-ax" x="${left}" y="${H - 4}">turn</text>`;
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Cost per turn, by model">${out}</svg>`;
  }

  function kindsChart(W) {
    const H = 120, left = 64, right = 8, rowH = 22, gap = 22;
    const plotW = W - left - right;
    let out = "";
    const rows = [
      ["Tokens", (k) => data.kinds[k].tokens, data.totals.tokens, (v) => tokens(v)],
      ["Cost", (k) => data.kinds[k].cost ?? 0, data.totals.cost ?? 0, (v) => money(v)],
    ];
    rows.forEach(([label, value, total, fmt], r) => {
      const yy = 14 + r * (rowH + gap);
      out += `<text class="st-ax" x="${left - 8}" y="${yy + rowH / 2 + 4}" text-anchor="end">${label}</text>`;
      let x = left;
      KINDS.forEach(([k, name], i) => {
        const v = value(k);
        const w = total ? (v / total) * plotW : 0;
        if (w <= 0) return;
        const last = KINDS.slice(i + 1).every(([k2]) => !value(k2));
        out += bar(x, yy, Math.max(0.5, w - (last ? 0 : 2)), rowH, `var(--st-k-${k})`, last);
        const text = share(v, total);
        if (w > text.length * 7 + 12) out += `<text class="st-in ${k === "output" || k === "cacheWrite" ? "dark" : ""}" x="${x + w / 2}" y="${yy + rowH / 2 + 4}" text-anchor="middle">${h(text)}</text>`;
        const id = `kind:${r}:${k}`;
        tips.set(id, () => [[fmt(v), true], [`${name} · ${share(v, total)} of ${label.toLowerCase()}`, false, `var(--st-k-${k})`]]);
        out += `<rect class="st-hit" data-tip="${id}" x="${x}" y="${yy - 3}" width="${Math.max(6, w)}" height="${rowH + 6}"/>`;
        x += w;
      });
    });
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Where the tokens went">${out}</svg>`;
  }

  function modelsChart(W) {
    const models = data.models.slice(0, MODEL_SLOTS);
    const rest = data.models.slice(MODEL_SLOTS);
    const rows = [...models.map((m) => ({ ...m, color: modelColor(m.model) })), ...(rest.length ? [{ model: `${rest.length} other models`, calls: rest.reduce((a, m) => a + m.calls, 0), tokens: rest.reduce((a, m) => a + m.tokens, 0), cost: rest.reduce((a, m) => a + (m.cost ?? 0), 0), color: "var(--st-other)", efforts: [] }] : [])];
    const labelW = Math.min(220, Math.max(...rows.map((r) => r.model.length)) * 7 + 12);
    const rowH = 18, pitch = 34, left = labelW, right = 150;
    const H = rows.length * pitch + 8;
    const max = Math.max(...rows.map((r) => r.cost ?? 0), 0.0001);
    let out = "";
    rows.forEach((r, i) => {
      const yy = 6 + i * pitch;
      const w = ((r.cost ?? 0) / max) * (W - left - right);
      out += `<text class="st-lab" x="${left - 10}" y="${yy + rowH / 2 + 4}" text-anchor="end">${h(r.model)}</text>`;
      out += bar(left, yy, Math.max(1, w), rowH, r.color);
      out += `<text class="st-val" x="${left + w + 8}" y="${yy + rowH / 2 + 4}">${h(money(r.cost))} <tspan class="st-dim">· ${h(share(r.cost ?? 0, data.totals.cost ?? 0))}</tspan></text>`;
      const id = `model:${i}`;
      tips.set(id, () => [[money(r.cost), true], [r.model, false, r.color], [`${r.calls.toLocaleString()} calls · ${tokens(r.tokens)} tokens`], ...(r.from ? [[`${clockOf(r.from)} to ${clockOf(r.to)}${r.efforts?.length ? ` · effort ${r.efforts.join(", ")}` : ""}`]] : [])]);
      out += `<rect class="st-hit" data-tip="${id}" x="0" y="${yy - 6}" width="${W}" height="${pitch}" tabindex="0"/>`;
    });
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Cost by model">${out}</svg>`;
  }

  function turnTimeChart(W) {
    const turns = data.turns;
    const H = 170, top = 10, bottom = 30, left = 52, right = 8;
    const plotW = W - left - right, plotH = H - top - bottom;
    const mins = turns.map((t) => (t.end - t.start) / 60000);
    const ys = ticks(Math.max(...mins, 0.1));
    const top$ = ys.at(-1);
    const band = plotW / Math.max(1, turns.length);
    const bw = Math.max(1.5, Math.min(24, band * 0.72));
    const y = (v) => top + plotH - (v / top$) * plotH;
    let out = ys.map((v) => `<line class="st-gl" x1="${left}" x2="${W - right}" y1="${y(v)}" y2="${y(v)}"/><text class="st-ax" x="${left - 8}" y="${y(v) + 4}" text-anchor="end">${v < 1 && v > 0 ? `${Math.round(v * 60)} s` : `${v} min`}</text>`).join("");
    const every = Math.max(1, Math.ceil(60 / band));
    turns.forEach((t, i) => {
      const hgt = (mins[i] / top$) * plotH;
      out += column(left + i * band + (band - bw) / 2, top + plotH - hgt, bw, Math.max(0.5, hgt), "var(--st-one)");
      const id = `time:${i}`;
      tips.set(id, () => [[duration(t.end - t.start), true], [`Turn ${t.n} · ${clockOf(t.start)}`], [`${t.tools} tool calls · ${money(t.cost)}`], ...(t.prompt ? [[`"${t.prompt}"`]] : [])]);
      out += `<rect class="st-hit" data-tip="${id}" x="${left + i * band}" y="${top}" width="${band}" height="${plotH}"/>`;
      if (i % every === 0) out += `<text class="st-ax" x="${left + i * band + band / 2}" y="${H - 10}" text-anchor="middle">${h(t.n)}</text>`;
    });
    out += `<line class="st-axis" x1="${left}" x2="${W - right}" y1="${top + plotH}" y2="${top + plotH}"/>`;
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Turn length">${out}</svg>`;
  }

  function contextChart(W) {
    const pts = data.context;
    const H = 170, top = 10, bottom = 30, left = 52, right = 10;
    const plotW = W - left - right, plotH = H - top - bottom;
    const window = Math.max(...pts.map((p) => p.window));
    const ys = ticks(window);
    const top$ = ys.at(-1);
    const t0 = pts[0].t, t1 = Math.max(pts.at(-1).t, t0 + 1);
    const x = (t) => left + ((t - t0) / (t1 - t0)) * plotW;
    const y = (v) => top + plotH - (v / top$) * plotH;
    let out = ys.map((v) => `<line class="st-gl" x1="${left}" x2="${W - right}" y1="${y(v)}" y2="${y(v)}"/><text class="st-ax" x="${left - 8}" y="${y(v) + 4}" text-anchor="end">${tokens(v)}</text>`).join("");
    out += `<line class="st-limit" x1="${left}" x2="${W - right}" y1="${y(window)}" y2="${y(window)}"/><text class="st-ax" x="${left + 6}" y="${y(window) + 15}">the model's window</text>`;
    for (const c of data.compactions) if (c >= t0 && c <= t1) out += `<line class="st-mark" x1="${x(c)}" x2="${x(c)}" y1="${top}" y2="${top + plotH}"/><text class="st-ax" x="${x(c) + 5}" y="${top + plotH - 6}">compacted</text>`;
    out += `<path class="st-line" d="${pts.map((p, i) => `${i ? "L" : "M"}${x(p.t).toFixed(1)},${y(p.used).toFixed(1)}`).join("")}"/>`;
    const last = pts.at(-1);
    out += `<circle class="st-dot" cx="${x(last.t)}" cy="${y(last.used)}" r="4"/>`;
    out += `<text class="st-ax" x="${left}" y="${H - 10}">${h(clockOf(t0))}</text><text class="st-ax" x="${W - right}" y="${H - 10}" text-anchor="end">${h(clockOf(t1))}</text>`;
    out += `<line class="st-cross" x1="0" x2="0" y1="${top}" y2="${top + plotH}" hidden/>`;
    out += `<rect class="st-hit" data-cross="context" x="${left}" y="${top}" width="${plotW}" height="${plotH}"/>`;
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Context over the session" data-x0="${h(t0)}" data-x1="${h(t1)}" data-left="${left}" data-plot="${plotW}">${out}</svg>`;
  }

  function toolsChart(W) {
    const top12 = data.tools.slice(0, 12);
    const rest = data.tools.slice(12);
    const rows = [...top12, ...(rest.length ? [{ name: `${rest.length} other tools`, calls: rest.reduce((a, t) => a + t.calls, 0), errors: rest.reduce((a, t) => a + t.errors, 0), ms: rest.reduce((a, t) => a + (t.ms ?? 0), 0) }] : [])];
    const label = (t) => t.name.replace(/^mcp__(.+?)__/, "$1.");
    const tail = (t) => `${t.calls.toLocaleString()}${t.ms ? ` · ${duration(t.ms)}` : ""}${t.errors ? ` · ! ${t.errors} failed` : ""}`;
    // Room for the longest label after its bar (about 7.4px a character).
    const left = Math.min(240, Math.max(...rows.map((t) => label(t).length)) * 7 + 14), right = Math.max(...rows.map((t) => tail(t).length)) * 7.4 + 16;
    const rowH = 14, pitch = 24;
    const H = rows.length * pitch + 6;
    const max = Math.max(...rows.map((t) => t.calls), 1);
    let out = "";
    rows.forEach((t, i) => {
      const yy = 5 + i * pitch;
      const w = (t.calls / max) * (W - left - right);
      out += `<text class="st-lab" x="${left - 10}" y="${yy + rowH / 2 + 4}" text-anchor="end">${h(label(t))}</text>`;
      out += bar(left, yy, Math.max(1, w), rowH, "var(--st-one)");
      out += `<text class="st-val" x="${left + w + 8}" y="${yy + rowH / 2 + 4}">${h(t.calls.toLocaleString())}<tspan class="st-dim">${t.ms ? ` · ${h(duration(t.ms))}` : ""}</tspan>${t.errors ? `<tspan class="st-bad"> · ! ${h(t.errors)} failed</tspan>` : ""}</text>`;
      const id = `tool:${i}`;
      tips.set(id, () => [[`${t.calls.toLocaleString()} calls`, true], [label(t)], ...(t.ms ? [[`${duration(t.ms)} in all, ${duration(t.ms / Math.max(1, t.calls))} each on average`]] : []), ...(t.errors ? [[`${t.errors} failed`]] : [])]);
      out += `<rect class="st-hit" data-tip="${id}" x="0" y="${yy - 5}" width="${W}" height="${pitch}"/>`;
    });
    return `<svg viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Tool calls">${out}</svg>`;
  }

  // ---------- tables (every chart's twin) ----------

  const table = (head, rows) => `<table class="st-table"><thead><tr>${head.map((x) => `<th>${h(x)}</th>`).join("")}</tr></thead><tbody>${rows.map((r) => `<tr>${r.map((x) => `<td>${h(x)}</td>`).join("")}</tr>`).join("")}</tbody></table>`;
  const TABLES = {
    turns: () => table(["Turn", "Started", "Took", "Cost", "Tokens", "Tools", "Prompt"], data.turns.map((t) => [t.n, clockOf(t.start), duration(t.end - t.start), money(t.cost), tokens(t.tokens), t.tools, t.prompt ?? ""])),
    kinds: () => table(["Kind", "Tokens", "Share", "Cost", "Share"], KINDS.map(([k, name]) => [name, tokens(data.kinds[k].tokens), share(data.kinds[k].tokens, data.totals.tokens), money(data.kinds[k].cost), share(data.kinds[k].cost ?? 0, data.totals.cost ?? 0)])),
    models: () => table(["Model", "Calls", "Tokens", "Cost", "From", "To"], data.models.map((m) => [m.model, m.calls, tokens(m.tokens), money(m.cost), clockOf(m.from), clockOf(m.to)])),
    time: () => table(["Turn", "Started", "Took"], data.turns.map((t) => [t.n, clockOf(t.start), duration(t.end - t.start)])),
    context: () => table(["Time", "Context", "Window"], data.context.map((p) => [clockOf(p.t), tokens(p.used), tokens(p.window)])),
    tools: () => table(["Tool", "Calls", "Failed", "Time"], data.tools.map((t) => [t.name, t.calls, t.errors, t.ms == null ? "" : duration(t.ms)])),
    subagents: () => table(["Subagent", "Model", "Calls", "Tokens", "Cost", "Took"], data.subagents.map((a) => [a.label ?? a.id, a.model, a.calls, tokens(a.tokens), money(a.cost), duration(a.to - a.from)])),
  };

  // ---------- the panel ----------

  const legend = (items) => `<div class="st-legend">${items.map(([label, color, kind = "rect"]) => `<span><i class="${kind}" style="background:${color}"></i>${h(label)}</span>`).join("")}</div>`;
  // A chart and its table, one showing at a time; `tableOnly` has no chart.
  const card = (id, title, body, { wide = false, sub = "", tableOnly = false } = {}) => `
    <section class="st-card ${wide ? "wide" : ""} ${tableOnly || tables.has(id) ? "as-table" : ""}" data-card="${id}">
      <header><h3>${h(title)}</h3>${sub ? `<span class="st-dim">${h(sub)}</span>` : ""}<span class="grow"></span>${tableOnly ? "" : `<button class="st-toggle" data-table="${id}">${tables.has(id) ? "Chart" : "Table"}</button>`}</header>
      ${body}${tableOnly ? "" : `<div class="st-chart" data-chart="${id}"></div>`}<div class="st-tablewrap">${TABLES[id]()}</div>
    </section>`;

  function render() {
    const d = data;
    const t = d.totals;
    const failed = d.tools.reduce((a, x) => a + x.errors, 0);
    const calls = d.tools.reduce((a, x) => a + x.calls, 0);
    const multi = d.models.length > 1;
    panel.querySelector(".m-sub").textContent = [shortPath(d.cwd, 60), d.branch, d.from && `${stamp(d.from)} to ${clockOf(d.to)}`].filter(Boolean).join(" · ");
    const kpi = (k, v, s, cls = "") => `<div class="st-kpi ${cls}"><div class="st-k">${h(k)}</div><div class="st-v">${h(v)}</div><div class="st-s">${h(s)}</div></div>`;
    panel.querySelector(".m-body").innerHTML = `
      <div class="st-kpis">
        ${kpi("Estimated cost", money(t.cost), t.cost == null ? "no API price for its models" : "at API prices on the day", "hero")}
        ${kpi("Tokens", tokens(t.tokens), `${share(t.cacheRead, t.tokens)} read from cache`)}
        ${kpi("At work", duration(d.activeMs), `over ${duration(d.to - d.from)} · ${d.turns.length} turn${d.turns.length === 1 ? "" : "s"}`)}
        ${kpi("Tool calls", calls.toLocaleString(), failed ? `${failed} failed` : "none failed")}
        ${kpi("Model calls", t.calls.toLocaleString(), multi ? `${d.models.length} models` : [d.models[0]?.model ?? "—", ...(d.models[0]?.efforts ?? [])].join(" · "))}
      </div>
      ${card("turns", multi ? "Cost per turn, by model" : "Cost per turn", multi ? legend(d.models.slice(0, MODEL_SLOTS).map((m) => [m.model, modelColor(m.model)]).concat(d.models.length > MODEL_SLOTS ? [["other models", "var(--st-other)"]] : [])) : "", { wide: true, sub: multi ? "" : d.models[0]?.model ?? "" })}
      <div class="st-grid">
        ${card("kinds", "Where the tokens went", legend(KINDS.map(([k, name]) => [`${name} · ${tokens(d.kinds[k].tokens)} · ${money(d.kinds[k].cost)}`, `var(--st-k-${k})`])), { sub: "cheapest to dearest", wide: !multi })}
        ${multi ? card("models", "Cost by model", "") : ""}
        ${card("time", "Turn length", "")}
        ${d.context.length > 1 ? card("context", "Context", "", { sub: "tokens in each call's prompt" }) : ""}
        ${d.tools.length ? card("tools", "Tool calls", "", { wide: true }) : ""}
        ${d.subagents.length ? card("subagents", "Subagents", `<p>${h(d.subagents.length)} subagent${d.subagents.length === 1 ? "" : "s"}, ${h(money(d.subagents.reduce((a, x) => a + x.cost, 0)))} of the cost</p>`, { wide: true, tableOnly: true }) : ""}
      </div>
      <p class="st-note">Estimated from the transcript at each model's API price on the day of each call, standard tier (batch and data residency not counted). On a subscription you paid the plan instead. Prices checked ${h(d.pricing.checked)}.${d.pricing.unpriced.length ? ` No price for ${h(d.pricing.unpriced.join(", "))}.` : ""}</p>`;
    draw();
  }

  // Charts drawn at each card's own width, so text stays its real size.
  function draw() {
    tips = new Map();
    const charts = { turns: turnsChart, kinds: kindsChart, models: modelsChart, time: turnTimeChart, context: contextChart, tools: toolsChart };
    for (const el of panel.querySelectorAll("[data-chart]")) {
      const make = charts[el.dataset.chart];
      if (make) el.innerHTML = make(Math.max(280, Math.floor(el.clientWidth)));
    }
  }

  async function load() {
    const res = await fetch(`/api/session/${encodeURIComponent(node.id)}/stats`);
    const body = await res.json();
    if (!res.ok) throw new Error(body.error ?? res.statusText);
    return body;
  }

  // Resolves once the numbers are on screen (or the reason they aren't).
  async function open(n) {
    node = n;
    data = null;
    tables.clear();
    clearInterval(timer);
    panel = modals.open(
      `<header class="m-head"><div><div class="m-chip">◫ stats · ${h(APP_NAME[n.app] ?? n.app)}</div><h2>${h(n.title ?? n.id)}</h2><div class="m-sub">Reading the transcript…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body st-body"><div class="m-empty">Adding up tokens and costs…</div></div><div class="st-tip" hidden></div>`,
      { cls: "full st", accent: n.app === "codex" ? "var(--codex)" : "var(--claude)" },
    );
    const me = panel;
    panel.addEventListener("click", (e) => {
      const b = e.target.closest("[data-table]");
      if (!b) return;
      const id = b.dataset.table;
      tables.has(id) ? tables.delete(id) : tables.add(id);
      const c = b.closest(".st-card");
      c.classList.toggle("as-table", tables.has(id));
      b.textContent = tables.has(id) ? "Chart" : "Table";
      sfx("select");
    });
    panel.addEventListener("pointermove", (e) => onPointer(e));
    panel.addEventListener("focusin", (e) => onPointer(e));
    panel.addEventListener("pointerleave", hideTip);
    panel.addEventListener("focusout", hideTip);
    try {
      data = await load();
      if (panel !== me) return;
      render();
    } catch (err) {
      if (panel === me) panel.querySelector(".m-body").innerHTML = `<div class="m-empty">${h(err.message)}</div>`;
      return;
    }
    // A session at work keeps adding up while the panel is open.
    if (n.activity === "active")
      timer = setInterval(async () => {
        if (!panel?.isConnected || panel !== me) return clearInterval(timer);
        try {
          data = await load();
          render();
        } catch {}
      }, 15000);
  }

  function onPointer(e) {
    const hit = e.target.closest?.("[data-tip], [data-cross]");
    if (!hit) return hideTip();
    if (hit.dataset.tip) return showTip(e, tips.get(hit.dataset.tip)?.() ?? []);
    // The context line: the nearest point to the pointer's time.
    const svg = hit.closest("svg");
    const box = svg.getBoundingClientRect();
    const scale = svg.viewBox.baseVal.width / box.width;
    const left = Number(svg.dataset.left), plot = Number(svg.dataset.plot), x0 = Number(svg.dataset.x0), x1 = Number(svg.dataset.x1);
    const px = ((e.clientX ?? box.left + box.width / 2) - box.left) * scale;
    const t = x0 + ((px - left) / plot) * (x1 - x0);
    const p = data.context.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
    const cross = svg.querySelector(".st-cross");
    const cx = left + ((p.t - x0) / (x1 - x0)) * plot;
    cross.setAttribute("x1", cx);
    cross.setAttribute("x2", cx);
    cross.hidden = false;
    showTip(e, [[`${tokens(p.used)} tokens`, true], [`${share(p.used, p.window)} of the ${tokens(p.window)} window · ${clockOf(p.t)}`]]);
  }

  addEventListener("resize", () => panel?.isConnected && data && draw());

  return { open, get data() { return panel?.isConnected ? data : null; } };
}
