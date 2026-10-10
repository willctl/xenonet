// This computer's load, between the status and Token use panels
// (core/system-stats.js): a ring each for the CPU, the GPU and memory, the
// GPU's own memory as the GPU's inner ring, each core's use, the last few
// minutes traced, and what the parts are. It asks every two seconds while
// the window shows it, and not at all while the window is hidden or the
// panel is turned off (the palette's Show or hide this computer's load).
import { escapeHtml as h } from "./format.js";

const EVERY = 2000;
const R = 44; // the ring's radius; the inner ring's is 36
const SWEEP = 270; // degrees, open at the bottom
const KEY = "xenonet.systemPanel";
const GB = 1024 ** 3;

const point = (r, deg) => [60 + r * Math.cos((deg * Math.PI) / 180), 60 + r * Math.sin((deg * Math.PI) / 180)];
// The ring's arc, from bottom left round over the top to bottom right.
const arc = (r) => {
  const [x1, y1] = point(r, 135);
  const [x2, y2] = point(r, 45);
  return `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${r} ${r} 0 1 1 ${x2.toFixed(2)} ${y2.toFixed(2)}`;
};
const clamp = (v) => Math.min(1, Math.max(0, Number(v) || 0));
const gb = (b) => (b >= 100 * GB ? Math.round(b / GB) : Math.round((b / GB) * 10) / 10);
const level = (v) => (v == null ? "" : v >= 0.95 ? "crit" : v >= 0.85 ? "warn" : "");
const OS = { win32: "Windows", darwin: "macOS", linux: "Linux" };

// The operating system's mark, drawn in the console's own lines inside a
// hexagon: Windows' four panes, an apple, or a penguin.
const MARKS = {
  win32: '<rect class="o-shape" x="3" y="3" width="8.4" height="8.4" /><rect class="o-shape" x="12.6" y="3" width="8.4" height="8.4" /><rect class="o-shape" x="3" y="12.6" width="8.4" height="8.4" /><rect class="o-shape" x="12.6" y="12.6" width="8.4" height="8.4" />',
  darwin: '<path class="o-shape" d="M16.4 12.7c0-2.3 1.9-3.4 2-3.5-1.1-1.6-2.8-1.8-3.4-1.8-1.4-.1-2.8.9-3.5.9-.7 0-1.8-.9-3-.8-1.5 0-3 .9-3.8 2.3-1.6 2.8-.4 7 1.2 9.3.8 1.1 1.7 2.4 2.9 2.3 1.2 0 1.6-.7 3-.7s1.8.7 3 .7c1.3 0 2.1-1.1 2.8-2.3.9-1.3 1.3-2.6 1.3-2.6s-2.5-1-2.5-3.8z" /><path class="o-shape" d="M14.1 5.8c.6-.8 1.1-1.8 1-2.8-.9 0-2 .6-2.6 1.4-.6.7-1.1 1.7-.9 2.7 1 .1 2-.5 2.5-1.3z" />',
  linux: '<ellipse class="o-shape" cx="12" cy="13.6" rx="6.2" ry="8.4" /><ellipse class="o-belly" cx="12" cy="15.6" rx="3.9" ry="5.4" /><circle class="o-dot" cx="10.2" cy="8.5" r="1.1" /><circle class="o-dot" cx="13.8" cy="8.5" r="1.1" /><path class="o-warm" d="M10.3 10.5Q12 9.6 13.7 10.5L12 12.2Z" /><path class="o-warm" d="M5.6 22.4q1.7-2.4 4.6-.9-.6 1.5-4.6.9zM18.4 22.4q-1.7-2.4-4.6-.9.6 1.5 4.6.9z" />',
};
export function osMark(platform) {
  const mark = MARKS[platform];
  if (!mark) return `<span class="sm-os-name">${h(platform ?? "")}</span>`;
  return `<svg class="sm-os" viewBox="0 0 32 32" role="img" aria-label="${OS[platform]}"><title>${OS[platform]}</title><path class="o-hex" d="M16 1.5 28.6 8.75v14.5L16 30.5 3.4 23.25V8.75Z" /><path class="o-hex-in" d="M16 4.6 25.9 10.3v11.4L16 27.4 6.1 21.7V10.3Z" /><g transform="translate(6 6) scale(.8333)">${mark}</g></svg>`;
}

export function uptimeText(s) {
  if (!Number.isFinite(s) || s <= 0) return "";
  const d = Math.floor(s / 86400);
  const hr = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `up ${d}d ${hr}h` : hr ? `up ${hr}h ${m}m` : `up ${m}m`;
}

