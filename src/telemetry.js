// Token-flux oscilloscope, usage-limit gauges and the live agents list.
import { escapeHtml as h, timeAgo } from "./format.js";
import { COLORS, THEME, categoryVar, withAlpha } from "./theme.js";
import { hostTag } from "./hosts.js";
import { stoppedWords } from "./live-words.js";
import { openRouterView } from "./openrouter-view.js";

const $ = (sel) => document.querySelector(sel);

export function compact(n) {
  if (n == null || !Number.isFinite(n)) return "—";
  const a = Math.abs(n);
  if (a >= 1e9) return (n / 1e9).toFixed(a >= 1e10 ? 0 : 1) + "B";
  if (a >= 1e6) return (n / 1e6).toFixed(a >= 1e7 ? 0 : 1) + "M";
  if (a >= 1e3) return (n / 1e3).toFixed(a >= 1e4 ? 0 : 1) + "k";
  return String(Math.round(n));
}

// Dollars as the panel shows them: cents under a hundred.
export const money = (v) => (v == null || !Number.isFinite(v) ? "—" : "$" + (Math.abs(v) >= 100 ? Math.round(v).toLocaleString() : v.toFixed(2)));
const RESET = { daily: "day", weekly: "week", monthly: "month" };

const windowLabel = (min) => (!min ? "" : min >= 10080 ? "week" : min >= 1440 ? `${Math.round(min / 1440)}d` : `${Math.round(min / 60)}h`);

// Numbers glide toward their targets instead of jumping.
function counter(el, format = compact) {
  let shown = 0, target = 0, raf = 0;
  const step = () => {
    shown += (target - shown) * 0.18;
    if (Math.abs(target - shown) < Math.max(1, Math.abs(target) * 0.001)) shown = target;
    el.textContent = format(shown);
    raf = shown === target ? 0 : requestAnimationFrame(step);
  };
  return (v) => {
    target = v ?? 0;
    if (!raf) raf = requestAnimationFrame(step);
  };
}

