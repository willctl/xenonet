// Running steps against a XenoNet Dev instance (core/control.js), one after
// another: the steps xenonet console takes on its command line
// ("select:atlas-api"), and scripts, which are JSON lists of the same steps
// as strings or as { op, args }: a driver method (src/driver.js) and its
// arguments, which is how the console logs what you do (xenonet script
// record). A JS module can do anything else (xenonet script run). Steps wait
// for the view to settle, so a script never sleeps for a fixed time unless
// it says so.

export const STEPS = `  launch[:inactive|front|hidden]  start it unless it's running, and wait until it's ready
  launch:restart[:…]              quit it if it's running, or end a copy stuck holding its
                                  lock, then start it afresh (launch:restart:hidden, say)
  quit                            quit it
  status / state                  the instance and its window / the page's state
  find:<words>                    nodes matching the words, with their ids
  select:<id or words>            select a node (a project gathers its sessions)
  deselect, back, next, prev, reset
  command:<id>                    a palette command; commands lists them
  stats:<id or words>             a session's stats panel
  simulate:<kind>[,a[,b]]         needs-you, collision, stuck, wiki-write, turn-end, doctor, clear;
                                  or an actor acting: actor-spawn, actor-edit, actor-message,
                                  actor-online, actor-offline (a: the actor, b: what it acts on);
                                  or actor-request[,name]: a made-up program asking to be an
                                  actor, waiting for an Allow, and actor-needs-you[,actor]: an
                                  agent waiting on you with a made-up question until clear,
                                  actor-tunnel[,actor] and actor-tunnel-down[,actor]: a
                                  made-up connection through a tunnel, with an adapter
                                  connected or not, and actor-read[,actor[,node]]: a read
                                  through it (all demo and test instances only);
                                  with XENONET_HOSTS=1 also host-add,<id>,<port> (another XenoNet
                                  here, shown as a host), host-offline and host-online (a: the
                                  host, the first if left out) and host-skew[,host[,ms]]
  actors                          the actors beside the map: presence, status, beams; and the
                                  programs waiting for an Allow
  feed[:<n>]                      the activity feed's last lines, sessions' and actors'
  filters[:<name>=on|off,…]        the toggles under Filters (claude, codex, grok, archived,
                                  external); with names, turn those on or off
  identities[:<keys>|all]         the identity chips; with chip keys (comma-separated), only their work
  mask:on|off                     hide identities (GitHub A, Claude B…) or show them
  hosts[:<ids>|all]               other machines on the map (XENONET_HOSTS=1) and how each
                                  stands; with host ids (comma-separated, @here for this
                                  machine), only their work
  root[:<folder>|reset]           the projects folder; with a folder (~/… or a full path), read
                                  that one instead, or reset to go back to ~/Projects
  key:<combo>                     Mod+K, Escape, ArrowDown, W… (Mod is ⌘ on a Mac, Ctrl elsewhere)
  type:<text>                     typed into whatever has focus
  click:<x,y | @id>               a real click; click2: double-clicks
  hover:<x,y | @id>               moves the pointer there
  drag:<x1,y1,x2,y2>              alt-drag: holds Alt, which moves a node
  scroll:<x,y,dy>                 a wheel turn
  size:<1600x900[@2] | window>    the page's size
  show / hide                     the window (shown without taking focus)
  still:on|off                    hold ambient animation still
  settle                          wait for the view to settle
  shot:<name>                     .xenonet/<name>.png, held still and settled
  eval:<js>                       an expression, or a body with return
  logs                            recent warnings and errors
  listen:<ms>                     new warnings and errors over that long
  theme:<name>                    switch the theme, live, and wait until it's drawn
  themes                          the themes, and the one in use
  environment:<id>                choose nebula, aurora, trench or vault independently of the theme
  environments                    the environments and the one in use
  environmentPicker               preview environment cards; keepEnvironment or revertEnvironment
  mcp:<node-id>                    open a shared system's MCP details; mcpTab and mcpSearch browse it
  previewTheme:<name>             show a theme in this window alone; keepTheme
                                  or revertTheme after
  themeSwitcher[:off]             "Theme…": the switcher; stepTheme[:-1] moves
                                  through it, showing each theme (keepTheme or
                                  revertTheme after, as Enter and Esc do)
  themeGallery[:<name>|off]       the gallery of themes; focusTheme:<name> looks
                                  at a card, makeTheme makes your own from it
  film[:<seconds>]                play a film of the day and wait for its end
  sleep:<ms>
  <method>[:<arg>]                any other driver method (src/driver.js), such as ops or clearOps`;

