// Whether a request made in an agent's name comes from that agent. Any
// program on this machine can reach XenoNet's port and say it's any agent; the
// process on the other end of the connection can't lie about where it runs
// (procs.js). So:
//
//   1. A request in a Claude Code session's name must come from a Claude
//      Code process or from under one (its MCP server, its hooks, its
//      shell), and a Codex thread's from a Codex process or from under it.
//   2. Claude Code sends its HTTP hooks from its own process, which makes
//      them the word on which process holds which session: when that
//      process is signed by Anthropic, they pin the session to it (its pid
//      and start time, since pids are reused). A name is easy to copy; the
//      signature isn't.
//   3. A request for a session pinned to a process that's still running
//      must come from under that process. A session with no such pin goes
//      to the first process that names it, until Claude Code's own hooks
//      say otherwise.
//
// Connecting your agents changes Claude Code's and Codex's settings, so it's
// yours alone: refused from under any agent, and allowed from XenoNet's own
// window (`home`, the app the server runs in, or the Desktop connected to
// the service; see desktop) even when an agent started XenoNet. This keeps agents from changing your setup in passing; it isn't a
// wall, since a process cut loose from its agent looks like nobody's, but
// all it can do is add or take out XenoNet's own hooks.
//
// The window's own routes (the person at the console's, with the key only
// the window holds; see core/actors/key.js) are stricter still: from a
// process of XenoNet's own app, which is where the page's requests come from,
// with no agent nearer. A plain terminal, a program run in XenoNet's own
// terminal, and anything under an agent are all refused, key or not.
//
// Where these checks rest on a path (XenoNet's own bundle, Hermes's agent
// hosts), it's the executable's real path, never argv[0], which a process
// sets itself; procs.chain reads those for the window's, an actor's and a
// registration's checks.
//
// Actors (docs/design/actors/interface.md, section 5) come with a token,
// and the token is held to processes too, so one an agent finds is no use
// to it:
//
//   - never from under Claude Code or Codex;
//   - never from an actor app's own process (Grok Bot's, Hermes.app's, the
//     ChatGPT app's, Muse's), which could be relaying for its cloud, as
//     opposed to a command the app runs;
//   - a product's actor only from under that product (Hermes's agent
//     hosts, a command Grok Bot runs), and an actor of your own never from
//     under a product;
//   - a pid it names must be the caller or one of its parents below the
//     first app (see ownOf).
//
// Registering is held to the same rules, with the product it asks to be,
// and so are your own routes, which no product's process may use either.
//
// What this can't tell apart: threads of the same Codex process (the
// ChatGPT app runs many in one), and a session no running process has been
// seen to hold yet. Codex, and Claude Code built from source or run by
// Node, never pin for sure, so theirs go to whoever names them first. Where
// XenoNet can't look (not macOS, for now), requests go through as before, and
// an actor's token alone decides.
//
// Where XenoNet can look, a request whose process it can't see is refused:
// one netstat has no line for, or one gone before the process table was
// read (a program that sends and exits at once). Otherwise closing the
// connection early would pass for a peer XenoNet can't tell. The one
// exception is an agent's request in its own session's name when netstat
// has no line for it at all, which goes through unchecked as before, so
// agents' tools keep working should netstat ever answer in a way XenoNet
// doesn't read (the doctor's self-test says so). `state.unseen` counts the
// refusals.
const APP = { claude: "Claude Code", codex: "Codex", grok: "Grok Build" };
const PRODUCT = { "hermes-agent": "Hermes Agent", "grok-bot": "Grok Bot", muse: "Muse", dots: "the ChatGPT app" };
// The products whose apps their makers sign, by the maker's name (their
// teams are in core/procs.js): their actors register and report only from
// under the app as its maker built it, not a bundle that only calls itself
// by its name and id.
const SIGNED_BY = { "grok-bot": "Cursor", muse: "Meta" };
// A verdict on a window request is kept this long per process: the page
// reports up to once a second, and the process table costs a tenth of a
// second of CPU to read.
const WINDOW_VERDICT_MS = 60e3;
// And one on an actor's, per actor, process and the pid it names: without
// it, every heartbeat would pay for the table.
const ACTOR_VERDICT_MS = 60e3;

// Whether a chain of processes runs under a product: under one of Hermes's
// agent hosts, or for the others under their app (a command Grok Bot runs).
const under = (chain, product) => chain.some((p) => p.product?.product === product && p.product.role === (product === "hermes-agent" ? "host" : "app"));

