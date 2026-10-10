// Identities on the page (see server/identities.js): the chips under
// Filters, the inspector's Identities section, the rings that named
// identities' projects stand on, and masking.
//
// Identity text is never drawn on the map's canvas, only colors, so posters,
// replay clips and wallpaper mode can't carry an email or a handle. "Hide
// identities" masks them in the panels too, as "GitHub A", "Claude B" and so
// on, for screenshots and screen sharing. It's remembered, and `?private` or
// XENONET_HIDE_IDENTITIES=1 start with it on.
import { escapeHtml as h } from "./format.js";
import { RING_COLORS } from "./theme.js";

export const KIND_LABEL = { claude: "Claude", chatgpt: "ChatGPT", openai: "OpenAI", github: "GitHub", git: "git" };
const ROLE_LABEL = { "ran-as": "ran as", owner: "owner", "commits-as": "commits as", "pushes-as": "pushes as" };
const CONF_LABEL = { yours: "your pin", exact: "recorded", likely: "likely", assumed: "assumed" };
const MASK_KEY = "xenonet.hideIdentities";
const MORE_KEY = "xenonet.identities.more";
// Chips shown until you ask for the rest, so Filters doesn't crowd out the
// activity feed.
const FEW = 4;

const letter = (n) => String.fromCharCode(65 + (n % 26)) + (n >= 26 ? Math.floor(n / 26) : "");

// A node's identity links. A session has the account it ran as, plus its
// project's (marked `inherited`); a project has its own.
export function linksOf(node, byId, parent) {
  if (!node) return [];
  if (node.kind === "project") return node.identities ?? [];
  if (node.kind !== "session") return [];
  const own = node.ranAs ? [{ ...node.ranAs, role: "ran-as" }] : [];
  for (let cur = parent.get(node.id), i = 0; cur && i < 12; cur = parent.get(cur), i++) {
    const n = byId.get(cur);
    if (n?.kind === "project") return [...own, ...(n.identities ?? []).map((l) => ({ ...l, inherited: true }))];
  }
  return own;
}

// The sessions and projects linked to any of `ids`: a session through its
// own account or its project, a project through its own links.
export function passing(nodes, parent, ids) {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const pass = new Set();
  for (const n of nodes) if ((n.kind === "session" || n.kind === "project") && linksOf(n, byId, parent).some((l) => ids.has(l.id))) pass.add(n.id);
  return pass;
}

