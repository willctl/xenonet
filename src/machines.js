// The Machines view: the load panel's larger view (system-panel.js), opened
// from its title, a machine's row or the palette, as Stats opens. Each
// machine of the network (or this computer, on its own) with its load now,
// what its parts are and its sessions on the map; the network's totals on
// top, and a table to compare them. Picking machines here is picking the
// Filters chips (hosts.js): one selection, which filters the map too. The
// platform, GPU and online filters and the sort are the view's own. It
// reads every two seconds while it's open, and stops when it closes.
import { escapeHtml as h } from "./format.js";
import { machinesOf, filterMachines, sortMachines, sessionsByMachine, hasGpu, OS_NAME, PLATFORMS, SORTS } from "./machines-model.js";
import { osMark, uptimeText, sinceText, gb, level, pctText, hostColor, WHY } from "./system-panel.js";
import { APP_LABEL } from "./theme.js";

const EVERY = 2000;
const SWEEP = 270;
const SORT_WORDS = { name: "Name", cpu: "CPU", gpu: "GPU", ram: "Memory", seen: "Last seen" };

// A small ring, as the panel's, for a card.
function ring(kind, label, v, sub = "") {
  const r = 20;
  const at = (deg) => [26 + r * Math.cos((deg * Math.PI) / 180), 26 + r * Math.sin((deg * Math.PI) / 180)];
  const [x1, y1] = at(135);
  const [x2, y2] = at(45);
  const d = `M ${x1.toFixed(2)} ${y1.toFixed(2)} A ${r} ${r} 0 1 1 ${x2.toFixed(2)} ${y2.toFixed(2)}`;
  const val = v == null ? 0 : Math.min(1, Math.max(0, v));
  return `<div class="mc-ring" data-ring="${kind}" data-level="${v == null ? "off" : level(v)}"><svg viewBox="0 0 52 52" aria-hidden="true"><path class="mc-track" d="${d}" pathLength="${SWEEP}" /><path class="mc-fill" d="${d}" pathLength="${SWEEP}" style="stroke-dasharray:${(val * SWEEP).toFixed(1)} ${SWEEP}" /><text x="26" y="29">${v == null ? "--" : `${Math.round(val * 100)}%`}</text></svg><span>${h(label)}</span><small>${sub || "&nbsp;"}</small></div>`;
}