// A trace's points, oldest at the left, across a 300 × 40 box.
export function tracePath(values, { area = false } = {}) {
  const pts = values.map((v, i) => (v == null ? null : [values.length < 2 ? 300 : (i / (values.length - 1)) * 300, 38 - clamp(v) * 34]));
  let d = "";
  let open = false;
  let first = null;
  let lastX = null;
  for (const p of pts) {
    if (!p) {
      open = false;
      continue;
    }
    d += `${open ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)} `;
    first ??= p[0];
    lastX = p[0];
    open = true;
  }
  if (area && first != null) d += `L${lastX.toFixed(1)} 40 L${first.toFixed(1)} 40 Z`;
  return d.trim();
}

function gauge(id, label) {
  return `<div class="sm-g" data-g="${id}">
    <svg viewBox="0 0 120 120" aria-hidden="true">
      <defs>
        <mask id="sm-m-${id}" maskUnits="userSpaceOnUse" x="0" y="0" width="120" height="120"><path class="sm-mask" d="${arc(R)}" pathLength="${SWEEP}" data-mask /></mask>
        <filter id="sm-f-${id}" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="2.2" result="b" /><feMerge><feMergeNode in="b" /><feMergeNode in="SourceGraphic" /></feMerge></filter>
      </defs>
      <circle class="sm-halo" cx="60" cy="60" r="30" />
      <circle class="sm-orbit" cx="60" cy="60" r="57" pathLength="360" />
      <circle class="sm-orbit2" cx="60" cy="60" r="52" pathLength="100" />
      <path class="sm-track" d="${arc(R)}" pathLength="${SWEEP}" />
      <g filter="url(#sm-f-${id})"><path class="sm-fill" d="${arc(R)}" pathLength="${SWEEP}" mask="url(#sm-m-${id})" /><circle class="sm-head" r="2.4" cx="${point(R, 135)[0].toFixed(2)}" cy="${point(R, 135)[1].toFixed(2)}" data-head /></g>
      <path class="sm-inner-track" d="${arc(36)}" pathLength="${SWEEP}" />
      <path class="sm-inner" d="${arc(36)}" pathLength="${SWEEP}" data-inner />
      <text class="sm-val" x="60" y="65" data-val>--</text>
      <text class="sm-lab" x="60" y="82">${label}</text>
    </svg>
    <div class="sm-sub" data-sub>&nbsp;</div>
  </div>`;
}

