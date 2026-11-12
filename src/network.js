// The network across machines in the console (docs/design/network.md,
// sections 4, 7 and 8): Settings → Network, which the palette calls
// Network settings, and Add a machine.
//
//   - On its own, a machine offers Start a network (this machine is its
//     founder) and Join one (the line Add a machine shows elsewhere, run
//     here in a terminal, or its invite and code here).
//   - In a network: its name and this machine, how this machine stands in
//     it, each machine (online or since when, its agents and what waits on
//     you, what's held for it, founder, window, where you're at) with
//     Remove and Replace, other machines' agents under them with Message
//     (with the text of what waits on you, or why one seems stuck, as
//     their machine shares it, escaped as transcripts are), the invites
//     open, a roster mismatch, and "Share with the network: titles only"; your
//     devices, with Revoke for a lost machine's, and the services this
//     machine publishes, with Publish a service (network-person.js).
//   - Add a machine asks the new machine's id and whether you'll use a
//     XenoNet window on it, then shows the join line once, masked as a
//     token is (secret.js): out of the page's text until Show, then for ten
//     seconds, and masked again when the window loses focus or the page is
//     held still for a screenshot. XenoNet keeps only the code's hash.
//
// The state comes from GET /api/net/state, read again every few seconds in
// a network, rarely out of one, and whenever the event stream says it
// changed (net). Other machines' agents come from the engine's list (GET
// /api/engine/agents), by their agt_ and machine. Starting, joining, invites,
// removing, replacing and the setting go through the window's own calls,
// with its key: they're yours, and marked so (data-yours). A machine on its
// own never sees any of this outside this panel and Add a machine.
import { escapeHtml as h, tildePath } from "./format.js";
import { hostOfId } from "./hosts.js";
import { ARM_MS } from "./actor-requests.js";
import { keepSecret, forgetSecrets, secretHtml, secretClick, hideOnBlur, secretValue } from "./secret.js";
import { personOf, servicesOf, devicesHtml, servicesHtml, publishHtml, publishBody } from "./network-person.js";
import { contextOf, contextHtml, approveBody, signBody } from "./network-context.js";
import { stateOf, networksOf, inNetwork, clock, linkWords, hostLine, hostTags, inviteWords, proposalWords, frozenWords, remoteAgents, remoteWaits, hostIdProblem, joinOf, joinFields, INVITE_MS, STARTS, WAKES, START_WORDS, WAKE_WORDS, reachOf, hostCan, widenWords, delegationWords } from "./network-model.js";

const EVERY_IN = 10e3;
// What a session starts as, in the Start a session box.
const START_APPS = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
  ["grok", "Grok Build"],
  ["hermes-agent", "Hermes"],
];
// The apps that start under their own settings: no mode.
const NO_MODE = new Set(["codex", "hermes-agent"]);
const START_MODES = [
  ["", "Your settings"],
  ["plan", "Plan"],
  ["acceptEdits", "Accept edits"],
  ["auto", "Auto"],
  ["dontAsk", "Don't ask"],
];
const EVERY_OUT = 60e3;
const SECRET = "net-join";

// The panel's looks, the Actors panel's and its own, loaded the first time
// it opens, after the page's own, as the Actors panel's are.
let loadingStyles = null;
const styles = () => (loadingStyles ??= Promise.all([import("./actors.css"), import("./network.css")]).catch(() => {}));