// read(url): a GET, as fetch.
export function createMachinesView({ modals, sfx = () => {}, hosts, graph = () => null, net = () => null, read = (url) => fetch(url), onPick = () => {} }) {
  let panel = null;
  let timer = null;
  let reading = null;
  let model = null;
  let version = null;
  const opts = { view: "cards", sort: "name", platform: null, gpu: false, online: false };

  const isOpen = () => !!panel?.isConnected;
  // The machines picked, as the Filters chips have them.
  const picked = () => hosts?.selected ?? [];

  async function load() {
    const n = net();
    const r = await read(n?.network ? "/api/system?hosts=1" : "/api/system");
    if (!r.ok) throw new Error(`XenoNet answered ${r.status}`);
    reading = await r.json();
    model = machinesOf(reading, n, Date.now());
  }

  // What's shown, with the filters and the sort.
  const shown = () => (model ? sortMachines(filterMachines(model.machines, { keys: picked(), ...opts }), opts.sort) : []);

  // A machine's place in the map's view of hosts (hub's or presence's):
  // its version and since when it's been live, where those are known.
  const hostRow = (id) => (Array.isArray(graph()?.hosts) ? graph().hosts.find((x) => x.id === id) : null);

  const kpi = (k, v, s) => `<div class="st-kpi"><div class="st-k">${h(k)}</div><div class="st-v">${h(v)}</div><div class="st-s">${h(s)}</div></div>`;

  function kpis(m) {
    const t = m.totals;
    const gpus = t.gpu.count;
    return `<div class="st-kpis mc-kpis">
      ${kpi(m.network ? "Machines online" : "Machine", m.network ? `${t.online}/${t.machines}` : "1", m.network ? `${t.counted} with their load here` : "this computer")}
      ${kpi("Threads", String(t.threads), `${t.counted} ${t.counted === 1 ? "machine" : "machines"}`)}
      ${kpi("GPUs", String(gpus), t.gpu.vram ? `${gb(t.gpu.vram.total)} GB of their own memory` : gpus ? "sharing memory" : "none found")}
      ${kpi("Memory", `${gb(t.ram.total)} GB`, `${gb(t.ram.used)} GB in use`)}
      ${kpi("CPU now", pctText(t.cpu), "of every thread")}
      ${kpi("GPU now", gpus ? pctText(t.gpu.mean) : "--", gpus ? `mean of ${gpus} ${gpus === 1 ? "GPU" : "GPUs"}` : "no GPU")}
      ${kpi("RAM now", pctText(t.ram.share), `${gb(t.ram.used)}/${gb(t.ram.total)} GB`)}
    </div>`;
  }

  function filters(m) {
    const sel = new Set(picked());
    const chip = (attr, on, label, title = "") => `<button class="mc-chip${on ? " on" : ""}" ${attr} title="${h(title)}">${label}</button>`;
    const machines = m.network
      ? `<div class="mc-row"><span class="mc-k">Machines</span>${m.machines.map((x) => chip(`data-mc-machine="${h(x.key)}" style="${hostColor(x.color)}"`, sel.has(x.key), `<i class="sm-sw"></i>${h(x.id ?? "")}${x.here ? '<em class="here">here</em>' : ""}`, "The same as its chip under Filters: it filters the map too")).join("")}${sel.size ? `<button class="mini" data-mc-all title="Every machine, here and on the map">show all</button>` : ""}</div>`
      : "";
    const platforms = [chip(`data-mc-platform=""`, !opts.platform, "All"), ...PLATFORMS.map((p) => chip(`data-mc-platform="${p}"`, opts.platform === p, OS_NAME[p]))].join("");
    const sorts = SORTS.map((s) => chip(`data-mc-sort="${s}"`, opts.sort === s, SORT_WORDS[s])).join("");
    return `<div class="mc-filters">${machines}
      <div class="mc-row"><span class="mc-k">Platform</span>${platforms}<span class="mc-gap"></span>${chip("data-mc-gpu", opts.gpu, "Has a GPU")}${chip("data-mc-online", opts.online, "Online")}</div>
      <div class="mc-row"><span class="mc-k">Sort</span>${sorts}<span class="grow"></span><button class="st-toggle" data-mc-view>${opts.view === "table" ? "Cards" : "Table"}</button></div>
    </div>`;
  }

  function hardware(x) {
    const r = x.reading;
    if (!r) return `<p class="mc-none">${h(x.online ? WHY[x.why] : sinceText(x.seen))}: no reading of its parts here yet.</p>`;
    const hw = r.hardware;
    const cpu = [hw.cpu.model ?? r.cpu.model, hw.cpu.threads ?? r.cpu.threads ? `${hw.cpu.threads ?? r.cpu.threads} threads` : null, hw.cpu.ghz ?? r.cpu.ghz ? `${hw.cpu.ghz ?? r.cpu.ghz} GHz` : null].filter(Boolean).join(" · ");
    const gpus = r.gpus.length
      ? r.gpus
          .map((g) => {
            const mem = g.memTotal ? `${g.shared ? "shares" : ""} ${gb(g.memTotal)} GB${g.shared ? " with the CPU" : ""}`.trim() : null;
            const now = x.counted ? [pctText(g.usage), g.temp != null ? `${g.temp}°C` : null, !g.shared && g.memTotal && g.memUsed != null ? `${gb(g.memUsed)} GB in use` : null].filter(Boolean).join(" · ") : "";
            return `<dd>${h([g.name, mem].filter(Boolean).join(" · "))}${now ? ` <em>${h(now)}</em>` : ""}</dd>`;
          })
          .join("")
      : `<dd class="dim">${r.source === "unavailable" ? "no reading" : "none found"}</dd>`;
    const system = [OS_NAME[hw.platform ?? r.platform] ?? hw.platform ?? r.platform, hw.arch ?? r.arch, uptimeText(r.uptime)].filter(Boolean).join(" · ");
    return `<dl class="mc-hw"><dt>CPU</dt><dd>${h(cpu || "unknown")}</dd><dt>Memory</dt><dd>${h(`${gb(hw.memory.total ?? r.memory.total)} GB`)}</dd><dt>${r.gpus.length > 1 ? "GPUs" : "GPU"}</dt>${gpus}<dt>System</dt><dd>${h(system)}</dd></dl>`;
  }

  function sessions(x, counts) {
    const c = counts.get(x.key);
    const apps = c ? Object.entries(c.apps).sort((a, b) => b[1] - a[1]).map(([app, n]) => `${n} ${APP_LABEL[app] ?? app}`) : [];
    const words = c ? `${c.total} ${c.total === 1 ? "session" : "sessions"} on the map: ${apps.join(", ")}` : "No sessions on the map";
    return `<div class="mc-sessions"><span>${h(words)}</span>${model.network ? `<button class="act" data-mc-show="${h(x.key)}" title="Show only this machine's work on the map">Show on the map</button>` : ""}</div>`;
  }

  function card(x, counts) {
    const row = x.here ? null : hostRow(x.id);
    const tags = [x.window ? "window host" : model.network ? "no window" : null, x.gateway ? "gateway" : null, x.founder ? "founder" : null].filter(Boolean);
    const ver = x.here ? version : (row?.version ?? null);
    const since = x.online ? (x.here ? "here" : row?.since ? `online ${sinceText(row.since)}` : "online") : sinceText(x.seen);
    const r = x.reading;
    const rings = x.counted
      ? ring("cpu", "CPU", x.cpu, `${r.cpu.threads}T`) + ring("gpu", "GPU", r.gpus.length ? x.gpu : null, r.gpus.length > 1 ? `${r.gpus.length} GPUs` : "") + ring("ram", "RAM", x.ram, `${gb(r.memory.used)}/${gb(r.memory.total)}`)
      : `<p class="mc-none">${h(!x.online ? `Offline, ${sinceText(x.seen)}. Not in the sums.` : r ? "No recent reading: not in the sums." : `${WHY[x.why]}.`)}</p>`;
    return `<section class="st-card mc-card${x.online ? "" : " off"}" data-mc-card="${h(x.key)}" style="${hostColor(x.color)}">
      <header><i class="sm-sw"></i>${osMark(x.platform, "sm-os mc-os")}<h3>${h(x.id ?? "This computer")}</h3>${x.here && model.network ? '<em class="here">here</em>' : ""}<span class="grow"></span><span class="st-dim">${h(since)}</span></header>
      <div class="mc-meta">${h([...tags, ver ? `XenoNet ${ver}` : null].filter(Boolean).join(" · "))}</div>
      <div class="mc-rings">${rings}</div>
      ${hardware(x)}
      ${sessions(x, counts)}
    </section>`;
  }

  function table(list, counts) {
    const head = ["Machine", "Online", "Platform", "Arch", "CPU", "Threads", "GHz", "CPU now", "GPUs", "GPU now", "VRAM", "Memory", "In use", "RAM now", "Uptime", "Sessions"];
    const rows = list.map((x) => {
      const r = x.reading;
      const vram = r ? r.gpus.filter((g) => !g.shared && g.memTotal).reduce((s, g) => s + g.memTotal, 0) : 0;
      return [
        `${x.id ?? "this computer"}${x.here && model.network ? " (here)" : ""}`,
        x.online ? "yes" : sinceText(x.seen),
        OS_NAME[x.platform] ?? x.platform ?? "",
        r?.arch ?? "",
        r?.cpu.model ?? "",
        r ? String(r.cpu.threads) : "",
        r?.cpu.ghz != null ? String(r.cpu.ghz) : "",
        x.counted ? pctText(x.cpu) : "",
        r ? r.gpus.map((g) => g.name).join(", ") : "",
        x.counted && r.gpus.length ? pctText(x.gpu) : "",
        vram ? `${gb(vram)} GB` : "",
        r ? `${gb(r.memory.total)} GB` : "",
        x.counted ? `${gb(r.memory.used)} GB` : "",
        x.counted ? pctText(x.ram) : "",
        r ? uptimeText(r.uptime) : "",
        String(counts.get(x.key)?.total ?? 0),
      ];
    });
    return `<section class="st-card wide as-table mc-table"><div class="st-tablewrap"><table class="st-table"><thead><tr>${head.map((c) => `<th>${h(c)}</th>`).join("")}</tr></thead><tbody>${rows.map((r, i) => `<tr data-mc-row="${h(list[i].key)}">${r.map((c) => `<td>${h(c)}</td>`).join("")}</tr>`).join("")}</tbody></table></div></section>`;
  }

  function render() {
    if (!isOpen() || !model) return;
    const counts = sessionsByMachine(graph());
    const list = shown();
    const m = model;
    panel.querySelector(".m-sub").textContent = m.network ? `${m.totals.machines} machines in ${net()?.network?.name ?? "your network"} · ${m.totals.online} online · ${list.length} shown` : "This computer: its load, and what its parts are";
    const scroll = panel.querySelector(".m-body").scrollTop;
    panel.querySelector(".m-body").innerHTML = `${kpis(m)}${filters(m)}
      ${list.length ? (opts.view === "table" ? table(list, counts) : `<div class="mc-cards">${list.map((x) => card(x, counts)).join("")}</div>`) : `<div class="m-empty">No machine fits these filters.</div>`}
      <p class="st-note">${h(m.network ? (m.windowed ? "Each machine's load comes with its map, every few seconds, while this console watches it, unless that machine turned off Share load with the network. An offline machine, or one with no recent reading, isn't in the sums." : "This machine isn't a window host: other machines' load comes only to window hosts' consoles, so only this one's is here.") : "Join a network (Network settings) to see your other machines here too.")}</p>`;
    panel.querySelector(".m-body").scrollTop = scroll;
  }

  async function refresh() {
    if (!isOpen()) return stop();
    try {
      await load();
      render();
    } catch (err) {
      if (isOpen() && !model) panel.querySelector(".m-body").innerHTML = `<div class="m-empty">${h(err.message)}</div>`;
    }
  }

  function stop() {
    clearInterval(timer);
    timer = null;
  }

  function onClick(e) {
    const t = e.target.closest("[data-mc-machine], [data-mc-all], [data-mc-platform], [data-mc-gpu], [data-mc-online], [data-mc-sort], [data-mc-view], [data-mc-show]");
    if (!t) return;
    if (t.dataset.mcMachine != null) hosts?.toggle(t.dataset.mcMachine);
    else if (t.hasAttribute("data-mc-all")) hosts?.setSelected([]);
    else if (t.dataset.mcPlatform != null) opts.platform = t.dataset.mcPlatform || null;
    else if (t.hasAttribute("data-mc-gpu")) opts.gpu = !opts.gpu;
    else if (t.hasAttribute("data-mc-online")) opts.online = !opts.online;
    else if (t.dataset.mcSort) opts.sort = t.dataset.mcSort;
    else if (t.hasAttribute("data-mc-view")) opts.view = opts.view === "table" ? "cards" : "table";
    else if (t.dataset.mcShow) {
      // Only this machine's work on the map, as its chip under Filters.
      hosts?.setSelected([t.dataset.mcShow]);
      onPick(t.dataset.mcShow);
      return modals.close();
    }
    if (!t.dataset.mcMachine && !t.hasAttribute("data-mc-all")) sfx("select");
    render();
  }

  // Opens it, at a machine (its card first) or not; resolves once the
  // first reading is on screen.
  async function open(at = null) {
    stop();
    model = null;
    panel = modals.open(
      `<header class="m-head"><div><div class="m-chip">▦ machines</div><h2>${net()?.network ? "Machines" : "This computer"}</h2><div class="m-sub">Reading each machine's load…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body st-body mc-body"><div class="m-empty">Reading the load…</div></div>`,
      { wide: true, cls: "full st machines", onClose: stop },
    );
    panel.addEventListener("click", onClick);
    if (version == null)
      read("/api/health")
        .then((r) => (r.ok ? r.json() : null))
        .then((v) => {
          version = v?.version ?? null;
          render();
        })
        .catch(() => {});
    await refresh();
    if (at) panel?.querySelector(`[data-mc-card="${CSS.escape(at)}"]`)?.scrollIntoView({ block: "nearest" });
    timer = setInterval(refresh, EVERY);
  }

  // For the driver and the console tool: filters as { machines: [keys],
  // platform, gpu, online }, a sort and a view.
  function set({ filter, sort, view } = {}) {
    if (filter) {
      if (Array.isArray(filter.machines)) hosts?.setSelected(filter.machines);
      if ("platform" in filter) {
        if (filter.platform != null && filter.platform !== "" && !PLATFORMS.includes(filter.platform)) throw new Error(`platform is one of ${PLATFORMS.join(", ")}`);
        opts.platform = filter.platform || null;
      }
      if ("gpu" in filter) opts.gpu = !!filter.gpu;
      if ("online" in filter) opts.online = !!filter.online;
    }
    if (sort != null) {
      if (!SORTS.includes(sort)) throw new Error(`sort by ${SORTS.join(", ")}`);
      opts.sort = sort;
    }
    if (view != null) {
      if (!["cards", "table"].includes(view)) throw new Error("view is cards or table");
      opts.view = view;
    }
    render();
  }

  return {
    open,
    set,
    close: () => isOpen() && modals.close(),
    get isOpen() {
      return isOpen();
    },
    // What it shows, for the driver: read-only.
    get state() {
      if (!isOpen()) return { open: false, view: opts.view, sort: opts.sort, filter: { machines: picked(), platform: opts.platform, gpu: opts.gpu, online: opts.online } };
      const list = shown();
      const counts = sessionsByMachine(graph());
      const t = model?.totals;
      return {
        open: true,
        view: opts.view,
        sort: opts.sort,
        filter: { machines: picked(), platform: opts.platform, gpu: opts.gpu, online: opts.online },
        network: !!model?.network,
        kpis: t ? { machines: t.machines, online: t.online, counted: t.counted, threads: t.threads, gpus: t.gpu.count, ram: t.ram.total, cpu: t.cpu, gpu: t.gpu.mean, ramShare: t.ram.share } : null,
        shown: list.map((x) => x.id ?? null),
        cards: list.map((x) => ({
          id: x.id,
          key: x.key,
          here: x.here,
          online: x.online,
          counted: x.counted,
          platform: x.platform,
          window: x.window,
          gateway: x.gateway,
          founder: x.founder,
          gpu: hasGpu(x),
          hardware: x.reading?.hardware ?? null,
          sessions: counts.get(x.key)?.total ?? 0,
        })),
        text: panel.innerText.replace(/\s+/g, " ").trim(),
      };
    },
  };
}