// Whether a process is an app's (a terminal, an editor, an actor app),
// which other programs run under too. A product's agent host can live
// inside its app (Hermes.app's own Python), and is the product's, not the
// app's.
const isApp = (p) => String(p?.program ?? "").includes(".app/") && p?.product?.role !== "host";

// The processes an actor's request may call its own, nearest first: the
// caller, and its parents below the first app above it (the terminal it
// runs in, say). An app's process is never one: naming Terminal would make
// every session started in any of its windows look like the actor's.
export function ownOf(chain) {
  const app = chain.findIndex((p, i) => i > 0 && isApp(p));
  return app < 0 ? chain : chain.slice(0, app);
}

// Why an actor's request (or a request to register one, `registering`)
// can't come from this chain, nearest process first, or null when it can.
// `product` is the product the actor is, or asks to be: a known one is held
// to its own processes, and any other is an actor of your own.
export function actorRefusal(chain, { product = null, pid = null, registering = false } = {}) {
  const agent = chain.find((p) => p.agent);
  if (agent) return registering ? `a session can't register an actor, only ask you to, and this came from under a ${APP[agent.agent]} process` : `an actor's token doesn't work from under a session, and this came from under a ${APP[agent.agent]} process`;
  const own = chain[0]?.product;
  if (own?.role === "app") return `${PRODUCT[own.product]}'s own processes can't ${registering ? "register an actor" : "use an actor's token"}: they could be relaying for its cloud`;
  if (Object.hasOwn(PRODUCT, product ?? "")) {
    if (!under(chain, product)) return `${PRODUCT[product]}'s actor ${registering ? "registers" : "works"} only from under ${PRODUCT[product]}'s own processes`;
  } else {
    const inside = chain.find((p) => p.product);
    if (inside) return registering ? `a program under ${PRODUCT[inside.product.product]} can't register an actor of its own` : `an actor of your own can't use its token from under ${PRODUCT[inside.product.product]}`;
  }
  if (pid != null && !ownOf(chain).some((p) => p.pid === pid)) {
    return chain.some((p) => p.pid === pid) ? "the pid it names is an app's, which other programs run under too; name the actor's own process, or a parent of it below the app it runs in" : "the pid it names has to be the calling process or one of its parents";
  }
  return null;
}

// The app bundle a program lies in (/Applications/XenoNet.app/), or its
// folder when it isn't in one.
export function bundleOf(program) {
  const p = String(program ?? "");
  const i = p.indexOf(".app/");
  return i >= 0 ? p.slice(0, i + ".app/".length) : p.slice(0, p.lastIndexOf("/") + 1);
}

// The file a process from procs.chain runs: its executable's real path
// when the chain was asked for those (null where it couldn't be read), and
// otherwise the program ps names.
const fileOf = (p) => (p.exe === undefined ? p.program : p.exe);