// `calls` are the window's own (actor-requests.js); `message(id)` opens the
// box that writes to a session as you; `onChange(view)` hears each new read.
export function createNetwork({ modals, calls, sfx = () => {}, toast = () => {}, onChange = () => {}, message = () => {}, projects = () => [], request = (...a) => globalThis.fetch(...a) }) {
  let state = stateOf(null);
  // The network the panel shows, by id, when this machine is in several:
  // the first until you pick another.
  let chosen = null;
  // This machine's confirmed checkouts' projects, for what's shared into
  // each network.
  let shareable = [];
  let agents = [];
  // What this machine publishes to the network (GET /api/net/svc).
  let services = [];
  // Xenolith's asks for a grant and the grants you gave (GET
  // /api/net/forwards).
  let context = contextOf(null);
  let read = false;
  let seq = 0;
  let timer = null;
  let panel = null;
  let view = { name: "main" };
  let drawn = "";
  let busy = false;
  // The join line on screen: who it's for and when it lapses. The line
  // itself is kept as a secret (secret.js), never here, but for its code,
  // which the driver's text masks.
  let joinCode = null;
  // The last start sent from the box, and the answers to starts you asked
  // for (the start event), newest first: for the driver.
  let lastStart = null;
  const answers = [];

  const isOpen = () => !!panel?.isConnected;
  const body = () => panel?.querySelector(".m-body");

  function schedule() {
    clearTimeout(timer);
    timer = setTimeout(() => void refresh().catch(() => {}), inNetwork(state) ? EVERY_IN : EVERY_OUT);
  }

  // A read, with the window's key where this window has it: the network's
  // routes are the owner's.
  async function getJson(path) {
    if (await calls.ready?.()) {
      const r = await calls.call("GET", path);
      return r.ok ? r.data : null;
    }
    const res = await request(path);
    return res.ok ? res.json() : null;
  }

  // Other machines' agents, as the engine has them: in its state, beside
  // this machine's, and else in its list of agents at work.
  async function engineAgents() {
    const state = await getJson("/api/engine/state").catch(() => null);
    const list = Array.isArray(state?.agents) ? state.agents : [];
    if (list.some((a) => typeof a?.host === "string" && a.host)) return list;
    const at = await getJson("/api/engine/agents").catch(() => null);
    return Array.isArray(at) ? at : list;
  }

  // A call to the network's routes for the network the panel shows: what
  // acts on one network names it, since a machine may be in several.
  const MACHINE_ROUTES = new Set(["/api/net/state", "/api/net/create", "/api/net/join", "/api/net/devices/revoke", "/api/net/forwards/approve", "/api/net/forwards/deny", "/api/net/forwards/revoke"]);
  function netCall(method, path, body = {}) {
    const named = path.startsWith("/api/net/") && !MACHINE_ROUTES.has(path) && state.network?.id && body && typeof body === "object" && !("network" in body);
    return calls.call(method, path, named ? { ...body, network: state.network.id } : body);
  }

  // The projects of this machine's confirmed checkouts, by id.
  async function projectsHere() {
    const r = await getJson("/api/checkouts").catch(() => null);
    const seen = new Map();
    for (const c of Array.isArray(r?.checkouts) ? r.checkouts : []) {
      if (c && typeof c.project === "string" && c.project.startsWith("prj_") && c.confirmed !== false && !c.gone && !seen.has(c.project)) seen.set(c.project, { id: c.project, path: typeof c.path === "string" ? c.path : c.project });
    }
    return [...seen.values()];
  }

  // The state and other machines' agents, read again.
  async function refresh() {
    const n = ++seq;
    const raw = await getJson("/api/net/state").catch(() => null);
    if (chosen && !networksOf(raw).some((x) => x.id === chosen)) chosen = null;
    const next = stateOf(raw, chosen);
    const list = inNetwork(next) ? await engineAgents() : [];
    const svcPath = next.network?.id ? `/api/net/svc?network=${encodeURIComponent(next.network.id)}` : "/api/net/svc";
    const published = inNetwork(next) ? servicesOf(await getJson(svcPath).catch(() => null)) : [];
    const projects = next.networks.length ? await projectsHere() : [];
    const forwards = next.networks.length ? contextOf(await getJson("/api/net/forwards").catch(() => null)) : contextOf(null);
    if (n !== seq) return state;
    state = next;
    agents = remoteAgents(list);
    services = published;
    context = forwards;
    shareable = projects;
    read = true;
    schedule();
    onChange(viewOf());
    render();
    return state;
  }

  const viewOf = () => ({ state, agents, waits: remoteWaits(state, agents) });

  // ---------- drawing ----------

  const subText = () => (!read ? "Reading…" : inNetwork(state) ? `${state.network.name}${state.self ? ` · this machine is ${state.self.id}` : ""}` : "This machine only");
  const head = () => `<header class="m-head"><div><div class="m-chip">⬡ network</div><h2>Network</h2><div class="m-sub" data-net-sub>${h(subText())}</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"></div>`;

  const errorHtml = (e) => `<p class="ac-error bad" data-net-error ${e ? "" : "hidden"}>${h(e ?? "")}</p>`;

  function aloneHtml() {
    return `<p class="ac-lead">This machine is on its own.</p>
      <p class="pj-note">A network joins the machines you use, each running XenoNet. The sessions on each can see and message each other, what waits on you shows in every console, and you can message any session from any of them. The machines reach each other straight: on your LAN, by direct paths, or through a relay you name.</p>
      <ol class="ac-list net-choices">
        <li class="ac-row"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">Start a network</span></div><div class="ac-what">This machine is the first one in it, and its founder.</div></div><div class="ac-acts"><button class="act primary" data-net-go="start">Start a network…</button></div></li>
        <li class="ac-row"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">Join one</span></div><div class="ac-what">Another machine is in a network: its Add a machine shows a line to run here.</div></div><div class="ac-acts"><button class="act" data-net-go="join">Join one…</button></div></li>
      </ol>`;
  }

  // How this machine stands in the network, in a word and a line.
  function linkHtml() {
    const w = linkWords(state.link);
    return `<div class="ac-gw" data-state="${h(w.state)}" data-net-standing data-net-link="${h(state.link?.state ?? "off")}">
        <div class="ac-gw-line"><span class="ac-gw-dot"></span><span class="ac-gw-word" data-net-link-word>${h(w.word)}</span></div>
        <div class="ac-gw-sub" data-net-link-sub>${h(w.sub)}</div>
      </div>`;
  }

  // Another machine's agent: what waits on you, or why it seems stuck, in
  // its machine's words when it shares them; what it said last on hover.
  function agentHtml(a) {
    const tag = a.waits ? (a.kind === "permission" ? "needs permission" : "waits on you") : a.stuck ? "may be stuck" : null;
    const what = a.waits ? (a.text || `The question shows in ${a.host}'s console.`) : a.stuck ? a.stuck.join(" · ") : [a.status, a.project, a.branch].filter(Boolean).join(" · ");
    const said = a.lastMessage ? ` title="${h(`Said last: ${a.lastMessage}`)}"` : "";
    return `<li class="net-agent${a.waits ? " waits" : ""}${a.stuck && !a.waits ? " stuck" : ""}" data-net-agent="${h(a.id)}"${said}><i class="sw ${h(a.app ?? "")}"></i><span class="net-agent-title">${h(a.title)}</span><span class="net-agent-what">${tag ? `<em>${h(tag)}</em> ` : ""}${h(what)}</span><button class="ac-link" data-net-message="${h(a.id)}" title="Message this session as you">Message</button></li>`;
  }

  // Who may start and wake sessions: one button per value, the one set
  // pressed. `on` names another machine, whose buttons send it the change
  // (data-net-set-on); this machine's set it here (data-net-set). Both are
  // yours.
  function settingHtml(setting, value, on = null) {
    const values = setting === "start" ? STARTS : WAKES;
    const words = setting === "start" ? START_WORDS : WAKE_WORDS;
    const attr = (v) => (on ? `data-net-set-on="${h(`${on}|${setting}:${v}`)}"` : `data-net-set="${h(`${setting}:${v}`)}"`);
    const label = setting === "start" ? "Start sessions" : "Wake for a message";
    return `<div class="net-seg" data-net-setting="${h(setting)}"><span class="net-seg-k">${label}</span>${values
      .map((v) => `<button class="act${v === value ? " on" : ""}" ${attr(v)} aria-pressed="${v === value}" data-yours>${h(words[v])}</button>`)
      .join("")}</div>`;
  }

  // Another machine's start and wake, changed from this window host: a
  // change to the network waits there for the founder or a second window
  // host, which approves it here.
  function reachHtml(x) {
    const r = reachOf(x);
    if (!r) return `<div class="net-reach" data-net-reach="${h(x.id)}"><div class="ac-via">Its XenoNet is too old to start sessions or to take these settings from here.</div></div>`;
    const window = !!state.self?.window;
    const pending = r.pending
      .map((w) => `<div class="ac-via net-warn" data-net-widen="${h(w.id)}">${h(widenWords(w, x.id))}. It needs the founder or a second window host.${window && w.by !== state.self?.id ? ` <button class="act" data-net-widen-approve="${h(`${x.id}|${w.id}`)}" data-yours>Approve</button>` : ""}</div>`)
      .join("");
    const controls = window
      ? `${settingHtml("start", r.start, x.id)}${settingHtml("wake", r.wake, x.id)}`
      : `<div class="ac-via">Starts: ${h(START_WORDS[r.start])} · wakes: ${h(WAKE_WORDS[r.wake])}</div>`;
    const delegation = r.start === "delegate" ? `<div class="ac-via" data-net-delegation="${h(x.id)}">${h(delegationWords(r.delegated))}</div>` : "";
    return `<div class="net-reach" data-net-reach="${h(x.id)}">${controls}${delegation}${pending}</div>`;
  }

  function hostHtml(x, confirm) {
    const me = x.id === state.self?.id;
    const tags = hostTags(x, state.self)
      .map((t) => `<span class="ac-badge${t === "you're here" ? " on" : ""}">${h(t)}</span>`)
      .join("");
    // The founder's key is the network's root: nobody removes or replaces
    // it (docs/design/network.md, section 3).
    const acts = me || x.founder
      ? ""
      : confirm === x.id
        ? `<span class="net-ask">Remove ${h(x.id)}?</span><button class="act" data-net-keep>Keep it</button><button class="act primary ac-danger" data-net-remove-yes="${h(x.id)}" data-yours>Remove</button>`
        : `<button class="act" data-net-replace="${h(x.id)}" data-yours title="It was set up again: a new line for it, under the same id">Replace…</button><button class="act" data-net-remove="${h(x.id)}" title="Take it out of the network">Remove</button>`;
    // A session started there: this machine, or another online whose
    // XenoNet takes starts.
    const starts = me || (x.online && hostCan(x, "start")) ? `<button class="act" data-net-start-session="${h(x.id)}" title="A new Claude Code, Codex, Grok Build or Hermes session there, with a first prompt">Start a session…</button>` : "";
    const theirs = agents.filter((a) => a.host === x.id);
    const mismatch = state.rosterMismatch.includes(x.id) ? `<div class="ac-via net-warn">sees another list of the network's machines</div>` : "";
    return `<li class="ac-row net-host${x.online ? "" : " offline"}" data-key="${h(x.id)}" data-net-host="${h(x.id)}">
        <span class="ac-dot"></span>
        <div class="ac-main"><div class="ac-name"><span class="ac-title">${h(x.id)}</span>${tags}</div><div class="ac-what" data-net-host-line>${h(hostLine(x))}</div>${mismatch}</div>
        <div class="ac-acts">${starts}${acts}</div>
        ${me ? "" : reachHtml(x)}
        ${theirs.length ? `<ol class="net-agents">${theirs.map(agentHtml).join("")}</ol>` : ""}
      </li>`;
  }

  function memberHtml() {
    const hosts = [...state.hosts].sort((a, b) => (b.id === state.self?.id) - (a.id === state.self?.id) || (b.online === true) - (a.online === true) || (a.id < b.id ? -1 : 1));
    const mismatch = state.rosterMismatch.length
      ? `<p class="net-warn" data-net-mismatch>${h(state.rosterMismatch.join(", "))} ${state.rosterMismatch.length === 1 ? "sees" : "see"} another list of the network's machines than this one. One of you missed a change to it; it comes when the two reach each other. The doctor says more.</p>`
      : "";
    const invites = state.invites.open.length
      ? `<div class="panel-title sub"><span>Waiting to join</span><span class="tick">${state.invites.open.length}</span></div><ul class="net-tickets" data-net-invites>${state.invites.open.map((t) => `<li>${h(inviteWords(t))}</li>`).join("")}</ul>`
      : "";
    // What waits for the founder or a second window host: a window host
    // signs it too here, unless it has.
    const mine = (p) => p.signers?.includes(state.self?.id);
    const approvals = state.proposals.length
      ? `<div class="panel-title sub"><span>Waiting for approval</span><span class="tick">${state.proposals.length}</span></div><ul class="net-tickets" data-net-proposals>${state.proposals
          .map((p) => `<li data-net-proposal="${h(p.id)}">${h(proposalWords(p))}. It needs the founder or a second window host.${state.self?.window && !mine(p) ? ` <button class="act" data-net-approve="${h(p.id)}" data-yours>Approve</button>` : ""}</li>`)
          .join("")}</ul>`
      : "";
    // A member that signed two entries at one place: its feed is frozen
    // here until you unfreeze it, from the window alone.
    const frozen = state.frozen.length
      ? `<div class="panel-title sub"><span>Frozen</span><span class="tick">${state.frozen.length}</span></div><ul class="net-tickets" data-net-frozen>${state.frozen
          .map((f) => `<li data-net-frozen-feed="${h(f.author)}">${h(frozenWords(f))}. Unfreeze it once you trust that machine again, or remove it.${state.self ? ` <button class="act" data-net-unfreeze="${h(f.author)}" data-yours>Unfreeze</button>` : ""}</li>`)
          .join("")}</ul>`
      : "";
    return `${linkHtml()}${mismatch}${frozen}
      <div class="panel-title sub"><span>Machines</span><span class="tick">${hosts.length}</span></div>
      <ol class="ac-list" data-net-hosts>${hosts.map((x) => hostHtml(x, view.confirm)).join("")}</ol>
      ${approvals}${invites}
      <div class="panel-title sub"><span>This machine</span></div>
      <label class="net-check"><input type="checkbox" data-net-titles data-yours ${state.titlesOnly ? "checked" : ""}> Share with the network: titles only</label>
      <p class="pj-note">On, the other consoles see this machine's sessions by their titles alone: not the text of a question waiting, the last message, or why a session seems stuck.</p>
      <label class="net-check"><input type="checkbox" data-net-load data-yours ${state.shareLoad !== false ? "checked" : ""}> Share load with the network</label>
      <p class="pj-note">On, the window hosts' consoles see this machine's CPU, GPU and memory, and what its parts are, in their Machines panel while they watch it.</p>
      ${settingHtml("start", state.start)}
      ${settingHtml("wake", state.wake)}
      ${state.start === "delegate" ? `<p class="pj-note" data-net-delegation="${h(state.self?.id ?? "")}">${h(delegationWords(state.delegated ? { by: state.delegated.by, at: state.delegated.at } : null))}</p>` : ""}
      <p class="pj-note">Who may start a session on this machine: nobody, you from a window host's console, the network, which adds the sessions and actors on window hosts, or delegated: the network, and what its sessions and actors start here runs on your standing delegation, within the limits above. Who may wake a session here to take a message at once: the sessions you allowed with <code>xenonet live allow</code>, or anyone in the network, within the wake budget. A window host can narrow these from its console; widening to the network from another machine takes the founder or a second window host.</p>
      ${selfWidens()}
      ${networksHtml()}
      ${devicesHtml(personOf(state), view.confirmDevice)}
      ${servicesHtml(services)}
      ${contextHtml(context, state.networks)}
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-go="start" data-net-another>Start another network…</button><button class="act" data-net-go="join">Join another…</button><span class="grow"></span><button class="act primary" data-net-go="add">Add a machine…</button></div>`;
  }

  // The networks this machine is in, the one shown picked; what the shown
  // one sees of this machine (its projects' sessions); its relays, and
  // this machine's own; how the home relay stands.
  function networksHtml() {
    const nets = state.networks;
    if (!nets.length) return "";
    const here = state.relaysHere ?? {};
    const list = nets
      .map((n) => {
        const shown = n.id === state.network?.id;
        const sees = n.sharing === "all" ? "all of this machine's projects" : n.sharing.length ? `${n.sharing.length} project${n.sharing.length === 1 ? "" : "s"}` : "none of this machine's projects";
        const pick = shown ? `<span class="tick">shown</span>` : `<button class="act" data-net-pick="${h(n.id)}">Show</button>`;
        return `<li data-net-network="${h(n.id)}"><b>${h(n.name)}</b> · this machine is ${h(n.host ?? "?")} · ${(n.hosts ?? []).length} machine${(n.hosts ?? []).length === 1 ? "" : "s"} · sees ${h(sees)} ${pick}</li>`;
      })
      .join("");
    const sharing = state.sharing === "all" ? "all" : Array.isArray(state.sharing) ? state.sharing : [];
    const projects = shareable
      .map((p) => `<label class="net-check"><input type="checkbox" data-net-share="${h(p.id)}" data-yours ${sharing === "all" || sharing.includes(p.id) ? "checked" : ""} ${sharing === "all" ? "disabled" : ""}> ${h(tildePath(p.path))}</label>`)
      .join("");
    const relays = (state.relays ?? []).map((u) => `<li data-net-relay="${h(u)}"><code>${h(u)}</code> <button class="act" data-net-relay-remove="${h(u)}" data-yours>Remove</button></li>`).join("");
    const own = (here.own ?? []).map((u) => `<li data-net-own-relay="${h(u)}"><code>${h(u)}</code> <button class="act" data-net-own-remove="${h(u)}" data-yours>Remove</button></li>`).join("");
    const direct = (here.directOnly ?? []).map((id) => nets.find((n) => n.id === id)?.name ?? id);
    const home = here.url ? `This machine's home relay is ${here.url}.` : "This machine has no home relay: the others reach it on the LAN, by a direct path, or when it dials them.";
    return `<div class="panel-title sub"><span>Networks</span><span class="tick">${nets.length}</span></div>
      <ul class="net-tickets" data-net-networks>${list}</ul>
      <div class="panel-title sub"><span>What ${h(state.network?.name ?? "this network")} sees</span></div>
      <label class="net-check"><input type="checkbox" data-net-share-all data-yours ${sharing === "all" ? "checked" : ""}> All of this machine's projects</label>
      ${projects || (sharing === "all" ? "" : `<p class="pj-note">No project here has a confirmed checkout yet.</p>`)}
      <p class="pj-note" data-net-sharing>Its members see the sessions in these projects' checkouts on this machine, and nothing of the others. The first network this machine is in sees all of them; one after it starts with none.</p>
      <div class="panel-title sub"><span>Relays</span></div>
      <ul class="net-tickets" data-net-relays>${relays || `<li class="dim">None: ${h(state.network?.name ?? "the network")}'s machines reach each other on the LAN and by direct paths.</li>`}</ul>
      <div class="ac-form"><label class="ac-field"><span class="ac-k">Add a relay</span><input class="ac-input" data-net-relay-url placeholder="https://relay.example.net, or n0" spellcheck="false" autocomplete="off"></label></div>
      <p class="net-relay-actions"><button class="act" data-net-relay-add data-yours>Add to ${h(state.network?.name ?? "the network")}</button><button class="act" data-net-own-add data-yours>Add as this machine's own</button></p>
      ${own ? `<p class="pj-note">This machine's own relays, for its own dials, in no network's list:</p><ul class="net-tickets" data-net-own-relays>${own}</ul>` : ""}
      <p class="pj-note" data-net-home>A relay is your own iroh-relay, or n0's public ones (rate-limited, run by n0). Adding one to a network asks you, and takes the founder or a second window host. ${h(home)}${direct.length > 1 || (direct.length && nets.length > 1) ? ` ${h(direct.join(", "))} list${direct.length === 1 ? "s" : ""} no relay this machine has.` : ""}</p>`;
  }

  function startHtml() {
    const lead = view.from === "add" ? `<p class="pj-note">A machine joins a network, so adding one starts with a network here.</p>` : "";
    return `<p class="ac-lead">Start a network here</p>${lead}
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Its name</span><input class="ac-input" data-net-name value="home" maxlength="64" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">This machine's id</span><input class="ac-input" data-net-self placeholder="studio" maxlength="32" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">Relay</span><select class="ac-input" data-net-relay-kind><option value="">None: the LAN and direct paths only</option><option value="own">Your own iroh-relay</option><option value="n0">n0's public relays (rate-limited, run by n0)</option></select></label>
        <label class="ac-field"><span class="ac-k">Your relay's URL</span><input class="ac-input" data-net-relay-own placeholder="https://relay.example.net" spellcheck="false" autocomplete="off"></label>
      </div>
      <p class="pj-note">The id is what every console calls this machine: lowercase letters, digits and dashes. Pick a name, since XenoNet never uses the machine's own. This machine is the network's founder, whose key is its root. In a terminal: <code>xenonet net create --name &lt;name&gt; --host &lt;id&gt; [--relay &lt;url|n0&gt;]</code>. With no relay, the network's machines reach each other on the LAN and by direct paths.</p>
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-create data-yours>Start the network</button></div>`;
  }

  function joinHtml() {
    return `<p class="ac-lead">Join a network</p>
      <p class="pj-note">On a machine that's in the network, choose Add a machine. It shows one line to run here, in a terminal:</p>
      <pre class="ac-message">xenonet net join &lt;invite&gt; &lt;code&gt;</pre>
      <p class="pj-note">Or paste that line, or its invite and code, here. The invite works once, for ten minutes.</p>
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Invite</span><input class="ac-input" data-net-join-invite placeholder="xni2_…" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">Code</span><input class="ac-input" type="password" data-net-join-code data-yours placeholder="KX7Q-49MZ-T2" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">Its address</span><input class="ac-input" data-net-join-at placeholder="192.168.1.20:47861, when the invite names no relay" spellcheck="false" autocomplete="off"></label>
      </div>
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-join data-yours>Join</button></div>`;
  }

  function addHtml() {
    return `<p class="ac-lead">Add a machine</p>
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Its id</span><input class="ac-input" data-net-add-host value="${h(view.host ?? "")}" placeholder="laptop" maxlength="32" spellcheck="false" autocomplete="off"></label>
      </div>
      <label class="net-check"><input type="checkbox" data-net-add-window ${view.window ? "checked" : ""}> I'll use a XenoNet window on it</label>
      <p class="pj-note">The id is what every console calls it: lowercase letters, digits and dashes. Pick a name, since XenoNet never uses the machine's own. A machine with a window can act for you from its console: message sessions as you, and add or remove machines. Leave it unticked for a server you only reach in a terminal. XenoNet then shows one line to run on the new machine, which works once, for ten minutes.</p>
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-make data-yours>Make the join line</button></div>`;
  }

  const joined = () => !!view.host && state.hosts.some((x) => x.id === view.host);

  function waitHtml() {
    const done = joined();
    return `<li class="ac-wait" data-ac-wait="${done ? "done" : "waiting"}" data-net-wait><span class="ac-step-text">${h(done ? `${view.host} joined the network` : `Waiting for ${view.host} to join…`)}</span></li>`;
  }

  function lineHtml() {
    return `<p class="ac-lead">Run this on ${h(view.host)}</p>
      <ol class="ac-steps">
        <li><div class="ac-step-head"><span class="ac-step-text">In a terminal on ${h(view.host)}, where XenoNet is installed, run this line.</span><button class="act primary" data-secret-copy="${SECRET}">Copy line</button></div>${secretHtml(SECRET, { attrs: "data-net-join-line", block: true })}</li>
        ${waitHtml()}
      </ol>
      <p class="pj-note" data-net-line-note>It works once, until ${h(clock(view.expires))}: ten minutes. XenoNet keeps only a hash of its code, so it can't show it again.${view.replaces ? ` ${h(view.host)}'s old keys stop working once it joins with this one.` : ""}${view.window ? ` From its window, ${h(view.host)} can act for you.` : ""}</p>
      <div class="m-foot"><span class="grow"></span><button class="act" data-net-done>Done</button></div>`;
  }

  // ---------- Start a session ----------

  // The machines a session can start on: this one, and the others online
  // whose XenoNet takes starts.
  function startHosts() {
    const me = state.self?.id ?? null;
    const others = state.hosts.filter((x) => x.id !== me && x.online && hostCan(x, "start")).map((x) => x.id);
    return [{ id: me, label: me ? `${me} (here)` : "This machine" }, ...others.map((id) => ({ id, label: id }))];
  }

  // A machine's folders to pick from: this one's projects and folders on
  // the map, another's sessions' folders as its presence reports them;
  // and home.
  function foldersOf(host) {
    const me = !host || host === state.self?.id;
    const list = me ? projects().map((p) => ({ value: p.id, label: p.path ?? p.name ?? p.id })) : [...new Set(agents.filter((a) => a.host === host && a.cwd).map((a) => a.cwd))].map((c) => ({ value: c, label: c }));
    return [{ value: "~", label: "home" }, ...list.slice(0, 200)];
  }

  const choice = (attr, value, label, on) => `<button class="act${on ? " on" : ""}" ${attr}="${h(value)}" aria-pressed="${on}">${h(label)}</button>`;

  function sessionHtml() {
    const hosts = startHosts();
    const host = hosts.some((x) => x.id === view.host) ? view.host : hosts[0].id;
    view.host = host;
    const app = view.app ?? "claude";
    const mode = NO_MODE.has(app) ? "" : (view.mode ?? "");
    const folders = foldersOf(host);
    return `<p class="ac-lead">Start a session</p>
      <div class="net-seg" data-net-start-hosts><span class="net-seg-k">Machine</span>${hosts.map((x) => choice("data-net-start-host", x.id ?? "", x.label, x.id === host)).join("")}</div>
      <div class="net-seg" data-net-start-apps><span class="net-seg-k">App</span>${START_APPS.map(([id, label]) => choice("data-net-start-app", id, label, id === app)).join("")}</div>
      <div class="net-seg" data-net-start-modes><span class="net-seg-k">Permissions</span>${START_MODES.map(([id, label]) => choice("data-net-start-mode", id, label, id === mode)).join("")}</div>
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Folder</span><input class="ac-input" data-net-start-folder list="net-start-folders" value="${h(view.folder ?? "~")}" spellcheck="false" autocomplete="off"></label>
        <datalist id="net-start-folders" data-net-start-folders>${folders.map((f) => `<option value="${h(f.value)}">${h(f.label)}</option>`).join("")}</datalist>
        <label class="ac-field"><span class="ac-k">Prompt</span><textarea class="ac-input net-start-prompt" data-net-start-prompt rows="5" placeholder="What the new session should do" spellcheck="true">${h(view.prompt ?? "")}</textarea></label>
      </div>
      <p class="pj-note">Claude Code and Codex run headless through XenoNet, Grok Build and Hermes over ACP, and each shows on the map like any other session. Its first prompt says you started it from this console. Permission prompts it can't settle by its settings come to Needs you, here and on every window host. Codex runs with its own config's sandbox and approvals. In a terminal: <code>xenonet start --on &lt;machine&gt; "&lt;prompt&gt;"</code></p>
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-start-send data-yours disabled title="Starts the session">Start</button></div>`;
  }

  // Send waits a moment after the box shows, so a click meant for what
  // was there before can't land on it.
  let armTimer = null;
  function arm() {
    clearTimeout(armTimer);
    view.armedAt = Date.now() + ARM_MS;
    armTimer = setTimeout(() => {
      const b = body()?.querySelector("[data-net-start-send]");
      if (b && !busy) b.disabled = false;
    }, ARM_MS);
  }
  const armed = () => view.name === "session" && view.armedAt != null && Date.now() >= view.armedAt;

  // A pick in the box, changed in place so what's typed stays.
  function pick(attr, value) {
    const key = { "data-net-start-host": "host", "data-net-start-app": "app", "data-net-start-mode": "mode" }[attr];
    view[key] = value;
    for (const b of body()?.querySelectorAll(`[${attr}]`) ?? []) {
      const on = b.getAttribute(attr) === value;
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", String(on));
    }
    if (key === "host") body().querySelector("[data-net-start-folders]").innerHTML = foldersOf(value).map((f) => `<option value="${h(f.value)}">${h(f.label)}</option>`).join("");
    // Codex takes its own config's, and Hermes its own settings: no mode.
    if (key === "app") {
      for (const b of body()?.querySelectorAll("[data-net-start-mode]") ?? []) b.disabled = NO_MODE.has(value) && b.dataset.netStartMode !== "";
      if (NO_MODE.has(value)) pick("data-net-start-mode", "");
    }
    sfx("toggle");
  }

  async function sendStart() {
    if (!armed()) return { ok: false, error: "not armed yet" };
    const b = body();
    const host = view.host || null;
    const folder = b.querySelector("[data-net-start-folder]").value.trim() || "~";
    const prompt = b.querySelector("[data-net-start-prompt]").value;
    const app = view.app ?? "claude";
    const mode = NO_MODE.has(app) ? "" : (view.mode ?? "");
    const remote = host && host !== state.self?.id;
    Object.assign(view, { folder, prompt });
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/start", { ...(remote ? { machine: host } : {}), folder, app, ...(mode ? { mode } : {}), prompt });
      if (!r.ok) return failed(r, `xenonet start${remote ? ` --on ${host}` : ""} "<prompt>"`);
      sfx("success");
      toast(r.data.state === "sent" ? `Sent to ${r.data.host}: whether it started comes to the feed.` : `Started ${r.data.session ?? "a Codex session"} in ${r.data.folder}.`);
      lastStart = { ...r.data, at: Date.now() };
      go({ name: "main" });
      return { ok: true, ...r.data };
    });
  }

  const VIEWS = { main: () => (inNetwork(state) ? memberHtml() : aloneHtml()), start: startHtml, join: joinHtml, add: addHtml, line: lineHtml, session: sessionHtml, publish: () => publishHtml(view.draft, view.error) };

  // Drawn again only when something it shows changed, so a read every few
  // seconds never swaps a button out from under the pointer. A form keeps
  // what's typed in it, and the join line only has its wait ticked.
  function render() {
    if (!isOpen()) return;
    const sub = panel.querySelector("[data-net-sub]");
    if (sub && sub.textContent !== subText()) sub.textContent = subText();
    const b = body();
    if (!read) {
      b.innerHTML = `<p class="pj-note">Reading…</p>`;
      drawn = "";
      return;
    }
    if (view.name === "line" && b.querySelector("[data-net-join-line]")) {
      const wait = b.querySelector("[data-net-wait]");
      if (wait && wait.dataset.acWait !== (joined() ? "done" : "waiting")) wait.outerHTML = waitHtml();
      return;
    }
    if ((view.name === "add" || view.name === "start" || view.name === "join" || view.name === "session" || view.name === "publish") && drawn === view.name) return;
    const html = VIEWS[view.name]?.() ?? VIEWS.main();
    if (view.name === "main" && html === drawn) return;
    b.innerHTML = html;
    drawn = view.name === "main" ? html : view.name;
    if (view.name === "session") arm();
    b.querySelector("[data-net-add-host], [data-net-self], [data-net-join-invite], [data-net-svc-name]")?.focus();
  }

  function go(next) {
    view = next;
    drawn = "";
    render();
  }

  // Says why it didn't go, in the open form.
  function failed(r, terminal) {
    sfx("error");
    const missing = r.status === 404 && !r.code;
    const error = missing ? `This XenoNet can't do that from here yet.${terminal ? ` In a terminal: ${terminal}` : ""}` : (r.error ?? "That didn't work.");
    view = { ...view, error };
    const p = body()?.querySelector("[data-net-error]");
    if (p && view.name !== "main") {
      p.textContent = error;
      p.hidden = false;
    } else go(view);
    return { ok: false, status: r.status, error };
  }

  async function busyWith(run) {
    if (busy) return { ok: false, error: "busy" };
    busy = true;
    for (const b of body()?.querySelectorAll("[data-yours]") ?? []) b.disabled = true;
    try {
      return await run();
    } finally {
      busy = false;
      for (const b of body()?.querySelectorAll("[data-yours]") ?? []) b.disabled = false;
    }
  }

  // Shows an invite's line once, masked, for `host`.
  function showLine(r, host, window, replaces = null) {
    const join = joinOf(r.data);
    if (!join) return failed({ status: 502, error: "XenoNet made an invite but sent no line to show." });
    forgetSecrets();
    keepSecret(SECRET, join.secret, join.line);
    joinCode = join.secret;
    sfx("success");
    go({ name: "line", host, window, expires: join.expires, replaces });
    void refresh().catch(() => {});
    return { ok: true, host, expires: join.expires };
  }

  async function make() {
    const host = body().querySelector("[data-net-add-host]").value.trim();
    const window = body().querySelector("[data-net-add-window]").checked;
    view = { ...view, host, window };
    const problem = hostIdProblem(host, state);
    if (problem) return failed({ status: 400, error: problem });
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/invite", { host, window });
      return r.ok ? showLine(r, host, window) : failed(r, `xenonet net invite ${host}${window ? " --window" : ""}`);
    });
  }

  async function replace(host) {
    const x = state.hosts.find((y) => y.id === host);
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/replace", { host, window: !!x?.window });
      return r.ok ? showLine(r, host, !!x?.window, host) : failed(r);
    });
  }

  async function remove(host) {
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/remove", { host });
      if (!r.ok) return failed(r, `xenonet net remove ${host}`);
      sfx("success");
      toast(`${host} is out of the network. What it sends is refused from now on.`);
      view = { name: "main" };
      await refresh();
      return { ok: true };
    });
  }

  // Revokes another of your devices, as for a lost machine: it asks you.
  async function revokeDevice(device) {
    const d = personOf(state)?.devices.find((x) => x.id === device);
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/devices/revoke", { device });
      if (!r.ok) return failed(r);
      sfx("success");
      toast(`${d?.label ?? device}'s device is revoked: nothing it signs counts on any machine now.`);
      view = { name: "main" };
      await refresh();
      return { ok: true, revoked: device };
    });
  }

  // Publishes a service from the form: it asks you.
  async function publish() {
    const b = body();
    const draft = { name: b.querySelector("[data-net-svc-name]").value, target: b.querySelector("[data-net-svc-target]").value, paths: b.querySelector("[data-net-svc-paths]").value, hosts: b.querySelector("[data-net-svc-hosts]").value };
    view = { ...view, draft };
    const sent = publishBody(draft);
    if (!sent.name || !sent.target || !sent.paths.length) return failed({ status: 400, error: "Give its name, its address and the paths other machines may reach." });
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/svc/publish", sent);
      if (!r.ok) return failed(r);
      sfx("success");
      toast(`${sent.name} is published: other machines reach it through their own XenoNet.`);
      view = { name: "main" };
      await refresh();
      return { ok: true, name: sent.name };
    });
  }

  // Takes a service off the network: it asks nothing.
  // Xenolith's grant: approve its ask (which asks you), turn it down, or
  // take back the grant it holds.
  async function contextAct(what, sent, done) {
    return busyWith(async () => {
      const r = await netCall("POST", `/api/net/forwards/${what}`, sent);
      if (!r.ok) return failed(r);
      sfx(what === "approve" ? "success" : "toggle");
      toast(done);
      await refresh();
      return { ok: true };
    });
  }

  async function unpublish(name) {
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/svc/unpublish", { name });
      if (!r.ok) return failed(r, `xenonet svc unpublish ${name}`);
      sfx("success");
      toast(`${name} is off the network.`);
      await refresh();
      return { ok: true };
    });
  }

  async function create() {
    const name = body().querySelector("[data-net-name]").value.trim();
    const host = body().querySelector("[data-net-self]").value.trim();
    const kind = body().querySelector("[data-net-relay-kind]")?.value ?? "";
    const relay = kind === "n0" ? "n0" : kind === "own" ? (body().querySelector("[data-net-relay-own]")?.value.trim() ?? "") : "";
    if (!name) return failed({ status: 400, error: "Give the network a name." });
    const problem = hostIdProblem(host, null);
    if (problem) return failed({ status: 400, error: problem });
    if (kind === "own" && !/^https?:\/\/\S+$/.test(relay)) return failed({ status: 400, error: "Give your relay's URL, as https://relay.example.net." });
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/create", { name, host, ...(relay ? { relay } : {}) });
      if (!r.ok) return failed(r, `xenonet net create --name ${name} --host ${host}${relay ? ` --relay ${relay}` : ""}`);
      sfx("success");
      chosen = networksOf(r.data).find((x) => x.name === name)?.id ?? chosen;
      if (r.data?.relayRefused) toast(`The network ${name} is started, without its relay: ${r.data.relayRefused}`, true);
      await refresh();
      toast(`The network ${name} is started.`);
      go({ name: "main" });
      return { ok: true };
    });
  }

  async function join() {
    const typed = joinFields(body().querySelector("[data-net-join-invite]").value, body().querySelector("[data-net-join-code]").value);
    body().querySelector("[data-net-join-code]").value = "";
    const at = typed.at ?? (body().querySelector("[data-net-join-at]")?.value.trim() || null);
    if (!typed.invite || !typed.code) return failed({ status: 400, error: "Give the invite and its code, as Add a machine showed them." });
    const sent = { invite: typed.invite, code: typed.code, lan: typed.lan, ...(at ? { at } : {}) };
    return busyWith(async () => {
      let r = await calls.call("POST", "/api/net/join", sent);
      // The machine that made the invite signs this one in, which may wait
      // for you there, or for a second approval: say why, and ask again
      // until the invite lapses.
      const until = Date.now() + INVITE_MS;
      let said = false;
      while (r.ok && r.data?.waiting && Date.now() < until) {
        if (!said) toast(r.data.message ?? "The join waits for the machine that made the invite.");
        said = true;
        await new Promise((done) => setTimeout(done, 3000));
        r = await calls.call("POST", "/api/net/join", sent);
      }
      if (r.ok && r.data?.waiting) return failed({ status: 409, error: r.data.message ?? "The invite lapsed while the join waited." });
      if (!r.ok) return failed(r, "xenonet net join <invite> <code>");
      sfx("success");
      // The network joined is the last the state lists: the panel shows it.
      chosen = networksOf(r.data).at(-1)?.id ?? chosen;
      await refresh();
      toast(inNetwork(state) ? `This machine is in ${state.network.name} now.` : "Joined.");
      go({ name: "main" });
      return { ok: true };
    });
  }

  async function unfreeze(author) {
    const f = state.frozen.find((x) => x.author === author);
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/feeds/unfreeze", { author });
      if (!r.ok) return failed(r);
      sfx("success");
      toast(`${f?.host ?? author}'s feed is taken again.`);
      await refresh();
      return { ok: true, unfrozen: author };
    });
  }

  async function approve(id) {
    const p = state.proposals.find((x) => x.id === id);
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/approve", { id });
      if (!r.ok) return failed(r, p ? `xenonet net approve ${p.host} --op ${p.op}` : null);
      sfx("success");
      toast(p ? `Approved: ${proposalWords(p)}.` : "Approved.");
      await refresh();
      return { ok: true };
    });
  }

  // Who may start or wake sessions ("start:network"): this machine's, or
  // another's (`host`), which that machine checks.
  async function setReach(host, sv) {
    const [setting, value] = String(sv).split(":");
    const words = (setting === "start" ? START_WORDS : WAKE_WORDS)[value] ?? value;
    return busyWith(async () => {
      const r = await netCall("PUT", "/api/net/settings", { ...(host ? { host } : {}), [setting]: value });
      if (!r.ok) return failed(r, `xenonet net set${host ? ` --on ${host}` : ""} --${setting} ${value}`);
      sfx("toggle");
      const what = setting === "start" ? "start sessions" : "wake sessions";
      if (!host) toast(`Who may ${what} here: ${words.toLowerCase()}.`);
      else if ((value === "network" || value === "delegate") && !state.self?.founder) {
        toast(value === "delegate" ? `Sent to ${host}. Putting it on your standing delegation waits for the founder or another window host.` : `Sent to ${host}. Letting the network ${what} there waits for the founder or another window host.`);
      }
      else toast(`Sent to ${host}: who may ${what} there, ${words.toLowerCase()}.`);
      await refresh();
      return { ok: true };
    });
  }

  // Another window host asked to widen a machine's settings: a second
  // window host's approval, sent to that machine.
  async function approveWiden(host, id) {
    return busyWith(async () => {
      const r = await netCall("PUT", "/api/net/settings", { host, approve: id });
      if (!r.ok) return failed(r, `xenonet net set --on ${host} --approve ${id}`);
      sfx("success");
      toast(`Approved, and sent to ${host}.`);
      await refresh();
      return { ok: true };
    });
  }

  // Widenings of this machine's settings that wait for another window host.
  function selfWidens() {
    const mine = reachOf(state.hosts.find((x) => x.id === state.self?.id));
    return (mine?.pending ?? []).map((w) => `<p class="net-warn" data-net-widen="${h(w.id)}">${h(widenWords(w, "this machine"))}. Another window host approves it, or set it here.</p>`).join("");
  }

  // What the shown network sees of this machine: all its projects, or the
  // ones ticked.
  async function share() {
    const all = body().querySelector("[data-net-share-all]")?.checked;
    const ticked = [...body().querySelectorAll("[data-net-share]")].filter((b) => b.checked).map((b) => b.dataset.netShare);
    const sharing = all ? "all" : ticked;
    const r = await netCall("PUT", "/api/net/sharing", { sharing });
    if (!r.ok) {
      failed(r, `xenonet net share ${all ? "all" : ticked.join(",") || "none"} --network ${state.network?.id ?? ""}`);
      return refresh();
    }
    sfx("toggle");
    toast(all ? `${state.network?.name} sees all of this machine's projects.` : ticked.length ? `${state.network?.name} sees ${ticked.length} of this machine's projects.` : `${state.network?.name} sees none of this machine's projects.`);
    return refresh();
  }

  // A relay into the shown network's list, or this machine's own (`mine`);
  // or out of it.
  async function relay(url, add, mine) {
    if (!url) return failed({ status: 400, error: "Give a relay's URL, or n0 for n0's public relays." });
    return busyWith(async () => {
      const r = await netCall("POST", "/api/net/relays", { url, add, mine });
      if (!r.ok) return failed(r, `xenonet net relay ${add ? "add" : "remove"} ${url}${mine ? " --mine" : ` --network ${state.network?.id ?? ""}`}`);
      sfx("success");
      toast(r.data?.proposed ? "Asked: the founder or another window host approves the relay." : add ? (mine ? "This machine dials through that relay too now." : `${state.network?.name} lists that relay now.`) : "That relay is out.");
      await refresh();
      return { ok: true };
    });
  }

  // The panel shows another of this machine's networks.
  async function pickNetwork(id) {
    if (!state.networks.some((x) => x.id === id)) throw new Error(`This machine isn't in ${id}.`);
    chosen = id;
    drawn = "";
    await refresh();
    return driverState();
  }

  async function titlesOnly(on) {
    const r = await netCall("PUT", "/api/net/settings", { titlesOnly: on });
    if (!r.ok) {
      failed(r);
      return refresh();
    }
    sfx("toggle");
    toast(on ? "The other consoles see this machine's sessions by title alone now." : "The other consoles see what waits on you here in full again.");
    return refresh();
  }

  async function shareLoad(on) {
    const r = await netCall("PUT", "/api/net/settings", { shareLoad: on });
    if (!r.ok) {
      failed(r);
      return refresh();
    }
    sfx("toggle");
    toast(on ? "The window hosts' consoles see this machine's load again." : "This machine's load stays here now.");
    return refresh();
  }

  async function onClick(e) {
    if (await secretClick(panel, e.target, (ok) => toast(ok ? "Line copied. Run it on the new machine." : "Couldn't copy it", !ok))) return;
    const t = e.target;
    const d = t.closest("button")?.dataset ?? {};
    if (d.netGo === "add") return addMachine();
    if (d.netGo) return go({ name: d.netGo });
    if (t.closest("[data-net-back]")) return go({ name: "main" });
    if (t.closest("[data-net-done]")) {
      forgetSecrets();
      joinCode = null;
      return go({ name: "main" });
    }
    if (t.closest("[data-net-make]")) return make();
    if (t.closest("[data-net-create]")) return create();
    if (d.netPick) return pickNetwork(d.netPick);
    if (t.closest("[data-net-relay-add]")) return relay(body().querySelector("[data-net-relay-url]")?.value.trim(), true, false);
    if (t.closest("[data-net-own-add]")) return relay(body().querySelector("[data-net-relay-url]")?.value.trim(), true, true);
    if (d.netRelayRemove) return relay(d.netRelayRemove, false, false);
    if (d.netOwnRemove) return relay(d.netOwnRemove, false, true);
    if (t.closest("[data-net-join]")) return join();
    if (d.netMessage) return message(d.netMessage);
    if (d.netRemove) return go({ name: "main", confirm: d.netRemove });
    if (t.closest("[data-net-keep]")) return go({ name: "main" });
    if (d.netRemoveYes) return remove(d.netRemoveYes);
    if (d.netRevoke) return go({ name: "main", confirmDevice: d.netRevoke });
    if (d.netRevokeYes) return revokeDevice(d.netRevokeYes);
    if (d.netUnpublish) return unpublish(d.netUnpublish);
    if (d.netCtxAllow) return contextAct("approve", approveBody(body(), d.netCtxAllow), "Allowed: Xenolith collects its grant and syncs over the network.");
    if (d.netCtxDeny) return contextAct("deny", { alpn: d.netCtxDeny.split("|")[0], key: d.netCtxDeny.split("|").slice(1).join("|") }, "Turned down.");
    if (d.netCtxSign) return contextAct("sign", signBody(d.netCtxSign), "Signed: Xenolith has taken it into the space's chain.");
    if (d.netCtxRevoke) return contextAct("revoke", { alpn: d.netCtxRevoke }, "Taken back: its sync over the network ends now.");
    if (t.closest("[data-net-publish]")) return publish();
    if (d.netReplace) return replace(d.netReplace);
    if (d.netApprove) return approve(d.netApprove);
    if (d.netUnfreeze) return unfreeze(d.netUnfreeze);
    if (d.netStartSession != null) return startSession({ host: d.netStartSession || null });
    for (const attr of ["data-net-start-host", "data-net-start-app", "data-net-start-mode"]) {
      const b = t.closest(`[${attr}]`);
      if (b) return pick(attr, b.getAttribute(attr));
    }
    if (t.closest("[data-net-start-send]")) return sendStart();
    if (d.netSet) return setReach(null, d.netSet);
    if (d.netSetOn) return setReach(...d.netSetOn.split("|"));
    if (d.netWidenApprove) return approveWiden(...d.netWidenApprove.split("|"));
  }

  // Opens the panel (`at`: start, join or add), or reads it again in place.
  async function open(at = null, opts = {}) {
    await styles();
    if (!isOpen()) {
      drawn = "";
      panel = modals.open(head(), {
        accent: "var(--cyan)",
        cls: "ac net",
        // The line on screen goes with the panel.
        onClose: () => {
          forgetSecrets();
          joinCode = null;
          clearTimeout(armTimer);
          unblur();
          view = { name: "main" };
          panel = null;
        },
      });
      // A shown line masks itself again as you switch to another app.
      const unblur = hideOnBlur(panel);
      panel.addEventListener("click", (e) => void onClick(e));
      panel.addEventListener("change", (e) => {
        if (e.target.matches("[data-net-titles]")) void titlesOnly(e.target.checked);
        if (e.target.matches("[data-net-load]")) void shareLoad(e.target.checked);
        if (e.target.matches("[data-net-share], [data-net-share-all]")) void share();
      });
      panel.addEventListener("keydown", (e) => {
        if (e.key !== "Enter" || !e.target.matches?.("input")) return;
        panel.querySelector("[data-net-make], [data-net-create], [data-net-join], [data-net-publish]")?.click();
      });
    }
    view = at ? { name: at, ...opts } : { name: "main" };
    // Asked for, a view is drawn afresh, a form emptied of what was in it.
    drawn = "";
    render();
    await refresh().catch(() => {});
    // Adding a machine starts with a network.
    if (view.name === "add" && !inNetwork(state)) go({ name: "start", from: "add" });
    return driverState();
  }

  // Add a machine: its form, with what's given filled in.
  const addMachine = ({ host = "", window = false } = {}) => open("add", { host, window: !!window });

  // The panel, scrolled to Your devices or Services (`what`), for the
  // driver: they're at its foot.
  async function reveal(what) {
    await open();
    panel?.querySelector(what === "services" ? "[data-net-services]" : what === "context" ? "[data-net-context]" : "[data-net-devices]")?.scrollIntoView({ block: "center" });
    return driverState();
  }

  // Publish a service: its form, with what's given filled in.
  const publishService = ({ name = "", target = "", paths = "", hosts = "" } = {}) => open("publish", { draft: { name, target, paths: [paths].flat().join(" "), hosts: [hosts].flat().join(" ") } });

  // Start a session: the box, on a machine (this one when none), with what's
  // given filled in.
  const startSession = ({ host = null, folder = null, app = null, mode = null, prompt = null } = {}) =>
    open("session", { host: host ?? state.self?.id ?? null, ...(folder ? { folder } : {}), ...(app ? { app } : {}), ...(mode ? { mode } : {}), ...(prompt ? { prompt } : {}) });

  // ---------- for the driver ----------

  const centre = (sel) => {
    const r = panel?.querySelector(sel)?.getBoundingClientRect();
    return r?.width ? { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) } : null;
  };

  // Where each button with `attr` is, by its value ("start:network",
  // "desk|wake:consent", "desk|<id>").
  const centres = (attr) => Object.fromEntries([...(panel?.querySelectorAll(`[${attr}]`) ?? [])].map((b) => [b.getAttribute(attr), centre(`[${attr}="${CSS.escape(b.getAttribute(attr))}"]`)]).filter(([, at]) => at));

  // Text from the panel with the code masked, shown or not.
  const masked = (text) => (joinCode ? text.split(joinCode).join("••••••••••••") : text);

  // Whether the join line shows masked: its code isn't in the text the
  // page shows. Never the code itself.
  function joinState() {
    if (view.name !== "line" || !isOpen()) return null;
    const box = panel.querySelector(`[data-secret="${SECRET}"]`);
    const text = box?.querySelector(".secret-text")?.innerText ?? "";
    return { host: view.host, window: !!view.window, expires: view.expires, shown: !!box?.classList.contains("shown"), masked: !joinCode || !text.includes(joinCode), joined: joined(), kept: secretValue(SECRET) != null };
  }

  function driverState() {
    const l = state.link;
    return {
      open: isOpen(),
      view: isOpen() ? view.name : null,
      inNetwork: inNetwork(state),
      simulated: !!state.simulated,
      network: state.network?.name ?? null,
      networkId: state.network?.id ?? null,
      self: state.self?.id ?? null,
      // Every network this machine is in, each with its host id here, what
      // it sees of this machine and its relays; the one shown is `current`.
      networks: state.networks.map((x) => ({ id: x.id, name: x.name, host: x.host, current: x.id === state.network?.id, hosts: (x.hosts ?? []).length, sharing: x.sharing, relays: x.relays, inContact: x.inContact ?? null })),
      relaysHere: state.relaysHere ?? null,
      shareable,
      link: l ? { state: l.state ?? "off", words: linkWords(l).word, since: l.since ?? null, reason: l.reason ?? null } : null,
      hosts: state.hosts.map((x) => ({
        id: x.id,
        online: !!x.online,
        seen: x.seen ?? null,
        you: !!x.you,
        here: x.id === state.self?.id,
        founder: !!x.founder,
        window: !!x.window,
        path: x.path ?? null,
        held: x.held ?? 0,
        agents: x.agents ?? 0,
        waitsOnYou: x.waitsOnYou ?? 0,
        reach: reachOf(x),
        can: Array.isArray(x.can) ? x.can : [],
        line: hostLine(x),
        row: isOpen() ? (panel.querySelector(`[data-net-host="${CSS.escape(x.id)}"] [data-net-host-line]`)?.textContent ?? null) : null,
        remote: agents.filter((a) => a.host === x.id).map((a) => ({ id: a.id, title: a.title, app: a.app, status: a.status, waits: a.waits, text: a.text, stuck: a.stuck, lastMessage: a.lastMessage, projectKey: a.projectKey })),
      })),
      invites: state.invites.open.map((t) => ({ host: t.host, window: !!t.window, expires: t.expires, wrong: t.wrong ?? 0 })),
      proposals: state.proposals.map((p) => ({ id: p.id, op: p.op, host: p.host, window: !!p.window, signers: p.signers ?? [], words: proposalWords(p) })),
      frozen: state.frozen.map((f) => ({ author: f.author, host: f.host ?? null, at: f.at ?? null, words: frozenWords(f) })),
      mismatch: state.rosterMismatch,
      titlesOnly: !!state.titlesOnly,
      shareLoad: state.shareLoad !== false,
      start: state.start ?? "you",
      wake: state.wake ?? "consent",
      waits: remoteWaits(state, agents).filter((w) => w.kind === "remote-need").map((w) => ({ agent: w.id, host: w.host, text: w.text, chime: w.chime })),
      stuck: remoteWaits(state, agents).filter((w) => w.kind === "remote-stuck").map((w) => ({ agent: w.id, host: w.host, text: w.text })),
      join: joinState(),
      // Your devices and how each asks, whether the root key is sealed
      // here; and what this machine publishes.
      person: personOf(state),
      services,
      // Xenolith's asks for a grant and the grants you gave; never a grant.
      context,
      // The Start a session box: what it's set to, whether Start is armed,
      // where each choice is; and the last start sent from it.
      session:
        isOpen() && view.name === "session"
          ? {
              host: view.host ?? null,
              hosts: startHosts().map((x) => x.id),
              app: view.app ?? "claude",
              mode: view.mode ?? "",
              folder: body()?.querySelector("[data-net-start-folder]")?.value ?? null,
              folders: foldersOf(view.host).map((f) => f.value),
              prompt: body()?.querySelector("[data-net-start-prompt]")?.value ?? null,
              armed: armed(),
              at: { send: centre("[data-net-start-send]"), hosts: centres("data-net-start-host"), apps: centres("data-net-start-app"), modes: centres("data-net-start-mode") },
            }
          : null,
      lastStart,
      answers: answers.slice(0, 10),
      error: isOpen() ? (view.error ?? null) : null,
      // What the panel says, with a join line shown masked all the same.
      text: isOpen() ? masked(body()?.innerText ?? "").replace(/\s+/g, " ").trim().slice(0, 4000) : null,
      at: isOpen() ? { make: centre("[data-net-make]"), create: centre("[data-net-create]"), join: centre("[data-net-join]"), show: centre("[data-secret-show]"), titles: centre("[data-net-titles]"), load: centre("[data-net-load]"), add: centre('[data-net-go="add"]'), approve: centre("[data-net-approve]"), unfreeze: centres("data-net-unfreeze"), set: centres("data-net-set"), setOn: centres("data-net-set-on"), widen: centres("data-net-widen-approve"), startSession: centres("data-net-start-session"), revoke: centres("data-net-revoke"), revokeYes: centre("[data-net-revoke-yes]"), publish: centre("[data-net-publish]"), unpublish: centres("data-net-unpublish"), pick: centres("data-net-pick"), shareAll: centre("[data-net-share-all]"), share: centres("data-net-share"), relayAdd: centre("[data-net-relay-add]"), ownAdd: centre("[data-net-own-add]"), relayRemove: centres("data-net-relay-remove"), ownRemove: centres("data-net-own-remove"), relayUrl: centre("[data-net-relay-url]"), relayKind: centre("[data-net-relay-kind]"), ctxAllow: centres("data-net-ctx-allow"), ctxDeny: centres("data-net-ctx-deny"), ctxRevoke: centres("data-net-ctx-revoke"), ctxSign: centres("data-net-ctx-sign"), ctxNet: centres("data-net-ctx-net") } : null,
    };
  }

  // Another machine's agent, from its card in the strip: the panel, at its
  // row.
  async function show(id) {
    await open();
    const row = panel?.querySelector(`[data-net-agent="${CSS.escape(id)}"]`) ?? panel?.querySelector(`[data-net-host="${CSS.escape(hostOfId(id) ?? "")}"]`);
    row?.scrollIntoView({ block: "center" });
    row?.classList.add("net-flash");
    return driverState();
  }

  return {
    open,
    addMachine,
    publishService,
    reveal,
    startSession,
    show,
    refresh,
    // The event stream said the network changed.
    heard: () => void refresh().catch(() => {}),
    // An answer to a start you asked for (the start event).
    heardStart: (s) => {
      answers.unshift({ state: s.state, host: s.host ?? null, session: s.session ?? null, reason: s.reason ?? null, exit: s.exit ?? null, words: s.words ?? null });
      answers.length = Math.min(answers.length, 10);
    },
    close: () => isOpen() && modals.close(),
    // One of another machine's agents, by its agt_.
    agent: (id) => agents.find((a) => a.id === id) ?? null,
    get state() {
      return state;
    },
    get view() {
      return viewOf();
    },
    get isOpen() {
      return isOpen();
    },
    driverState,
    pickNetwork,
    // GET /api/net/state as this window reads it, with its key: what the
    // service serves, before the page reads it leniently.
    served: () => getJson("/api/net/state"),
  };
}
