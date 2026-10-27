// The network across machines in the console (docs/design/network.md,
// sections 4, 7 and 8): Settings → Network, which the palette calls
// Network settings, and Add a machine.
//
//   - On its own, a machine offers Start a network (this machine is the
//     founder and the gateway) and Join one (the line Add a machine shows
//     elsewhere, run here in a terminal, or its address and ticket here).
//   - In a network: its name and this machine, the gateway (its address,
//     kind and link, since when, why it's down, clock skew), each machine
//     (online or since when, its agents and what waits on you, founder,
//     gateway, window, where you're at) with Remove and Replace, other
//     machines' agents under them with Message (with the text of what
//     waits on you, or why one seems stuck, as their machine shares it,
//     escaped as transcripts are), the tickets waiting, the outbox, a
//     roster mismatch, and "Share with the network: titles only".
//   - Add a machine asks the new machine's id and whether you'll use a
//     XenoNet window on it, then shows the join line once, masked as a
//     token is (secret.js): out of the page's text until Show, then for ten
//     seconds, and masked again when the window loses focus or the page is
//     held still for a screenshot. XenoNet keeps only the ticket's hash.
//
// The state comes from GET /api/net/state, read again every few seconds in
// a network, rarely out of one, and whenever the event stream says it
// changed (net). Other machines' agents come from the engine's list (GET
// /api/engine/agents), with @<host>: ids. Starting, joining, tickets,
// removing, replacing and the setting go through the window's own calls,
// with its key: they're yours, and marked so (data-yours). A machine on its
// own never sees any of this outside this panel and Add a machine.
import { escapeHtml as h } from "./format.js";
import { hostOfId } from "./hosts.js";
import { ARM_MS } from "./actor-requests.js";
import { keepSecret, forgetSecrets, secretHtml, secretClick, hideOnBlur, secretValue } from "./secret.js";
import { stateOf, inNetwork, clock, linkWords, skewWords, hostLine, hostTags, outboxWords, ticketWords, proposalWords, remoteAgents, remoteWaits, hostIdProblem, joinOf, joinFields, KIND_WORDS, TICKET_MS, STARTS, WAKES, START_WORDS, WAKE_WORDS, reachOf, hostCan, widenWords } from "./network-model.js";

const EVERY_IN = 10e3;
// What a session starts as, in the Start a session box.
const START_APPS = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
  ["grok", "Grok Build"],
];
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

// An address, hidden while identities are (Hide identities), as the
// gateway's is in the Actors panel.
const addrHtml = (text) => `<span class="addr-real">${h(text)}</span><span class="addr-masked">address hidden</span>`;