export function createSystemPanel({ el, tick: tickEl }) {
  let on = true;
  try {
    on = localStorage.getItem(KEY) !== "off";
  } catch {}
  let last = null;
  let stale = false;
  let timer = null;
  let busy = false;

  el.querySelector("[data-sm-body]").innerHTML = `
    <div class="sm-gauges">${gauge("cpu", "CPU")}${gauge("gpu", "GPU")}${gauge("ram", "RAM")}</div>
    <div class="sm-cores" data-cores title="Each core's use"></div>
    <div class="sm-trace sm-more">
      <svg viewBox="0 0 300 40" preserveAspectRatio="none" aria-hidden="true">
        <defs><linearGradient id="sm-tg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" class="sm-tg-a" /><stop offset="1" class="sm-tg-b" /></linearGradient></defs>
        <path class="sm-grid" d="M0 12.5H300M0 21H300M0 29.5H300M50 0V40M100 0V40M150 0V40M200 0V40M250 0V40" />
        <path class="sm-area" data-trace="area" />
        <path class="sm-line mem" data-trace="mem" />
        <path class="sm-line gpu" data-trace="gpu" />
        <path class="sm-line cpu" data-trace="cpu" />
      </svg>
      <span class="sm-sweep"></span>
      <div class="sm-trace-key"><span class="cpu">cpu</span><span class="gpu">gpu</span><span class="mem">ram</span><span class="sm-span" data-span></span></div>
    </div>
    <div class="sm-parts sm-more"><span data-part="cpu"></span><span data-part="gpu"></span></div>`;
  const $ = (s) => el.querySelector(s);

  function setGauge(id, v, { text, sub, inner = null, off = false }) {
    const g = $(`[data-g="${id}"]`);
    const val = v == null ? 0 : clamp(v);
    g.dataset.level = off ? "off" : level(v);
    g.querySelector("[data-mask]").style.strokeDasharray = `${(val * SWEEP).toFixed(2)} ${SWEEP}`;
    const [x, y] = point(R, 135 + val * SWEEP);
    const head = g.querySelector("[data-head]");
    head.setAttribute("cx", x.toFixed(2));
    head.setAttribute("cy", y.toFixed(2));
    head.style.opacity = v == null ? "0" : "1";
    const innerEl = g.querySelector("[data-inner]");
    innerEl.style.strokeDasharray = `${(clamp(inner) * SWEEP).toFixed(2)} ${SWEEP}`;
    innerEl.style.opacity = inner == null ? "0" : "1";
    g.querySelector("[data-val]").innerHTML = text;
    g.querySelector("[data-sub]").innerHTML = sub;
  }

  function render() {
    const s = last;
    // While the view is held still (driver.js still, for screenshots), the
    // panel keeps the reading it shows.
    if (!s || document.body.classList.contains("still")) return;
    const pct = (v) => (v == null ? "--" : `${Math.round(clamp(v) * 100)}<tspan dx="1" class="sm-pct">%</tspan>`);
    const cpu = s.cpu ?? {};
    setGauge("cpu", cpu.usage, { text: pct(cpu.usage), sub: `${cpu.threads ?? "?"}<small>T</small>${cpu.ghz ? ` · ${cpu.ghz}<small>GHz</small>` : ""}` });
    const g = s.gpus?.[0] ?? null;
    const vram = g?.memTotal ? clamp(g.memUsed / g.memTotal) : null;
    if (g)
      setGauge("gpu", g.usage, {
        text: pct(g.usage),
        inner: vram,
        sub: g.memTotal ? `${g.shared ? "" : "<small>VRAM</small> "}${gb(g.memUsed)}/${gb(g.memTotal)}<small>GB</small>` : g.temp != null ? `${g.temp}<small>°C</small>` : "&nbsp;",
      });
    else setGauge("gpu", null, { text: "--", sub: `<small>${s.gpu === "unavailable" ? "no reading" : "none found"}</small>`, off: true });
    const mem = s.memory?.total ? clamp(s.memory.used / s.memory.total) : null;
    setGauge("ram", mem, { text: pct(mem), sub: s.memory?.total ? `${gb(s.memory.used)}/${gb(s.memory.total)}<small>GB</small>` : "&nbsp;" });

    const cores = cpu.cores ?? [];
    const coresEl = $("[data-cores]");
    if (coresEl.children.length !== cores.length) coresEl.innerHTML = cores.map(() => "<i></i>").join("");
    cores.forEach((c, i) => {
      const bar = coresEl.children[i];
      bar.style.setProperty("--v", clamp(c).toFixed(3));
      bar.dataset.level = level(c);
    });

    const hist = s.history ?? [];
    $('[data-trace="area"]').setAttribute("d", tracePath(hist.map((x) => x.cpu), { area: true }));
    for (const k of ["cpu", "gpu", "mem"]) $(`[data-trace="${k}"]`).setAttribute("d", tracePath(hist.map((x) => x[k])));
    const span = hist.length > 1 ? Math.round((hist.at(-1).at - hist[0].at) / 60e3) : 0;
    $("[data-span]").textContent = span ? `−${span}m` : "";

    $('[data-part="cpu"]').innerHTML = cpu.model ? `<b>cpu</b>${h(cpu.model)}` : "";
    $('[data-part="gpu"]').innerHTML = g ? `<b>gpu</b>${h(g.name)}${g.temp != null ? ` <em>${g.temp}°C</em>` : ""}${s.gpus.length > 1 ? ` <em>+${s.gpus.length - 1}</em>` : ""}` : "";
    const up = uptimeText(s.uptime);
    if (tickEl.dataset.os !== s.platform) {
      tickEl.dataset.os = s.platform;
      tickEl.innerHTML = `${osMark(s.platform)}<span class="sm-up"></span>`;
    }
    tickEl.querySelector(".sm-up").textContent = up;
  }

  async function tick() {
    clearTimeout(timer);
    if (!on || document.hidden || busy) return;
    busy = true;
    try {
      const r = await fetch("/api/system");
      if (!r.ok) throw new Error(String(r.status));
      last = await r.json();
      stale = false;
      render();
    } catch {
      stale = true;
    }
    busy = false;
    el.classList.toggle("stale", stale);
    if (on && !document.hidden) timer = setTimeout(tick, EVERY);
  }

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) tick();
  });

  function show(value) {
    on = value;
    try {
      localStorage.setItem(KEY, on ? "on" : "off");
    } catch {}
    el.hidden = !on;
    if (on) tick();
    else clearTimeout(timer);
  }

  el.hidden = !on;
  tick();

  return {
    toggle: () => show(!on),
    show,
    get on() {
      return on;
    },
    // For the driver (src/driver.js): what the panel shows, read-only.
    get state() {
      const g = last?.gpus?.[0] ?? null;
      return {
        shown: !el.hidden,
        stale,
        at: last?.at ?? null,
        source: last?.gpu ?? null,
        platform: last?.platform ?? null,
        cpu: last ? { usage: last.cpu?.usage ?? null, threads: last.cpu?.threads ?? null, model: last.cpu?.model ?? null, cores: last.cpu?.cores?.length ?? 0 } : null,
        memory: last ? { used: last.memory?.used ?? null, total: last.memory?.total ?? null } : null,
        gpu: g ? { name: g.name, usage: g.usage, memUsed: g.memUsed, memTotal: g.memTotal } : null,
        history: last?.history?.length ?? 0,
        text: el.innerText.replace(/\s+/g, " ").trim(),
      };
    },
  };
}