export function createVerifier({ procs, store, home = null, now = () => Date.now() }) {
  const state = { checked: 0, refused: 0, unchecked: 0, unseen: 0, lastRefusal: null };
  const verdicts = new Map();
  const actorVerdicts = new Map();
  // Which desktop() call is the latest: one that finishes after the Desktop
  // went (or another came) mustn't make its process home.
  let desktops = 0;

  // The process on the other end of a socket and the ones above it: no pid
  // when netstat had no line for the connection, and no chain when the
  // process had gone before the table was read (see unseen). `exe`: with
  // the executables' real paths, for the checks that rest on a path.
  async function see(socket, { exe = false } = {}) {
    const pid = await procs.peer(socket).catch(() => null);
    if (!pid) return { pid: null, chain: [] };
    return { pid, chain: await procs.chain(pid, { exe }).catch(() => []) };
  }

  // `hook`: the request is one of the app's hooks, which for Claude Code may
  // come from Claude Code itself. Returns { ok, checked, root?, reason? }.
  async function check(socket, agent, { hook = false } = {}) {
    const app = String(agent).split(":")[0];
    if (!APP[app] || !procs.supported) return pass();
    const { pid, chain } = await see(socket);
    if (!pid) return pass();
    if (!chain.length) return unseen(agent, `requests in ${agent}'s name have to come from a ${APP[app]} process, and this one had gone before XenoNet could look`);
    state.checked++;
    const holder = chain.find((p) => p.agent === app);
    if (!holder) return refuse(agent, `requests in ${agent}'s name have to come from a ${APP[app]} process, and this one doesn't`);
    const root = { pid: holder.pid, start: holder.start };
    const same = (p) => p.pid === root.pid && p.start === root.start;
    // The pins of processes still running; the others go.
    const held = [];
    for (const p of store.pins(agent)) {
      if (await procs.alive(p)) held.push(p);
      else store.unpin(agent, p);
    }

    // Claude Code itself, in one of its HTTP hooks: this process holds the session.
    if (hook && app === "claude" && chain[0].pid === holder.pid && (await procs.signed(holder))) {
      store.pin(agent, root, true, now());
      store.dropGuesses(agent);
      return { ok: true, checked: true, root, sure: true };
    }
    if (held.some(same)) {
      store.pin(agent, root, held.find(same).sure, now());
      return { ok: true, checked: true, root };
    }
    if (held.length) return refuse(agent, `${agent} belongs to another ${APP[app]} process`);
    store.pin(agent, root, false, now());
    return { ok: true, checked: true, root };
  }

  // Requests only you make (connecting your agents, removing actors): not
  // from any agent, nor from under an actor product's processes. The nearer
  // of one of those and XenoNet's own app decides, so a claude run in XenoNet's
  // terminal is still an agent.
  async function person(socket) {
    if (!procs.supported) return pass();
    const { chain } = await see(socket);
    if (!chain.length) return unseen(null, "this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet");
    state.checked++;
    const near = chain.find((p) => p.agent || p.product || p.pid === home);
    if (near?.agent) return refuse(null, `this is yours to do, and it came from under a ${APP[near.agent]} process; run it in your own terminal or in XenoNet`);
    if (near?.product) return refuse(null, `this is yours to do, and it came from under ${PRODUCT[near.product.product]}; run it in your own terminal or in XenoNet`);
    return { ok: true, checked: true };
  }

  // A request with an actor's token (`who`: its slug and product id),
  // naming `pid` as its process or not. Says where the request comes from
  // (root) and its chain, for the checks that follow (a session it claims
  // to have spawned, say).
  async function actor(socket, who = {}, { pid = null } = {}) {
    if (!procs.supported) return pass();
    const id = who.slug ? `actor:${who.slug}` : null;
    const blind = "XenoNet couldn't see which program sent this, so it doesn't take the token from it";
    const peer = await procs.peer(socket).catch(() => null);
    if (!peer) return unseen(id, blind);
    const key = `${who.slug ?? ""}|${who.product ?? ""}|${peer}|${pid ?? ""}`;
    const kept = actorVerdicts.get(key);
    if (kept && now() - kept.at < ACTOR_VERDICT_MS) return kept.verdict;
    const chain = await procs.chain(peer, { exe: true }).catch(() => []);
    if (!chain.length) return unseen(id, blind);
    state.checked++;
    const why = actorRefusal(chain, { product: who.product ?? null, pid }) ?? (await unsigned(chain, who.product));
    const verdict = why ? refuse(who.slug ? `actor:${who.slug}` : null, why) : { ok: true, checked: true, root: { pid: chain[0].pid, start: chain[0].start }, chain };
    if (actorVerdicts.size > 256) actorVerdicts.clear();
    actorVerdicts.set(key, { at: now(), verdict });
    return verdict;
  }

  // Why a call under a product whose maker signs its app isn't that app's,
  // or null: the app process nearest the caller (the one that ran the
  // command, or the app above that helper) must be signed by the maker.
  async function unsigned(chain, product, { registering = false } = {}) {
    if (!Object.hasOwn(SIGNED_BY, product ?? "")) return null;
    const app = chain.find((p) => p.product?.product === product && p.product.role === "app");
    if (app && (await procs.signed(app).catch(() => false))) return null;
    return `${PRODUCT[product]}'s actor ${registering ? "registers" : "works"} only from under ${PRODUCT[product]}'s app as ${SIGNED_BY[product]} signed it, and the app this came from isn't signed by ${SIGNED_BY[product]}`;
  }

  // A request to register an actor, asking to be `product` (or an actor of
  // your own), naming `pid` or not: the rules an actor's token is held to,
  // and the chain and folder of the program that asked, for the card the
  // window shows.
  async function register(socket, { product = null, pid = null } = {}) {
    if (!procs.supported) return pass();
    const { pid: peer, chain } = await see(socket, { exe: true });
    if (!chain.length) return unseen(null, "XenoNet couldn't see which program asked to register, so it can't show you who asked");
    state.checked++;
    const why = actorRefusal(chain, { product, pid, registering: true }) ?? (await unsigned(chain, product, { registering: true }));
    if (why) return refuse(null, why);
    return { ok: true, checked: true, chain, cwd: (await procs.cwd?.(peer).catch(() => null)) ?? null };
  }

  // The process on the other end of a socket, or null where XenoNet can't
  // look: what the limit on bad tokens counts by.
  async function peer(socket) {
    if (!procs.supported) return null;
    return procs.peer(socket).catch(() => null);
  }

  // A running process and the ones above it, nearest first, as pids; null
  // where XenoNet can't look, and [] for a process that's gone. What's asked
  // about are sessions' pins, processes the table had when they were
  // pinned, so one missing from it is gone and isn't looked for again.
  async function lineage(pid) {
    if (!procs.supported) return null;
    return (await procs.chain(pid, { again: false }).catch(() => [])).map((p) => p.pid);
  }

  // Requests only XenoNet's own window makes: from a process of XenoNet's app
  // (its network helper, for the page), with no agent nearer. Without a
  // home (a server no app started, with no Desktop connected to it; see
  // desktop) there's no window, so nothing passes where XenoNet can look.
  async function window(socket) {
    if (!procs.supported) return pass();
    const blind = "this is for XenoNet's own window, and XenoNet couldn't see which program asked";
    const pid = await procs.peer(socket).catch(() => null);
    if (!pid) return unseen(null, blind);
    const kept = verdicts.get(pid);
    if (kept && now() - kept.at < WINDOW_VERDICT_MS) return kept.verdict;
    const chain = await procs.chain(pid, { exe: true }).catch(() => []);
    if (!chain.length) return unseen(null, blind);
    state.checked++;
    const near = chain.find((p) => p.agent || (home != null && p.pid === home));
    let verdict;
    if (near?.agent) verdict = refuse(null, `this is for XenoNet's own window, and it came from under a ${APP[near.agent]} process`);
    else if (!near) verdict = refuse(null, "this is for XenoNet's own window, and it came from outside XenoNet");
    // Every process from the caller up to XenoNet's app is the app's own, by
    // the executable's real path: a shell in XenoNet's terminal, and whatever
    // runs in it (XenoNet's own binary as Node included), isn't the window,
    // nor is a program that names itself as one of XenoNet's (exec -a).
    else if (chain.slice(0, chain.indexOf(near)).some((p) => !fileOf(p)?.startsWith(bundleOf(fileOf(near) ?? near.program)))) verdict = refuse(null, "this is for XenoNet's own window, and it came from a program run in XenoNet");
    else verdict = { ok: true, checked: true };
    if (verdicts.size > 256) verdicts.clear();
    verdicts.set(pid, { at: now(), verdict });
    return verdict;
  }

  // XenoNet's Desktop, where the server is a service of its own that no app
  // started (XenoNet from 0.6), so it has no home: the process on the other
  // end of the Desktop's connection (`socket`) becomes home, as the app
  // that starts the server is in XenoNet Dev, and the window's own routes
  // pass from it again. Only an app's own executable, by its real path,
  // that isn't an agent or an actor product's: a runtime holding the
  // service's token (the xenonet command, a script an agent runs) may say
  // it's the Desktop, but never becomes home. null: the Desktop went.
  async function desktop(socket) {
    const mine = ++desktops;
    home = null;
    verdicts.clear();
    if (socket == null) return { ok: true, checked: false };
    if (!procs.supported) return pass();
    const { pid, chain } = await see(socket, { exe: true });
    if (!pid || !chain.length) return unseen(null, "XenoNet couldn't see which program connected as its Desktop");
    state.checked++;
    const [p] = chain;
    let verdict;
    if (p.agent) verdict = refuse(null, `a ${APP[p.agent]} process can't be XenoNet's Desktop`);
    else if (p.product) verdict = refuse(null, `${PRODUCT[p.product.product]}'s processes can't be XenoNet's Desktop`);
    else if (!/\.app\/Contents\/MacOS\/[^/]+$/.test(fileOf(p) ?? "")) verdict = refuse(null, "only an app's own executable can be XenoNet's Desktop");
    else verdict = { ok: true, checked: true };
    if (verdict.ok && mine === desktops) home = p.pid;
    return verdict;
  }

  function pass() {
    state.unchecked++;
    return { ok: true, checked: false };
  }
  function refuse(agent, reason) {
    state.refused++;
    state.lastRefusal = { agent, reason, at: now() };
    return { ok: false, checked: true, reason };
  }
  // A request whose process XenoNet couldn't see, where it can look: never
  // kept as a verdict, so the next request from that process is looked at
  // afresh.
  function unseen(agent, reason) {
    state.unseen++;
    return { ...refuse(agent, reason), unseen: true };
  }

  return { check, person, window, desktop, actor, register, peer, lineage, state };
}