// `calls` are the window's own (actor-requests.js); `message(id)` opens the
// box that writes to a session as you; `gateway()` opens the Actors panel,
// where the gateway's address is set; `onChange(view)` hears each new read.
export function createNetwork({ modals, calls, sfx = () => {}, toast = () => {}, onChange = () => {}, message = () => {}, gateway = () => {}, projects = () => [], request = (...a) => globalThis.fetch(...a) }) {
  let state = stateOf(null);
  let agents = [];
  let read = false;
  let seq = 0;
  let timer = null;
  let panel = null;
  let view = { name: "main" };
  let drawn = "";
  let busy = false;
  // The join line on screen: who it's for and when it lapses. The line
  // itself is kept as a secret (secret.js), never here.
  let joinTicket = null;
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
    if (list.some((a) => typeof a?.id === "string" && a.id.startsWith("@"))) return list;
    const at = await getJson("/api/engine/agents").catch(() => null);
    return Array.isArray(at) ? at : list;
  }

  // The state and other machines' agents, read again.
  async function refresh() {
    const n = ++seq;
    const next = stateOf(await getJson("/api/net/state").catch(() => null));
    const list = inNetwork(next) ? await engineAgents() : [];
    if (n !== seq) return state;
    state = next;
    agents = remoteAgents(list);
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
      <p class="pj-note">A network joins the machines you use, each running XenoNet. The sessions on each can see and message each other, what waits on you shows in every console, and you can message any session from any of them. One machine is the gateway: the others reach it, and it passes what they send on without reading it.</p>
      <ol class="ac-list net-choices">
        <li class="ac-row"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">Start a network</span></div><div class="ac-what">This machine is the first one in it, and its gateway.</div></div><div class="ac-acts"><button class="act primary" data-net-go="start">Start a network…</button></div></li>
        <li class="ac-row"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">Join one</span></div><div class="ac-what">Another machine is in a network: its Add a machine shows a line to run here.</div></div><div class="ac-acts"><button class="act" data-net-go="join">Join one…</button></div></li>
      </ol>`;
  }

  function gatewayHtml() {
    const g = state.gateway;
    const w = linkWords(g);
    // Where it is, beside its address; how the link stands, under it.
    const where = g ? [KIND_WORDS[g.kind], state.self?.gateway ? "this machine" : null].filter(Boolean).join(" · ") : "";
    const addr = g?.address ? `<code class="ac-gw-addr" data-net-address>${addrHtml(g.address)}</code>` : "";
    const setup = !g || state.self?.gateway ? `<button class="act${g ? "" : " primary"}" data-net-gateway-setup title="The gateway's tunnel or LAN address is set in the Actors panel">${g ? "Change the address…" : "Set the address…"}</button>` : "";
    return `<div class="ac-gw" data-state="${h(w.state)}" data-net-gateway data-net-link="${h(g?.link ?? "none")}">
        <div class="ac-gw-line"><span class="ac-gw-dot"></span><span class="ac-gw-word" data-net-link-word>${h(w.word)}</span>${addr}${where ? `<span class="net-where">${h(where)}</span>` : ""}<span class="grow"></span>${setup}</div>
        <div class="ac-gw-sub" data-net-link-sub>${h([w.sub, g && skewWords(g.skewMs)].filter(Boolean).join(" · "))}</div>
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
    return `<div class="net-reach" data-net-reach="${h(x.id)}">${controls}${pending}</div>`;
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
    const starts = me || (x.online && hostCan(x, "start")) ? `<button class="act" data-net-start-session="${h(x.id)}" title="A new Claude Code, Codex or Grok Build session there, with a first prompt">Start a session…</button>` : "";
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
      ? `<p class="net-warn" data-net-mismatch>${h(state.rosterMismatch.join(", "))} ${state.rosterMismatch.length === 1 ? "sees" : "see"} another list of the network's machines than this one. One of you missed a change to it, or the gateway is holding one back. The doctor says more.</p>`
      : "";
    const tickets = state.tickets.length
      ? `<div class="panel-title sub"><span>Waiting to join</span><span class="tick">${state.tickets.length}</span></div><ul class="net-tickets" data-net-tickets>${state.tickets.map((t) => `<li>${h(ticketWords(t))}</li>`).join("")}</ul>`
      : "";
    // What waits for the founder or a second window host: a window host
    // signs it too here, unless it has.
    const mine = (p) => p.signers?.includes(state.self?.id);
    const approvals = state.proposals.length
      ? `<div class="panel-title sub"><span>Waiting for approval</span><span class="tick">${state.proposals.length}</span></div><ul class="net-tickets" data-net-proposals>${state.proposals
          .map((p) => `<li data-net-proposal="${h(p.id)}">${h(proposalWords(p))}. It needs the founder or a second window host.${state.self?.window && !mine(p) ? ` <button class="act" data-net-approve="${h(p.id)}" data-yours>Approve</button>` : ""}</li>`)
          .join("")}</ul>`
      : "";
    return `${gatewayHtml()}${mismatch}
      <div class="panel-title sub"><span>Machines</span><span class="tick">${hosts.length}</span></div>
      <ol class="ac-list" data-net-hosts>${hosts.map((x) => hostHtml(x, view.confirm)).join("")}</ol>
      ${approvals}${tickets}
      <p class="net-outbox" data-net-outbox>${h(outboxWords(state.outbox))}</p>
      <div class="panel-title sub"><span>This machine</span></div>
      <label class="net-check"><input type="checkbox" data-net-titles data-yours ${state.titlesOnly ? "checked" : ""}> Share with the network: titles only</label>
      <p class="pj-note">On, the other consoles see this machine's sessions by their titles alone: not the text of a question waiting, the last message, or why a session seems stuck.</p>
      <label class="net-check"><input type="checkbox" data-net-load data-yours ${state.shareLoad !== false ? "checked" : ""}> Share load with the network</label>
      <p class="pj-note">On, the window hosts' consoles see this machine's CPU, GPU and memory, and what its parts are, in their Machines panel while they watch it.</p>
      ${settingHtml("start", state.start)}
      ${settingHtml("wake", state.wake)}
      <p class="pj-note">Who may start a session on this machine: nobody, you from a window host's console, or the network, which adds the sessions and actors on window hosts. Who may wake a session here to take a message at once: the sessions you allowed with <code>xenonet live allow</code>, or anyone in the network, within the wake budget. A window host can narrow these from its console; widening to the network from another machine takes the founder or a second window host.</p>
      ${selfWidens()}
      ${errorHtml(view.error)}
      <div class="m-foot"><span class="grow"></span><button class="act primary" data-net-go="add">Add a machine…</button></div>`;
  }

  function startHtml() {
    const lead = view.from === "add" ? `<p class="pj-note">A machine joins a network, so adding one starts with a network here.</p>` : "";
    return `<p class="ac-lead">Start a network here</p>${lead}
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Its name</span><input class="ac-input" data-net-name value="home" maxlength="64" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">This machine's id</span><input class="ac-input" data-net-self placeholder="studio" maxlength="32" spellcheck="false" autocomplete="off"></label>
      </div>
      <p class="pj-note">The id is what every console calls this machine: lowercase letters, digits and dashes. Pick a name, since XenoNet never uses the machine's own. This machine is the network's founder, whose key is its root, and its gateway. The other machines reach the gateway at an address, a tunnel or this machine's address on your LAN, which you set next. In a terminal: <code>xenonet net create --name &lt;name&gt; --host &lt;id&gt;</code></p>
      ${errorHtml(view.error)}
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span><button class="act primary" data-net-create data-yours>Start the network</button></div>`;
  }

  function joinHtml() {
    return `<p class="ac-lead">Join a network</p>
      <p class="pj-note">On a machine that's in the network, choose Add a machine. It shows one line to run here, in a terminal:</p>
      <pre class="ac-message">xenonet net join &lt;address&gt; &lt;ticket&gt;</pre>
      <p class="pj-note">Or paste that line, or its address and ticket, here. The ticket works once, for ten minutes.</p>
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">Address</span><input class="ac-input" data-net-join-address placeholder="gw.example.net" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">Ticket</span><input class="ac-input" type="password" data-net-join-ticket data-yours placeholder="KX7Q-49MZ-T2" spellcheck="false" autocomplete="off"></label>
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
      <div class="m-foot"><button class="act" data-net-back>Cancel</button><span class="grow"></span>${view.error && /address/i.test(view.error) ? `<button class="act" data-net-gateway-setup>Set the address…</button>` : ""}<button class="act primary" data-net-make data-yours>Make the join line</button></div>`;
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
      <p class="pj-note" data-net-line-note>It works once, until ${h(clock(view.expires))}: ten minutes. XenoNet keeps only a hash of its ticket, so it can't show it again.${view.replaces ? ` ${h(view.host)}'s old keys stop working once it joins with this one.` : ""}${view.window ? ` From its window, ${h(view.host)} can act for you.` : ""}</p>
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
    const mode = app === "codex" ? "" : (view.mode ?? "");
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
      <p class="pj-note">It runs headless through XenoNet, and shows on the map like any other session. Its first prompt says you started it from this console. Permission prompts it can't settle by its settings come to Needs you, here and on every window host. Codex runs with its own config's sandbox and approvals. In a terminal: <code>xenonet start --on &lt;machine&gt; "&lt;prompt&gt;"</code></p>
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
    // Codex takes its own config's: no mode.
    if (key === "app") for (const b of body()?.querySelectorAll("[data-net-start-mode]") ?? []) b.disabled = value === "codex" && b.dataset.netStartMode !== "";
    sfx("toggle");
  }

  async function sendStart() {
    if (!armed()) return { ok: false, error: "not armed yet" };
    const b = body();
    const host = view.host || null;
    const folder = b.querySelector("[data-net-start-folder]").value.trim() || "~";
    const prompt = b.querySelector("[data-net-start-prompt]").value;
    const app = view.app ?? "claude";
    const mode = app === "codex" ? "" : (view.mode ?? "");
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

  const VIEWS = { main: () => (inNetwork(state) ? memberHtml() : aloneHtml()), start: startHtml, join: joinHtml, add: addHtml, line: lineHtml, session: sessionHtml };

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
    if ((view.name === "add" || view.name === "start" || view.name === "join" || view.name === "session") && drawn === view.name) return;
    const html = VIEWS[view.name]?.() ?? VIEWS.main();
    if (view.name === "main" && html === drawn) return;
    b.innerHTML = html;
    drawn = view.name === "main" ? html : view.name;
    if (view.name === "session") arm();
    b.querySelector("[data-net-add-host], [data-net-self], [data-net-join-address]")?.focus();
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
      // The address problem gets its fix beside it.
      if (view.name === "add" && /address/i.test(error) && !body().querySelector(".m-foot [data-net-gateway-setup]")) body().querySelector("[data-net-make]")?.insertAdjacentHTML("beforebegin", `<button class="act" data-net-gateway-setup>Set the address…</button>`);
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

  // Shows a ticket's line once, masked, for `host`.
  function showLine(r, host, window, replaces = null) {
    const join = joinOf(r.data);
    if (!join) return failed({ status: 502, error: "XenoNet made a ticket but sent no line to show." });
    forgetSecrets();
    keepSecret(SECRET, join.secret, join.line);
    joinTicket = join.secret;
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
      const r = await calls.call("POST", "/api/net/ticket", { host, window });
      return r.ok ? showLine(r, host, window) : failed(r, `xenonet net add-machine ${host}${window ? " --window" : ""}`);
    });
  }

  async function replace(host) {
    const x = state.hosts.find((y) => y.id === host);
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/replace", { host, window: !!x?.window });
      return r.ok ? showLine(r, host, !!x?.window, host) : failed(r);
    });
  }

  async function remove(host) {
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/remove", { host });
      if (!r.ok) return failed(r, `xenonet net remove ${host}`);
      sfx("success");
      toast(`${host} is out of the network. Its frames are refused from now on.`);
      view = { name: "main" };
      await refresh();
      return { ok: true };
    });
  }

  async function create() {
    const name = body().querySelector("[data-net-name]").value.trim();
    const host = body().querySelector("[data-net-self]").value.trim();
    if (!name) return failed({ status: 400, error: "Give the network a name." });
    const problem = hostIdProblem(host, null);
    if (problem) return failed({ status: 400, error: problem });
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/create", { name, host });
      if (!r.ok) return failed(r, `xenonet net create --name ${name} --host ${host}`);
      sfx("success");
      await refresh();
      toast(state.gateway ? `The network ${name} is started.` : `The network ${name} is started. Set the gateway's address next.`);
      go({ name: "main" });
      return { ok: true };
    });
  }

  async function join() {
    const { address, ticket } = joinFields(body().querySelector("[data-net-join-address]").value, body().querySelector("[data-net-join-ticket]").value);
    body().querySelector("[data-net-join-ticket]").value = "";
    if (!address || !ticket) return failed({ status: 400, error: "Give the address and the ticket from the line Add a machine showed." });
    return busyWith(async () => {
      let r = await calls.call("POST", "/api/net/join", { address, ticket });
      // A ticket made on a machine that isn't the gateway waits for that
      // machine to sign this one in, or for a second approval: say why, and
      // ask again until the ticket lapses.
      const until = Date.now() + TICKET_MS;
      let said = false;
      while (r.ok && r.data?.waiting && Date.now() < until) {
        if (!said) toast(r.data.message ?? "The join waits for the machine that made the ticket.");
        said = true;
        await new Promise((done) => setTimeout(done, 3000));
        r = await calls.call("POST", "/api/net/join", { address, ticket });
      }
      if (r.ok && r.data?.waiting) return failed({ status: 409, error: r.data.message ?? "The ticket lapsed while the join waited." });
      if (!r.ok) return failed(r, "xenonet net join <address> <ticket>");
      sfx("success");
      await refresh();
      toast(inNetwork(state) ? `This machine is in ${state.network.name} now.` : "Joined.");
      go({ name: "main" });
      return { ok: true };
    });
  }

  async function approve(id) {
    const p = state.proposals.find((x) => x.id === id);
    return busyWith(async () => {
      const r = await calls.call("POST", "/api/net/approve", { id });
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
      const r = await calls.call("PUT", "/api/net/settings", { ...(host ? { host } : {}), [setting]: value });
      if (!r.ok) return failed(r, `xenonet net set${host ? ` --on ${host}` : ""} --${setting} ${value}`);
      sfx("toggle");
      const what = setting === "start" ? "start sessions" : "wake sessions";
      if (!host) toast(`Who may ${what} here: ${words.toLowerCase()}.`);
      else if (value === "network" && !state.self?.founder) toast(`Sent to ${host}. Letting the network ${what} there waits for the founder or another window host.`);
      else toast(`Sent to ${host}: who may ${what} there, ${words.toLowerCase()}.`);
      await refresh();
      return { ok: true };
    });
  }

  // Another window host asked to widen a machine's settings: a second
  // window host's approval, sent to that machine.
  async function approveWiden(host, id) {
    return busyWith(async () => {
      const r = await calls.call("PUT", "/api/net/settings", { host, approve: id });
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

  async function titlesOnly(on) {
    const r = await calls.call("PUT", "/api/net/settings", { titlesOnly: on });
    if (!r.ok) {
      failed(r);
      return refresh();
    }
    sfx("toggle");
    toast(on ? "The other consoles see this machine's sessions by title alone now." : "The other consoles see what waits on you here in full again.");
    return refresh();
  }

  async function shareLoad(on) {
    const r = await calls.call("PUT", "/api/net/settings", { shareLoad: on });
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
      joinTicket = null;
      return go({ name: "main" });
    }
    if (t.closest("[data-net-gateway-setup]")) return gateway();
    if (t.closest("[data-net-make]")) return make();
    if (t.closest("[data-net-create]")) return create();
    if (t.closest("[data-net-join]")) return join();
    if (d.netMessage) return message(d.netMessage);
    if (d.netRemove) return go({ name: "main", confirm: d.netRemove });
    if (t.closest("[data-net-keep]")) return go({ name: "main" });
    if (d.netRemoveYes) return remove(d.netRemoveYes);
    if (d.netReplace) return replace(d.netReplace);
    if (d.netApprove) return approve(d.netApprove);
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
          joinTicket = null;
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
      });
      panel.addEventListener("keydown", (e) => {
        if (e.key !== "Enter" || !e.target.matches?.("input")) return;
        panel.querySelector("[data-net-make], [data-net-create], [data-net-join]")?.click();
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

  // Text from the panel with the ticket masked, shown or not.
  const masked = (text) => (joinTicket ? text.split(joinTicket).join("••••••••••••") : text);

  // Whether the join line shows masked: its ticket isn't in the text the
  // page shows. Never the ticket itself.
  function joinState() {
    if (view.name !== "line" || !isOpen()) return null;
    const box = panel.querySelector(`[data-secret="${SECRET}"]`);
    const text = box?.querySelector(".secret-text")?.innerText ?? "";
    return { host: view.host, window: !!view.window, expires: view.expires, shown: !!box?.classList.contains("shown"), masked: !joinTicket || !text.includes(joinTicket), joined: joined(), kept: secretValue(SECRET) != null };
  }

  function driverState() {
    const g = state.gateway;
    return {
      open: isOpen(),
      view: isOpen() ? view.name : null,
      inNetwork: inNetwork(state),
      simulated: !!state.simulated,
      network: state.network?.name ?? null,
      self: state.self?.id ?? null,
      gateway: g ? { address: g.address, kind: g.kind, link: g.link, words: linkWords(g).word, since: g.since ?? null, reason: g.reason ?? null, skewMs: g.skewMs ?? null } : null,
      hosts: state.hosts.map((x) => ({
        id: x.id,
        online: !!x.online,
        seen: x.seen ?? null,
        you: !!x.you,
        here: x.id === state.self?.id,
        founder: !!x.founder,
        gateway: !!x.gateway,
        window: !!x.window,
        agents: x.agents ?? 0,
        waitsOnYou: x.waitsOnYou ?? 0,
        reach: reachOf(x),
        can: Array.isArray(x.can) ? x.can : [],
        line: hostLine(x),
        row: isOpen() ? (panel.querySelector(`[data-net-host="${CSS.escape(x.id)}"] [data-net-host-line]`)?.textContent ?? null) : null,
        remote: agents.filter((a) => a.host === x.id).map((a) => ({ id: a.id, title: a.title, app: a.app, status: a.status, waits: a.waits, text: a.text, stuck: a.stuck, lastMessage: a.lastMessage, repoKey: a.repoKey })),
      })),
      tickets: state.tickets.map((t) => ({ host: t.host, window: !!t.window, expires: t.expires })),
      proposals: state.proposals.map((p) => ({ id: p.id, op: p.op, host: p.host, window: !!p.window, gateway: p.gateway ?? null, signers: p.signers ?? [], words: proposalWords(p) })),
      outbox: { frames: state.outbox.frames ?? 0, bytes: state.outbox.bytes ?? 0, oldest: state.outbox.oldest ?? null },
      mismatch: state.rosterMismatch,
      titlesOnly: !!state.titlesOnly,
      shareLoad: state.shareLoad !== false,
      start: state.start ?? "you",
      wake: state.wake ?? "consent",
      waits: remoteWaits(state, agents).filter((w) => w.kind === "remote-need").map((w) => ({ agent: w.id, host: w.host, text: w.text, chime: w.chime })),
      stuck: remoteWaits(state, agents).filter((w) => w.kind === "remote-stuck").map((w) => ({ agent: w.id, host: w.host, text: w.text })),
      join: joinState(),
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
      at: isOpen() ? { make: centre("[data-net-make]"), create: centre("[data-net-create]"), join: centre("[data-net-join]"), show: centre("[data-secret-show]"), titles: centre("[data-net-titles]"), load: centre("[data-net-load]"), add: centre('[data-net-go="add"]'), approve: centre("[data-net-approve]"), set: centres("data-net-set"), setOn: centres("data-net-set-on"), widen: centres("data-net-widen-approve"), startSession: centres("data-net-start-session") } : null,
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
    // One of another machine's agents, by its @<host>: id.
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
    // GET /api/net/state as this window reads it, with its key: what the
    // service serves, before the page reads it leniently.
    served: () => getJson("/api/net/state"),
  };
}