export function createIdentities({ sfx = () => {}, toast = () => {}, onFilter = () => {}, onLens = () => {}, onChange = () => {} } = {}) {
  let list = [];
  let byId = new Map();
  let groups = [];
  let groupOf = new Map();
  let letters = new Map();
  let groupLetters = new Map();
  let selected = new Set(); // group keys shown; empty shows everything
  let seeded = false;
  let lensKey = null;
  let masked = false;
  let graphNow = null;
  let more = false;
  try {
    more = localStorage.getItem(MORE_KEY) === "on";
  } catch {}
  try {
    masked = localStorage.getItem(MASK_KEY) === "on" || new URLSearchParams(location.search).has("private");
  } catch {}
  document.body.classList.toggle("ids-masked", masked);

  // ---------- data ----------

  function setData(graph) {
    // XENONET_HIDE_IDENTITIES=1 starts masked, once; your toggle wins after.
    if (!seeded && graph.hideIdentities && !masked) setMasked(true, { quiet: true });
    seeded = true;
    graphNow = graph;
    list = graph.identities ?? [];
    byId = new Map(list.map((i) => [i.id, i]));
    // Identities with the same name are one group.
    const g = new Map();
    for (const i of list) {
      const key = i.name ? `name:${i.name.toLowerCase()}` : i.id;
      if (!g.has(key)) g.set(key, { key, name: i.name ?? null, ids: [], kinds: new Set(), hidden: true, color: null, ring: null });
      const grp = g.get(key);
      grp.ids.push(i.id);
      grp.kinds.add(i.kind);
      if (!i.hidden) grp.hidden = false;
    }
    groups = [...g.values()];
    // Each name keeps its ring as others come and go: the one its hash
    // picks, or the next one free. The ring colors are the theme's (see
    // ringColors in core/themes.js, clear of the colors that mean something
    // on the map), so a name keeps its place and the panels follow a change
    // of theme through var(--ring-N).
    const taken = new Set();
    for (const x of groups.filter((x) => x.name && !x.hidden).sort((a, b) => a.name.localeCompare(b.name))) {
      const start = [...x.name.toLowerCase()].reduce((n, c) => (n * 31 + c.charCodeAt(0)) >>> 0, 7) % RING_COLORS.length;
      let k = start;
      while (taken.has(k) && taken.size < RING_COLORS.length) k = (k + 1) % RING_COLORS.length;
      taken.add(k);
      x.ring = k;
      x.color = `var(--ring-${k})`;
    }
    groupOf = new Map(groups.flatMap((x) => x.ids.map((id) => [id, x])));
    // Masked letters, stable while the set of identities is.
    letters = new Map();
    const per = {};
    for (const i of [...list].sort((a, b) => a.id.localeCompare(b.id))) letters.set(i.id, letter((per[i.kind] = (per[i.kind] ?? -1) + 1)));
    groupLetters = new Map([...groups].sort((a, b) => a.key.localeCompare(b.key)).map((x, k) => [x.key, letter(k)]));
    for (const key of selected) if (!g.has(key)) selected.delete(key);
  }

  // Named identities' projects stand on a ring in the name's color (see
  // nodes.js): the owner's name, else the commits' or the pushes'.
  function decorate(nodes) {
    for (const n of nodes) {
      if (n.kind !== "project") continue;
      const links = n.identities ?? [];
      const grp = ["owner", "commits-as", "pushes-as"].map((r) => groupOf.get(links.find((l) => l.role === r)?.id)).find((x) => x?.color);
      n.identityRing = grp?.ring ?? null;
    }
  }

  // ---------- names ----------

  const handleOf = (i) => (i.kind === "openai" ? "API key" : i.handle ?? `${KIND_LABEL[i.kind] ?? i.kind} account ${String(i.id.split(":")[1] ?? "").slice(0, 8)}`);
  function label(id) {
    const i = byId.get(id);
    if (!i) return masked ? "an identity" : "an unknown identity";
    if (masked) return `${KIND_LABEL[i.kind] ?? i.kind} ${letters.get(id)}`;
    return i.name ?? handleOf(i);
  }
  function groupLabel(grp) {
    if (masked) return grp.ids.length === 1 ? label(grp.ids[0]) : `Identity ${groupLetters.get(grp.key)}`;
    return grp.name ?? handleOf(byId.get(grp.ids[0]));
  }
  // Plan, organization, the name git commits with, and whether it's signed in.
  function detailOf(i) {
    if (masked) return [i.signedIn ? "signed in" : null, i.active ? "gh's active account" : null].filter(Boolean).join(" · ");
    const d = i.detail ?? {};
    return [d.plan && `${d.plan} plan`, d.org, i.kind === "git" && d.name ? `as ${d.name}` : null, d.host && d.host !== "github.com" ? d.host : null, i.signedIn ? "signed in" : null, i.active ? "gh's active account" : null].filter(Boolean).join(" · ");
  }
  function glyph(i) {
    const grp = groupOf.get(i.id);
    return grp?.color ? `<i class="idg named" style="--c:${grp.color}"></i>` : `<i class="idg ${h(i.kind)}"></i>`;
  }

  // An identity as a link that filters the map to it.
  function chipLink(id) {
    const i = byId.get(id);
    if (!i) return `<span class="dim">${h(label(id))}</span>`;
    const grp = groupOf.get(id);
    const name = !masked && i.name ? ` <span class="tag id-tag" style="--c:${grp?.color ?? "var(--muted)"}">${h(i.name)}</span>` : "";
    return `<a href="#" class="id-link" data-identity="${h(grp?.key ?? id)}" title="${h(detailOf(i) || KIND_LABEL[i.kind])}">${glyph(i)}${h(masked || !i.name ? label(id) : handleOf(i))}</a>${name}`;
  }

  // ---------- mismatches ----------

  // A push that signs in as one of your GitHub accounts into a repo another
  // of yours owns, or commits under one name into a repo under another. (A
  // push into an organization's repo is always some user's, so that's fine.)
  function mismatches(links) {
    const of = (role) => links.find((l) => l.role === role);
    const owner = byId.get(of("owner")?.id);
    const push = byId.get(of("pushes-as")?.id);
    const commit = byId.get(of("commits-as")?.id);
    const nameOf = (i) => (i ? groupOf.get(i.id)?.name?.toLowerCase() ?? null : null);
    const out = [];
    if (owner?.kind === "github" && owner.signedIn && push && push.id !== owner.id) out.push(`pushes as ${label(push.id)}, but ${label(owner.id)} owns it`);
    if (commit && owner && nameOf(commit) && nameOf(owner) && nameOf(commit) !== nameOf(owner)) out.push(`commits as ${label(commit.id)}, but ${label(owner.id)} owns it`);
    return out;
  }

  // Every project with a mismatch, for the identities window.
  function allMismatches() {
    if (!graphNow) return [];
    return graphNow.nodes
      .filter((n) => n.kind === "project")
      .map((n) => ({ id: n.id, name: n.name, texts: mismatches(n.identities ?? []) }))
      .filter((w) => w.texts.length);
  }

  // ---------- the inspector ----------

  function sectionHtml(node, { index, parent }) {
    if (!list.length || !(node?.kind === "session" || node?.kind === "project")) return "";
    const links = linksOf(node, index, parent);
    const row = (k, v) => `<div class="kv"><span class="k">${h(k)}</span><span class="v">${v}</span></div>`;
    const how = (l) => `<span class="dim"> · ${h(CONF_LABEL[l.confidence] ?? l.confidence)}${l.via && l.via !== "your pin" ? `, ${h(l.via)}` : ""}</span>`;
    let rows = "";
    if (node.kind === "session") {
      const ran = links.find((l) => l.role === "ran-as");
      rows += row("ran as", ran ? chipLink(ran.id) + how(ran) : `<span class="dim">not known${node.app === "codex" ? ": Codex doesn't record the account" : ""}</span>`);
    }
    for (const l of links.filter((x) => x.role !== "ran-as")) rows += row(node.kind === "session" && l.role === "owner" ? "repo owner" : ROLE_LABEL[l.role], chipLink(l.id) + how(l));
    const warn = mismatches(links).map((t) => `<div class="id-warn">⚠ ${h(t)}</div>`).join("");
    return `<div class="panel-title sub"><span>Identities</span><span class="tick">${masked ? "masked" : ""}</span></div>${warn}<div class="grid">${rows}</div>`;
  }

  // ---------- the filter ----------

  const idsOf = (keys) => new Set([...keys].flatMap((k) => groups.find((x) => x.key === k)?.ids ?? []));
  // What the filter lets through, or null while it's off.
  function filterPass(nodes, parent) {
    return selected.size ? passing(nodes, parent, idsOf(selected)) : null;
  }
  function lensPass(nodes, parent, key) {
    const pass = passing(nodes, parent, idsOf([key]));
    for (const id of [...pass]) for (let p = parent.get(id); p && !pass.has(p); p = parent.get(p)) pass.add(p);
    return pass;
  }

  function setSelected(keys) {
    selected = new Set(keys.filter((k) => groups.some((x) => x.key === k)));
    sfx("toggle");
    renderFilters();
    onFilter();
  }
  function toggle(key) {
    const next = new Set(selected);
    next.has(key) ? next.delete(key) : next.add(key);
    setSelected([...next]);
  }
  const only = (key) => setSelected([key]);

  function setMasked(on, { quiet = false } = {}) {
    masked = !!on;
    try {
      localStorage.setItem(MASK_KEY, masked ? "on" : "off");
    } catch {}
    document.body.classList.toggle("ids-masked", masked);
    if (quiet) return;
    sfx("toggle");
    toast(masked ? "Identities hidden: shown as GitHub A, Claude B…" : "Identities shown");
    onChange();
  }

  // The chips under Filters: one per identity, or per name you gave several,
  // that reaches something on the map, with how many sessions it reaches.
  let filtersEl = null;
  let lastCtx = null;
  let lastReach = new Map();
  let shownSessions = null;
  function renderFilters(el, ctx) {
    filtersEl = el ?? filtersEl;
    lastCtx = ctx ?? lastCtx;
    if (!filtersEl || !lastCtx) return;
    const { nodes, parent } = lastCtx;
    const byNode = new Map(nodes.map((n) => [n.id, n]));
    const reach = new Map();
    for (const n of nodes) {
      if (n.kind !== "session" && n.kind !== "project") continue;
      for (const key of new Set(linksOf(n, byNode, parent).map((l) => groupOf.get(l.id)?.key).filter(Boolean))) {
        const c = reach.get(key) ?? { sessions: 0, projects: 0 };
        c[n.kind === "session" ? "sessions" : "projects"]++;
        reach.set(key, c);
      }
    }
    lastReach = reach;
    shownSessions = selected.size ? [...filterPass(nodes, parent)].filter((id) => byNode.get(id)?.kind === "session").length : null;
    // Named ones first, then the ones that reach the most sessions.
    const ordered = groups
      .filter((x) => !x.hidden && (reach.has(x.key) || selected.has(x.key)))
      .sort((a, b) => !!b.name - !!a.name || (reach.get(b.key)?.sessions ?? 0) - (reach.get(a.key)?.sessions ?? 0) || groupLabel(a).localeCompare(groupLabel(b)));
    const shown = more ? ordered : ordered.filter((x, i) => i < FEW || selected.has(x.key));
    const rest = ordered.length - shown.length;
    filtersEl.hidden = !list.length;
    if (!list.length) return;
    // A Claude and a ChatGPT account (and a git identity) often share an
    // email; their chips say which is which.
    const text = new Map(ordered.map((x) => [x.key, groupLabel(x)]));
    const seen = [...text.values()].reduce((m, t) => m.set(t, (m.get(t) ?? 0) + 1), new Map());
    const kindOf = (x) => (seen.get(text.get(x.key)) > 1 && x.ids.length === 1 ? KIND_LABEL[byId.get(x.ids[0]).kind] : null);
    const chip = (x) => {
      const c = reach.get(x.key) ?? { sessions: 0, projects: 0 };
      const kinds = masked ? "" : x.ids.map((id) => `${KIND_LABEL[byId.get(id).kind]} ${handleOf(byId.get(id))}`).join(" · ");
      const title = `${kinds ? `${kinds}\n` : ""}${c.sessions} sessions · ${c.projects} projects on the map. Click to show only its work; hover to light it up.`;
      const g = x.color ? `<i class="idg named"></i>` : `<i class="idg ${h([...x.kinds][0])}"></i>`;
      return `<button class="id-chip ${selected.has(x.key) ? "on" : ""}" data-id-group="${h(x.key)}" title="${h(title)}" ${x.color ? `style="--c:${x.color}"` : ""}>${g}<span class="id-label">${h(text.get(x.key))}</span>${kindOf(x) ? `<span class="id-kind">${h(kindOf(x))}</span>` : ""}<span class="count">${c.sessions}</span></button>`;
    };
    filtersEl.innerHTML = `
      <div class="id-head">
        <span class="sub-k">Identities</span>
        ${selected.size ? `<button class="mini" data-id-clear title="Show every identity's work">show all</button>` : ""}
        <button class="mini id-mask ${masked ? "on" : ""}" data-id-mask title="${masked ? "Show identities" : "Hide identities: show them as GitHub A, Claude B… for screenshots and screen sharing"}">${masked ? "◑ hidden" : "◐ hide"}</button>
      </div>
      <div class="id-chips ${selected.size ? "filtering" : ""}">${shown.map(chip).join("") || `<span class="dim">none on the map</span>`}${rest > 0 ? `<button class="id-chip id-more" data-id-more title="Show every identity">+${rest}</button>` : more && ordered.length > FEW ? `<button class="id-chip id-more" data-id-more title="Show fewer">fewer</button>` : ""}</div>`;
  }

  function wire(el) {
    filtersEl = el;
    el.addEventListener("click", (e) => {
      const chip = e.target.closest("[data-id-group]");
      if (chip) return toggle(chip.dataset.idGroup);
      if (e.target.closest("[data-id-clear]")) return setSelected([]);
      if (e.target.closest("[data-id-more]")) {
        more = !more;
        try {
          localStorage.setItem(MORE_KEY, more ? "on" : "off");
        } catch {}
        sfx("toggle");
        return renderFilters();
      }
      if (e.target.closest("[data-id-mask]")) return setMasked(!masked);
    });
    // Hovering a chip lights its sessions and projects and dims the rest.
    el.addEventListener("mouseover", (e) => {
      const key = e.target.closest("[data-id-group]")?.dataset.idGroup ?? null;
      if (key === lensKey) return;
      lensKey = key;
      onLens(key && lastCtx ? lensPass(lastCtx.nodes, lastCtx.parent, key) : null);
    });
    el.addEventListener("mouseleave", () => {
      if (lensKey === null) return;
      lensKey = null;
      onLens(null);
    });
  }
  // An identity in the inspector (or the identities window) filters to it.
  document.addEventListener("click", (e) => {
    const a = e.target.closest("[data-identity]");
    if (!a) return;
    e.preventDefault();
    only(a.dataset.identity);
    toast(`Showing only ${groupLabel(groups.find((x) => x.key === a.dataset.identity) ?? { ids: [], key: "" })}'s work`);
  });

  return {
    setData,
    decorate,
    wire,
    renderFilters,
    sectionHtml,
    allMismatches,
    filterPass,
    label,
    groupLabel,
    handleOf,
    detailOf,
    glyph,
    setMasked,
    toggleMask: () => setMasked(!masked),
    setSelected,
    only,
    get masked() {
      return masked;
    },
    get list() {
      return list;
    },
    get groups() {
      return groups;
    },
    get selected() {
      return [...selected];
    },
    // For the driver: what each chip reaches on the map, and how many
    // sessions the filter lets through (null while it's off).
    reachOf: (key) => lastReach.get(key) ?? { sessions: 0, projects: 0 },
    get shownSessions() {
      return shownSessions;
    },
  };
}
