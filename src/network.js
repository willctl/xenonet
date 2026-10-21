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
//     machines' agents under them with Message, the tickets waiting, the
//     outbox, a roster mismatch, and "Share with the network: titles
//     only".
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
import { keepSecret, forgetSecrets, secretHtml, secretClick, hideOnBlur, secretValue } from "./secret.js";
import { stateOf, inNetwork, clock, linkWords, skewWords, hostLine, hostTags, outboxWords, ticketWords, remoteAgents, remoteWaits, hostIdProblem, joinOf, joinFields, KIND_WORDS } from "./network-model.js";

const EVERY_IN = 10e3;
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
export function createNetwork({ modals, calls, sfx = () => {}, toast = () => {}, onChange = () => {}, message = () => {}, gateway = () => {}, request = (...a) => globalThis.fetch(...a) }) {
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

  function agentHtml(a) {
    const what = a.waits ? (a.text || "Waiting for you") : [a.status, a.project, a.branch].filter(Boolean).join(" · ");
    return `<li class="net-agent${a.waits ? " waits" : ""}" data-net-agent="${h(a.id)}"><i class="sw ${h(a.app ?? "")}"></i><span class="net-agent-title">${h(a.title)}</span><span class="net-agent-what">${a.waits ? `<em>${h(a.kind === "permission" ? "needs permission" : "waits on you")}</em> ` : ""}${h(what)}</span><button class="ac-link" data-net-message="${h(a.id)}" title="Message this session as you">Message</button></li>`;
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
    const theirs = agents.filter((a) => a.host === x.id);
    const mismatch = state.rosterMismatch.includes(x.id) ? `<div class="ac-via net-warn">sees another list of the network's machines</div>` : "";
    return `<li class="ac-row net-host${x.online ? "" : " offline"}" data-key="${h(x.id)}" data-net-host="${h(x.id)}">
        <span class="ac-dot"></span>
        <div class="ac-main"><div class="ac-name"><span class="ac-title">${h(x.id)}</span>${tags}</div><div class="ac-what" data-net-host-line>${h(hostLine(x))}</div>${mismatch}</div>
        <div class="ac-acts">${acts}</div>
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
    return `${gatewayHtml()}${mismatch}
      <div class="panel-title sub"><span>Machines</span><span class="tick">${hosts.length}</span></div>
      <ol class="ac-list" data-net-hosts>${hosts.map((x) => hostHtml(x, view.confirm)).join("")}</ol>
      ${tickets}
      <p class="net-outbox" data-net-outbox>${h(outboxWords(state.outbox))}</p>
      <div class="panel-title sub"><span>This machine</span></div>
      <label class="net-check"><input type="checkbox" data-net-titles data-yours ${state.titlesOnly ? "checked" : ""}> Share with the network: titles only</label>
      <p class="pj-note">On, the other consoles see this machine's sessions by their titles alone: not the text of a question waiting, the last message, or why a session seems stuck.</p>
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

  const VIEWS = { main: () => (inNetwork(state) ? memberHtml() : aloneHtml()), start: startHtml, join: joinHtml, add: addHtml, line: lineHtml };

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
    if ((view.name === "add" || view.name === "start" || view.name === "join") && drawn === view.name) return;
    const html = VIEWS[view.name]?.() ?? VIEWS.main();
    if (view.name === "main" && html === drawn) return;
    b.innerHTML = html;
    drawn = view.name === "main" ? html : view.name;
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
      const r = await calls.call("POST", "/api/net/join", { address, ticket });
      if (!r.ok) return failed(r, "xenonet net join <address> <ticket>");
      sfx("success");
      await refresh();
      toast(inNetwork(state) ? `This machine is in ${state.network.name} now.` : "Joined.");
      go({ name: "main" });
      return { ok: true };
    });
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

  // ---------- for the driver ----------

  const centre = (sel) => {
    const r = panel?.querySelector(sel)?.getBoundingClientRect();
    return r?.width ? { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) } : null;
  };

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
        line: hostLine(x),
        row: isOpen() ? (panel.querySelector(`[data-net-host="${CSS.escape(x.id)}"] [data-net-host-line]`)?.textContent ?? null) : null,
        remote: agents.filter((a) => a.host === x.id).map((a) => ({ id: a.id, title: a.title, app: a.app, status: a.status, waits: a.waits, text: a.text })),
      })),
      tickets: state.tickets.map((t) => ({ host: t.host, window: !!t.window, expires: t.expires })),
      outbox: { frames: state.outbox.frames ?? 0, bytes: state.outbox.bytes ?? 0, oldest: state.outbox.oldest ?? null },
      mismatch: state.rosterMismatch,
      titlesOnly: !!state.titlesOnly,
      waits: remoteWaits(state, agents).map((w) => ({ agent: w.id, host: w.host, text: w.text, chime: w.chime })),
      join: joinState(),
      error: isOpen() ? (view.error ?? null) : null,
      // What the panel says, with a join line shown masked all the same.
      text: isOpen() ? masked(body()?.innerText ?? "").replace(/\s+/g, " ").trim().slice(0, 4000) : null,
      at: isOpen() ? { make: centre("[data-net-make]"), create: centre("[data-net-create]"), join: centre("[data-net-join]"), show: centre("[data-secret-show]"), titles: centre("[data-net-titles]"), add: centre('[data-net-go="add"]') } : null,
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
    show,
    refresh,
    // The event stream said the network changed.
    heard: () => void refresh().catch(() => {}),
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
  };
}