const point = (arg) => {
  if (arg.startsWith("@")) return { node: arg.slice(1) };
  const [x, y] = arg.split(",").map(Number);
  return { x, y };
};
export const brief = (st) => ({ selection: st.selection?.id ?? null, gathered: st.gathered ? `${st.gathered.members.length} around ${st.gathered.root}` : null, overlays: Object.keys(st.overlays).filter((k) => st.overlays[k]), alerts: st.alerts.length, settled: st.motion.settled });

// Selects a node by id, or else the best match for the words.
async function select(s, arg) {
  try {
    return await s.driver("select", arg);
  } catch (err) {
    if (!/has no node/.test(err.message)) throw err;
  }
  const { nodes } = await s.driver("find", { query: arg, limit: 1 });
  if (!nodes.length) throw new Error(`nothing on the map matches "${arg}"`);
  return s.driver("select", nodes[0].id);
}

// Plays a film of the day (src/film.js) and waits for its end card to go.
async function film(s, seconds) {
  await s.driver("film", { seconds });
  for (let seen = false; ; await new Promise((r) => setTimeout(r, 400))) {
    const st = await s.driver("state");
    if (st.overlays.film) seen = true;
    else if (seen) return st;
  }
}

// One step: a string ("select:atlas-api") or { op, args }. `say` gets what
// each step has to show.
export async function runStep(s, step, say = () => {}) {
  if (step && typeof step === "object") {
    const { op, args = [] } = step;
    if (op === "wait") return new Promise((r) => setTimeout(r, Number(args[0]) || 0));
    if (op === "shot") return runStep(s, `shot:${args[0] ?? "shot"}`, say);
    if (op === "film") return say(brief(await film(s, args[0]?.seconds ?? args[0] ?? 45)));
    if (op === "theme") return runStep(s, `theme:${args[0]}`, say);
    return say(brief(await s.driver(op, ...args)));
  }
  const show = async (pending) => say(brief(await pending));
  const i = String(step).indexOf(":");
  const [cmd, arg] = i < 0 ? [step, ""] : [step.slice(0, i), step.slice(i + 1)];
  switch (cmd) {
    case "launch": {
      // restart and how the window shows, either or both, in either order.
      const opts = {};
      for (const word of arg ? arg.split(":") : []) {
        if (word === "restart") opts.restart = true;
        else if (["inactive", "front", "hidden"].includes(word)) opts.show = word;
        else throw new Error(`launch takes restart, inactive, front or hidden, not "${word}"`);
      }
      say(await s.launch(opts));
      break;
    }
    case "quit":
      say(await s.quit());
      break;
    case "status":
      say((await s.ping()) ?? `${s.name} isn't running`);
      break;
    case "state":
      say(await s.driver("state"));
      break;
    case "find":
      for (const n of (await s.driver("find", { query: arg, limit: 30 })).nodes) say(`${n.id}  ${n.kind}  ${n.title}${n.drawn ? "" : "  (not drawn)"}`);
      break;
    case "select":
      await show(select(s, arg));
      break;
    case "deselect":
      await show(s.driver("select", null));
      break;
    case "back":
    case "reset":
      await show(s.driver(cmd));
      break;
    case "next":
    case "prev":
      await show(s.driver("step", cmd === "next" ? 1 : -1));
      break;
    case "stats": {
      // By id, or the best match for the words; the map needn't settle.
      const id = arg.includes(":") ? arg : (await s.driver("find", { query: arg, kind: "session", limit: 1 })).nodes[0]?.id;
      if (!id) throw new Error(`no session matches "${arg}"`);
      const st = await s.driver("stats", id, { settle: false });
      say({ overlays: Object.keys(st.overlays).filter((k) => st.overlays[k]), stats: st.stats });
      break;
    }
    case "command":
      await show(s.driver("command", arg));
      break;
    case "commands":
      for (const c of await s.driver("commands")) say(`${c.id}  ${c.label}`);
      break;
    case "simulate": {
      // An actor and what it acts on may follow the kind, after commas;
      // ids have colons in them, never commas.
      const [kind, a, b] = arg.split(",");
      // A made-up registration's token is still a token: it isn't printed.
      const { token, ...done } = await (a || b ? s.simulate(kind, { a: a || undefined, b: b || undefined }) : s.simulate(kind));
      say(done);
      break;
    }
    case "actors": {
      const seen = await s.driver("actors");
      const gates = new Map((seen.tunnels ?? []).map((g) => [g.actor, g]));
      for (const a of seen.actors) say(`${a.id}  ${a.presence}  ${a.title}${a.status ? ` · ${a.status}` : ""}${a.beams.length ? `  → ${a.beams.length} beam${a.beams.length === 1 ? "" : "s"}` : ""}${gates.has(a.id) ? `  ⊙ tunnel ${gates.get(a.id).open >= 0.5 ? "open" : "shut"}` : ""}`);
      const { requests = [] } = (await s.driver("actorRequests").catch(() => null)) ?? {};
      for (const r of requests) say(`${r.actor}  waiting  ${r.name} asks to join${r.asker ? ` · asked by ${r.asker.program}` : ""}`);
      break;
    }
    case "feed":
      for (const l of (await s.driver("feed", { limit: Number(arg) || 20 })).lines) say(`${l.kind}  ${l.title}${l.detail ? ` · ${l.detail}` : ""}`);
      break;
    case "filters": {
      const set = arg ? Object.fromEntries(arg.split(",").map((pair) => pair.split("=")).map(([name, v]) => [name.trim(), !/^(off|false|0|no)$/i.test(String(v ?? "on").trim())])) : null;
      const st = set ? await s.driver("filters", set) : await s.driver("state");
      say(Object.entries(st.filters ?? {}).map(([k, v]) => `${k} ${v ? "on" : "off"}`).join("  "));
      break;
    }
    case "identities":
      if (arg) await show(s.driver("filterIdentities", arg === "all" ? [] : arg.split(",")));
      say(await s.driver("identities"));
      break;
    case "mask":
      await show(s.driver("maskIdentities", arg !== "off"));
      break;
    case "hosts":
      if (arg) await show(s.driver("filterHosts", arg === "all" ? [] : arg.split(",")));
      for (const x of (await s.driver("hosts")).hosts) say(`${x.id}  ${x.state}${x.stale ? "  (not live)" : ""}  ${x.sessions} sessions${x.drawn ? "" : "  (not drawn)"}`);
      break;
    case "root": {
      if (!arg) {
        const r = await s.driver("root");
        say(`${r.path}  ${r.source}`);
        break;
      }
      const st = await s.driver("setRoot", arg === "reset" ? null : arg);
      say({ ...brief(st), root: st.root.path });
      break;
    }
    case "key":
      say((await s.input({ keys: arg })).did);
      break;
    case "type":
      say((await s.input({ text: arg })).did);
      break;
    case "click":
    case "click2":
      say((await s.input({ click: point(arg), count: cmd === "click2" ? 2 : 1 })).did);
      break;
    case "hover":
      say((await s.input({ move: point(arg) })).did);
      break;
    case "drag":
    case "alt-drag": {
      const [x1, y1, x2, y2] = arg.split(",").map(Number);
      say((await s.input({ drag: { from: { x: x1, y: y1 }, to: { x: x2, y: y2 } }, modifiers: cmd === "alt-drag" ? ["Alt"] : [] })).did);
      break;
    }
    case "scroll": {
      const [x, y, dy] = arg.split(",").map(Number);
      say((await s.input({ scroll: { x, y, dy } })).did);
      break;
    }
    case "size":
      say(await s.window({ size: arg === "window" ? null : arg }));
      break;
    case "show":
      say(await s.window({ show: arg || "inactive" }));
      break;
    case "hide":
      say(await s.window({ hide: true }));
      break;
    case "still":
      say({ still: await s.driver("still", arg !== "off") });
      break;
    case "settle":
      say(await s.driver("settled"));
      break;
    case "shot": {
      const { file, width, height, notes } = await s.screenshot({ name: arg || "shot", inline: false });
      say(`shot ${file} (${width}×${height})${notes.length ? ` ${notes.join("; ")}` : ""}`);
      break;
    }
    case "eval":
      say(await s.eval(arg));
      break;
    case "logs":
      for (const e of (await s.logs({ level: "warning" })).entries) say(`${e.level.toUpperCase()} [${e.source}] ${e.text}`);
      break;
    case "listen": {
      const { cursor } = await s.logs({ limit: 1 });
      await new Promise((r) => setTimeout(r, Number(arg) || 3000));
      const { entries } = await s.logs({ since: cursor, level: "warning" });
      say(entries.map((e) => `${e.level.toUpperCase()} [${e.source}] ${e.text}`).join("\n") || "no errors");
      break;
    }
    // Through the page, which switches without a reload and returns once
    // the theme is drawn and the view has settled.
    case "theme":
      await s.driver("theme", arg);
      say(`theme ${arg}`);
      break;
    case "film":
      say(brief(await film(s, Number(arg) || 45)));
      break;
    case "sleep":
      await new Promise((r) => setTimeout(r, Number(arg)));
      break;
    // Any other driver method (src/driver.js), with the text after the
    // colon as its one argument.
    default: {
      const st = await s.driver(cmd, ...(arg ? [arg] : [])).catch((err) => {
        throw /the driver has no/.test(err.message) ? new Error(`no step "${cmd}"; xenonet console --help lists them`) : err;
      });
      say(st?.overlays ? brief(st) : st);
    }
  }
}

export async function runScript(s, steps, say) {
  for (const step of steps) await runStep(s, step, say);
}