export function createTelemetry({ onSelect, onDecisionKey = null }) {
  const canvas = $("#scope");
  const ctx = canvas.getContext("2d");
  const setTpm = counter($("#flux-tpm"));
  const setTotal = counter($("#flux-total"));
  const setSpend = counter($("#spend-today"), money);
  const setPace = counter($("#spend-pace"), money);
  let snap = null;
  // What OpenRouter's key has spent (GET /api/spend), read every minute
  // while the window shows.
  let spend = null;
  let spendTimer = null;
  $("#gauges").addEventListener("click", (e) => { if (e.target.closest("[data-openrouter-setup]")) onDecisionKey?.(); });
  async function readSpend() {
    if (!document.hidden) {
      spend = await fetch("/api/spend").then((r) => (r.ok ? r.json() : null)).catch(() => null);
      if (snap) renderGauges();
    }
    clearTimeout(spendTimer);
    spendTimer = setTimeout(readSpend, 60e3);
  }
  readSpend();
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && !spend) readSpend();
  });
  let index = new Map();

  // The canvas's pixels per CSS pixel, kept from its last resize so a
  // frame never has to ask the page's layout.
  let ratio = 1;
  function resize() {
    const r = canvas.getBoundingClientRect();
    const pr = Math.min(devicePixelRatio, 2);
    canvas.width = Math.max(1, Math.round(r.width * pr));
    canvas.height = Math.max(1, Math.round(r.height * pr));
    ratio = canvas.width / Math.max(1, r.width);
  }
  addEventListener("resize", resize);
  if (typeof ResizeObserver === "function") new ResizeObserver(resize).observe(canvas);
  resize();

  const smooth = (arr, k = 4) => arr.map((_, i) => {
    let s = 0, c = 0;
    for (let j = Math.max(0, i - k); j <= Math.min(arr.length - 1, i + k); j++) (s += arr[j]), c++;
    return s / c;
  });

  // The series smoothed, worked out once for each snapshot.
  let smoothed = null, smoothedFor = null;
  // Drawn on the map's frames (main.js), at most SCOPE_FPS a second: it
  // scrolls two pixels a second, and its marker pulses slowly.
  const SCOPE_FPS = 15;
  let drawnAt = -Infinity;
  function frame(now) {
    if (now - drawnAt < 1000 / SCOPE_FPS - 2) return;
    drawnAt = now;
    draw();
  }
  function draw() {
    const W = canvas.width, H = canvas.height;
    const pr = ratio;
    ctx.clearRect(0, 0, W, H);
    // grid
    ctx.strokeStyle = withAlpha(THEME.colors.accent, 0.08);
    ctx.lineWidth = 1;
    for (let i = 1; i < 4; i++) {
      const y = Math.round((H * i) / 4) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
      ctx.stroke();
    }
    const shift = snap ? Math.min(2, (Date.now() - snap.at) / 1000) : 0;
    for (let s = 0; s <= 5; s++) {
      const x = Math.round(W - ((s * 60 + shift) / 300) * W) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
      ctx.stroke();
    }
    if (!snap) return;
    if (smoothedFor !== snap) (smoothedFor = snap), (smoothed = { claude: smooth(snap.series.claude), codex: smooth(snap.series.codex) });
    const series = smoothed;
    const max = Math.max(2000, ...series.claude, ...series.codex);
    const scale = (v) => H - 4 - (Math.log10(1 + v) / Math.log10(1 + max)) * (H - 10);
    for (const app of ["codex", "claude"]) {
      const data = series[app];
      const color = COLORS[app];
      ctx.beginPath();
      data.forEach((v, i) => {
        const x = W - ((data.length - 1 - i + shift) / 300) * W;
        i ? ctx.lineTo(x, scale(v)) : ctx.moveTo(x, scale(v));
      });
      ctx.save();
      ctx.shadowColor = color;
      ctx.shadowBlur = 10 * pr;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.6 * pr;
      ctx.lineJoin = "round";
      ctx.stroke();
      ctx.restore();
      ctx.lineTo(W, H);
      ctx.lineTo(0, H);
      ctx.closePath();
      const grd = ctx.createLinearGradient(0, 0, 0, H);
      grd.addColorStop(0, color + "33");
      grd.addColorStop(1, color + "00");
      ctx.fillStyle = grd;
      ctx.fill();
    }
    // "now" marker
    ctx.fillStyle = withAlpha(THEME.colors.accent, 0.35 + 0.3 * Math.sin(Date.now() / 260));
    ctx.fillRect(W - 2 * pr, 0, 2 * pr, H);
  }

  // Just the bar and its value; the forecast and reset time are on hover.
  function gauge(label, pct, sub, forecast, value = null, cls = "", setup = false) {
    const title = h([sub, forecast?.text].filter(Boolean).join(" · "));
    if (pct == null) return `<${setup ? 'button type="button" data-openrouter-setup' : "div"} class="gauge off ${cls}${setup ? " setup" : ""}" title="${title}"><span class="g-k">${h(label)}</span><span class="g-bar"></span><span class="g-v">${h(value ?? "—")}</span></${setup ? "button" : "div"}>`;
    const segs = 24;
    const lit = Math.round((Math.min(100, pct) / 100) * segs);
    const tone = pct > 90 ? "crit" : pct > 70 ? "warn" : "";
    const bar = Array.from({ length: segs }, (_, i) => `<i class="${i < lit ? "on" : ""}"></i>`).join("");
    return `<div class="gauge ${tone} ${cls}" title="${title}"><span class="g-k">${h(label)}</span><span class="g-bar">${bar}</span><span class="g-v">${h(value ?? `${Math.round(pct)}%`)}</span></div>`;
  }

  const when = (ms) => {
    const d = new Date(ms);
    const days = Math.round((d - new Date().setHours(0, 0, 0, 0)) / 86400e3 - 0.5);
    const t = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    return days <= 0 ? `today ${t}` : days === 1 ? `tomorrow ${t}` : `${d.toLocaleDateString([], { weekday: "short" })} ${t}`;
  };
  const inRel = (ms) => {
    const m = Math.max(0, Math.round((ms - Date.now()) / 60e3));
    return m >= 1440 ? `${Math.floor(m / 1440)}d ${Math.floor((m % 1440) / 60)}h` : m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
  };

  // "At this pace" projections, honest about what's known.
  function forecastText(fc, { resetLabel = true } = {}) {
    if (!fc) return null;
    if (fc.hitsBeforeReset && fc.eta) return { text: `at this pace, full by ${when(fc.eta)}${fc.resetsAt && resetLabel ? ` · resets in ${inRel(fc.resetsAt)}` : ""}`, warn: true };
    if (fc.resetsAt && resetLabel) return { text: `resets in ${inRel(fc.resetsAt)}`, warn: false };
    return { text: fc.perHour > 0.05 ? `+${fc.perHour.toFixed(1)}% an hour` : "not changing", warn: false };
  }

  function renderGauges() {
    const L = snap.limits ?? {};
    const F = snap.limitForecast ?? {};
    const cx = L.codex;
    const resets = (w) => (w?.resetsAt ? `resets ${new Date(w.resetsAt).toLocaleString()}` : "");
    $("#gauges").innerHTML =
      gauge(`codex · ${windowLabel(cx?.primary?.windowMinutes) || "limit"}`, cx?.primary?.pct ?? null, "Codex usage, current window", forecastText(F.codex)) +
      (cx?.secondary ? gauge(`codex · ${windowLabel(cx.secondary.windowMinutes)}`, cx.secondary.pct, resets(cx.secondary)) : "") +
      claudeGauge("claude · 5h", "fiveHour", "five-hour", F.claudeFiveHour) +
      claudeGauge("claude · 7d", "sevenDay", "seven-day", F.claudeSevenDay) +
      (L.others ?? []).map(otherGauges).join("") +
      openrouterGauge();
    renderCosts();
  }

  // OpenRouter's key: how much of its limit is spent, or with no limit,
  // this month's spend; the rest on hover. Missing keys stay visible so
  // the person can see what needs setting up.
  function openrouterGauge() {
    const o = spend?.openrouter;
    const view = openRouterView(o);
    if (view) return gauge("openrouter", null, view.hint, null, view.value, "or", view.setup);
    const spent = [o.today != null && `${money(o.today)} today`, o.week != null && `${money(o.week)} this week`, o.month != null && `${money(o.month)} this month`].filter(Boolean).join(", ");
    if (o.limit) {
      const used = Math.max(0, o.limit - (o.remaining ?? o.limit));
      return gauge("openrouter", (used / o.limit) * 100, `OpenRouter: ${spent} · ${money(used)} of the key's ${money(o.limit)}${RESET[o.reset] ? ` a ${RESET[o.reset]}` : ""} spent, ${money(o.remaining)} left`, null, money(used), "or");
    }
    return gauge("openrouter", null, `OpenRouter: ${spent}${o.freeTier ? " · free tier" : ""} · no limit on the key`, null, money(o.month), "or");
  }

  // About what today's tokens cost at list prices, and at this pace.
  function renderCosts() {
    const c = snap.costs;
    const el = $("#spend");
    el.hidden = !c;
    if (!c) return;
    setSpend(c.today);
    setPace(c.perHour);
    el.title = [`About ${money(c.byApp.claude)} for Claude Code and ${money(c.byApp.codex)} for Codex today, at Anthropic's and OpenAI's API prices (checked ${c.checked}).`, c.unpriced.length ? `Not counted, with no price: ${c.unpriced.join(", ")}.` : "", "On a plan, the work costs the plan; this is what it would cost on the API."].filter(Boolean).join(" ");
  }

  // An account signed in only on another machine (with XENONET_HOSTS=1): its
  // plan's bars of their own under the machine it was read on, never named
  // by the account, so hidden identities stay hidden.
  function otherGauges(o) {
    const r = o.reading ?? {};
    const on = `Read on ${o.host}, for an account that isn't signed in here`;
    const head = `<div class="gauge-host" title="${h(on)}">${h(`${o.app === "codex" ? "Codex" : "Claude"} account on ${o.host}`)}</div>`;
    if (o.app === "codex") return head + gauge(`codex · ${windowLabel(r.primary?.windowMinutes) || "limit"}`, r.primary?.pct ?? null, on) + (r.secondary ? gauge(`codex · ${windowLabel(r.secondary.windowMinutes)}`, r.secondary.pct ?? null, on) : "");
    if (r.stale) return head + gauge("claude · 5h", null, `${on}, with no recent reading`);
    return head + gauge("claude · 5h", r.fiveHour ?? null, on) + gauge("claude · 7d", r.sevenDay ?? null, on);
  }

  // Claude's gauges come from Anthropic, or Claude desktop's samples. With no
  // recent reading they show "—", and the tooltip says since when and why.
  function claudeGauge(label, key, window, forecast) {
    const c = snap.limits?.claude;
    if (!c?.stale) return gauge(label, c?.[key] ?? null, `Claude plan usage, ${window} window`, forecastText(forecast));
    const since = c.observedAt ? `since ${new Date(c.observedAt).toLocaleString()}` : "yet";
    return gauge(label, null, `No Claude plan usage reading ${since}${c.reason ? `: ${c.reason}` : ""}`);
  }

  const STATUS_LABEL = { tool: "tool", thinking: "thinking", idle: "idle" };

  function renderAgents() {
    const now = Date.now();
    const rows = snap.sessions
      .filter((s) => index.has(s.id) && (s.status !== "idle" || now - s.lastEventAt < 3 * 60e3 || (s.needsYou && s.needsYou.kind !== "review")))
      .sort((a, b) => (a.status === "idle") - (b.status === "idle") || b.lastEventAt - a.lastEventAt)
      .slice(0, 8);
    $("#agents-count").textContent = String(snap.sessions.filter((s) => s.status !== "idle").length).padStart(2, "0");
    $("#agents").innerHTML =
      rows
        .map((s) => {
          const n = index.get(s.id);
          const pct = s.context.window ? Math.min(100, (s.context.used / s.context.window) * 100) : 0;
          // A turn you stopped says so, while the session's idle after it.
          const stopped = stoppedWords(s, now);
          const tool = s.status === "tool" && s.lastTool ? `<span class="a-tool" style="--c:${categoryVar(s.lastTool.category)}">${h(s.lastTool.name.replace(/^mcp__/, "").replace(/__/g, "."))}</span>` : `<span class="a-state ${h(s.status)}">${STATUS_LABEL[s.status]}${stopped ? ` · ${h(stopped)}` : ""}</span>`;
          const subs = s.subagents.filter((a) => a.active).length;
          const need = s.needsYou && s.needsYou.kind !== "review" ? `<span class="a-need">◉ waiting</span>` : "";
          const stuck = s.stuck ? `<span class="a-stuck">⟳ may be stuck</span>` : "";
          const left = s.forecast?.turnsLeft;
          const fc = left != null && left <= 10 ? `<span class="a-fc ${left <= 3 ? "crit" : "warn"}">${left <= 0 ? "compaction due" : `~${h(left)} turn${left === 1 ? "" : "s"}`}</span>` : "";
          return `<li data-goto="${h(s.id)}" class="${h(s.status)}">
            <i class="sw ${h(s.app)}"></i>
            <div class="a-main">
              <div class="a-title">${hostTag(n)}${h(n.title)}</div>
              <div class="a-meta">${need}${stuck}${tool}${subs ? `<span class="a-subs">◈ ${subs}</span>` : ""}${fc}<span class="a-tpm">${compact(s.tpm)}/m</span></div>
              <div class="ctx-bar" title="context ${compact(s.context.used)} / ${compact(s.context.window)}"><b style="width:${pct.toFixed(1)}%" class="${pct > 90 ? "crit" : pct > 70 ? "warn" : ""}"></b></div>
            </div>
          </li>`;
        })
        .join("") || `<li class="empty">Nothing running · last activity ${timeAgo(Math.max(0, ...snap.sessions.map((s) => s.lastEventAt)))}</li>`;
  }

  return {
    refreshSpend: readSpend,
    // One of the map's frames (scene.onDrawn): the scope moves on.
    frame,
    update(next, graphIndex) {
      snap = next;
      index = graphIndex;
      setTpm(snap.tpm);
      setTotal(snap.total24h);
      renderGauges();
      renderAgents();
    },
    get snapshot() {
      return snap;
    },
    // What OpenRouter's key has spent, as GET /api/spend last said.
    get spend() {
      return spend;
    },
  };
}
