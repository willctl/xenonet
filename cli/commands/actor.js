// xenonet actor (and xenonet actors, for its list): actors on this machine
// (core/actors; docs/design/actors/interface.md, section 3). Anyone may
// list them or look at one. You, or a program's setup script, ask to
// register one, which waits for you to allow it in XenoNet's window. An actor
// reports with its token, from XENONET_ACTOR_TOKEN_FILE (or
// XENONET_ACTOR_TOKEN), never from an argument: heartbeats, what it did,
// what it acts on, its inbox and messages, listening for them, and
// leaving. Removing actors, switching observers and seeing or testing how
// an actor is woken are yours, from your own terminal. There's no allow, no
// new token and no webhook here: those happen in XenoNet's window.
import path from "node:path";
import os from "node:os";
import fs from "node:fs";
import { execFileSync, spawn } from "node:child_process";
import { c, print, json, table, ago, clip, Fail } from "../lib/out.js";
import { need, get, request, nameOf } from "../../core/client.js";
import { tokenFromEnv, writeTokenFile, tokenFileRefusal } from "../../core/actors/token-file.js";
import { productOf } from "../../core/actors/products.js";
import { tilde } from "../../core/paths.js";
import { confirm } from "./connect.js";

export const help = `An actor works across sessions. It's an always-on agent, or a program of
your own that reports to XenoNet. Every registered actor waits for you to
allow it in XenoNet's window.

For anyone:
  xenonet actor list                 the actors (also xenonet actors)
  xenonet actor show <slug>          one actor and what it did lately
  xenonet actor observe [<product> on|off]
                                   the products XenoNet observes; turning one
                                   off is yours, from your own terminal

To register a program (you, or its setup script):
  xenonet actor register --name <name> [--slug <slug>] [--capabilities a,b]
      [--heartbeat <seconds>] [--product <id>] [--token-file <file>] [--force]
                                   asks, waits up to 10 minutes for you to
                                   allow it in XenoNet's window, then writes
                                   its token to the file (0600) or prints it

For an actor, with XENONET_ACTOR_TOKEN_FILE=<file> (or XENONET_ACTOR_TOKEN):
  xenonet actor heartbeat [--presence working|idle|offline] [--status <text>]
      [--instance <name>] [--cwd <dir>] [--pid <n>]
                                   it's here (beat for short); cwd is this
                                   folder, and pid this command's parent
                                   unless that's a shell, or you say
  xenonet actor status               whether it's allowed, and its unread mail
  xenonet actor event <kind> [--target <id> | --path <dir>] [--count <n>]
      [--paths <file> …] [--platform <p>] [--subject <s>] [--app <a>] [--ref <r>]
                                   what it did: spawned, edited, ran,
                                   messaged, finished or failed
  xenonet actor targets [--add <id or path>=<relation> …] [--remove <id or path>[=<relation>] …] [--replace]
  xenonet actor inbox [--wait <seconds>]
  xenonet actor listen [--exec <program>]
                                   waits for messages and reads each as it
                                   arrives: printed (as JSON lines with
                                   --json), or given to the program on its
                                   standard input, one run per message
  xenonet actor send <to> <text>     to a session or actor, "repo" or "all"
  xenonet actor leave                its token stops working

Yours, from your own terminal:
  xenonet actor remove <slug>        removes an actor that reports itself; asks
                                   first unless --yes. One XenoNet only
                                   observes goes off the map with
                                   xenonet actor observe <product> off
  xenonet actor wake <slug> [test|off]
                                   how it's woken when a message arrives;
                                   test calls its webhook now, off stops
                                   waking it. A webhook is set in XenoNet's
                                   window: Actors, then Wake… on the actor

  --json   answers as JSON`;

const DOTS = { working: c.green("●"), idle: c.dim("○"), away: c.yellow("◐"), offline: c.dim("·"), unknown: c.dim("?") };
const dot = (presence) => DOTS[presence] ?? DOTS.unknown;
const APPS = { claude: "a Claude Code session", codex: "a Codex session", grok: "a Grok Build session" };
// A node of the map as a person reads it: a place by its folder's name, a
// session by its app.
const nodeWords = (id) => {
  const m = /^(project|zone|native|claude|codex|grok):(.*)$/.exec(String(id));
  if (!m) return String(id);
  if (APPS[m[1]]) return APPS[m[1]];
  return m[1] === "native" ? "a project" : path.basename(m[2]) || m[2];
};
const slugOf = (s) => String(s).replace(/^actor:/, "");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Shells, by the name ps gives their program: its path, or a login
// shell's -name.
const SHELLS = new Set(["zsh", "bash", "sh", "dash", "fish", "ksh", "tcsh", "csh", "nu", "pwsh"]);
export const isShell = (program) => SHELLS.has(path.basename(String(program ?? "")).replace(/^-/, "").replace(/\.exe$/i, "").toLowerCase());

// The program a process runs, as ps names it, or null where that can't be
// told (no ps, as on Windows, or a process that's gone).
function programOf(pid) {
  try {
    return execFileSync("ps", ["-o", "comm=", "-p", String(pid)], { encoding: "utf8", timeout: 3000, stdio: ["ignore", "pipe", "ignore"] }).trim() || null;
  } catch {
    return null;
  }
}

// The process a heartbeat names as the actor's own when --pid doesn't say:
// this command's parent, when that's a program (Python, Node and the like),
// so the actor goes offline when that program ends. Not when the parent is
// a shell, which is your terminal's when you run it by hand, and a shell
// script's too: the sessions you start there would then pass for ones the
// actor spawned, so a script names its process with --pid. Nor when what
// the parent runs can't be told. Null for none.
export function defaultPid({ ppid = process.ppid, program = programOf } = {}) {
  if (!Number.isInteger(ppid) || ppid <= 1) return null;
  const runs = program(ppid);
  return runs && !isShell(runs) ? ppid : null;
}

// A product as a person reads it: one XenoNet knows by its id, maker and
// version, and any other as the actor's own words, quoted and said to be,
// since any program can send any id, maker and version.
function productWords(p) {
  if (productOf(p.id)) return [p.id, p.vendor && `by ${p.vendor}`, p.version].filter(Boolean).join(" ");
  const parts = [`“${p.id}”`, p.vendor && `by “${p.vendor}”`, p.version].filter(Boolean);
  return `${parts.join(" ")} ${c.dim(`(its own word${parts.length > 1 ? "s" : ""})`)}`;
}

// The token this command runs with, as an actor.
function tokenOf() {
  let found;
  try {
    found = tokenFromEnv();
  } catch (err) {
    throw new Fail(err.message);
  }
  if (!found) throw new Fail("this is for an actor: say where its token is with XENONET_ACTOR_TOKEN_FILE=<file> (or XENONET_ACTOR_TOKEN); a token never goes on the command line", 2);
  return found.token;
}
const asActor = (s, method, route, body) => request(s, method, route, body, { authorization: `Bearer ${tokenOf()}` });

// ---------- for anyone ----------

async function list(_args, flags) {
  const s = await need(flags);
  const { actors } = await get(s, "/api/actors");
  if (flags.json) return json(actors);
  // You, outside XenoNet's window, are only a placeholder.
  const shown = actors.filter((a) => a.id !== "actor:you" || a.presence !== "unknown");
  if (!shown.length) return print(c.dim("No actors yet. A program asks with xenonet actor register, and you allow it in XenoNet's window."));
  const rows = shown.map((a) => {
    const doing = a.status ?? (a.targets.length ? `on ${[...new Set(a.targets.map((t) => nodeWords(t.id)))].slice(0, 3).join(", ")}` : "");
    return [dot(a.presence), c.bold(a.name), c.dim(a.id), c.dim(a.via.filter((v) => v !== "demo").join(", ") || "demo"), a.presence, doing];
  });
  print(table(rows));
}

async function show([slug], flags) {
  if (!slug) throw new Fail("say which actor: xenonet actor show <slug>", 2);
  const s = await need(flags);
  const a = await get(s, `/api/actors/${encodeURIComponent(slugOf(slug))}`);
  if (flags.json) return json(a);
  print(`${dot(a.presence)} ${c.bold(a.name)}  ${c.dim(a.id)}`);
  const facts = [
    ["Presence", `${a.presence}${a.presenceSince ? c.dim(` since ${ago(a.presenceSince)}`) : ""}${a.lastSeen ? c.dim(` · last heard ${ago(a.lastSeen)}`) : ""}`],
    ["Status", a.status ?? c.dim("none")],
    ["Known by", a.via.join(", ")],
    a.product && ["Product", productWords(a.product)],
    ["Allowed", a.capabilities.length ? a.capabilities.join(", ") : c.dim("nothing beyond reporting itself")],
    a.heartbeatMs && ["Heartbeat", `every ${Math.round(a.heartbeatMs / 1000)} s`],
    ["Acts on", a.targets.length ? a.targets.map((t) => `${nodeWords(t.id)} (${t.relation})`).join(", ") : c.dim("nothing lately")],
  ].filter(Boolean);
  print(table(facts.map(([k, v]) => [c.dim(k), v]), { indent: 2, wrap: true }));
  if (a.recent.length) {
    print(`\n${c.bold("Lately")}`);
    print(table([...a.recent].reverse().slice(0, 10).map((e) => [c.dim(ago(e.at)), e.detail ?? e.kind]), { indent: 2, wrap: true }));
  }
}

// The products XenoNet observes, or one turned on or off (yours).
async function observe([product, state], flags) {
  const s = await need(flags);
  if (!product) {
    const { observers } = await get(s, "/api/actors/observers");
    if (flags.json) return json(observers);
    return print(table(observers.map((o) => [o.on ? c.green("on") : c.dim("off"), o.name, c.dim(o.product)])));
  }
  if (state !== "on" && state !== "off") throw new Fail("xenonet actor observe <product> on|off", 2);
  const out = await request(s, "POST", "/api/actors/observers", { product, on: state === "on" });
  if (flags.json) return json(out);
  print(`${c.green("✓")} ${nameOf(s)} ${out.on ? "observes" : "no longer observes"} ${product}.`);
}

// ---------- registering ----------

async function register(_args, flags) {
  if (typeof flags.name !== "string" || !flags.name.trim()) throw new Fail("say what it's called: xenonet actor register --name <name>", 2);
  const body = { schema: "actor/registration/v1", name: flags.name };
  if (typeof flags.slug === "string") body.slug = flags.slug;
  if (typeof flags.capabilities === "string") body.capabilities = flags.capabilities.split(",").map((x) => x.trim()).filter(Boolean);
  if (flags.heartbeat !== undefined) {
    const seconds = Number(flags.heartbeat);
    if (!(seconds > 0)) throw new Fail("--heartbeat is a number of seconds", 2);
    body.heartbeatMs = Math.round(seconds * 1000);
  }
  if (typeof flags.product === "string") body.product = { id: flags.product };
  const s = await need(flags);
  // The file is checked before asking, so no token is made for nothing.
  const file = typeof flags.tokenFile === "string" ? path.resolve(flags.tokenFile) : null;
  const projectsDir = (await get(s, "/api/workspace/root").catch(() => null))?.path ?? null;
  const refused = file && tokenFileRefusal(file, { force: !!flags.force, projectsDir });
  if (refused) throw new Fail(refused);

  const creds = await request(s, "POST", "/api/actors/register", body);
  if (!flags.json) print(`Waiting for you to allow "${flags.name}" in ${nameOf(s)}'s window (it asks for 10 minutes)…`);
  // How often to ask whether you've answered; tests ask faster.
  const every = Number(process.env.XENONET_ACTOR_WAIT_MS) || 2000;
  const until = (creds.expiresAt ?? Date.now() + 10 * 60e3) + 5000;
  for (;;) {
    await sleep(every);
    try {
      const me = await request(s, "GET", "/api/actors/me", undefined, { authorization: `Bearer ${creds.token}` });
      if (me.state === "allowed") break;
    } catch (err) {
      if (err.status === 401) throw new Fail(`${creds.id} wasn't allowed: ${err.message}.`);
      if (err.status) throw new Fail(err.message);
      // XenoNet restarting: keep waiting.
    }
    if (Date.now() > until) throw new Fail("No answer in 10 minutes, so the request lapsed. Ask again when you're at XenoNet's window.");
  }
  if (file) {
    const saved = writeTokenFile(file, creds.token, { force: !!flags.force, projectsDir });
    if (flags.json) return json({ id: creds.id, state: "allowed", file: saved, ignored: creds.ignored });
    print(`${c.green("✓")} ${creds.id} is allowed. Its token is in ${tilde(saved, os.homedir())}, readable only by you.`);
  } else {
    if (flags.json) return json({ ...creds, state: "allowed", expiresAt: undefined });
    print(`${c.green("✓")} ${creds.id} is allowed. Its token, shown once:\n${creds.token}`);
    print(c.dim("Keep it in a file only you can read (chmod 600) and pass it as XENONET_ACTOR_TOKEN_FILE; never on a command line."));
  }
  if (creds.ignored?.length) print(c.dim(`XenoNet doesn't know these capabilities, so it left them out: ${creds.ignored.join(", ")}`));
}

// ---------- for an actor, with its token ----------

async function heartbeat(_args, flags) {
  const body = { schema: "actor/report/v1" };
  if (flags.presence !== undefined) {
    if (!["working", "idle", "offline"].includes(flags.presence)) throw new Fail("--presence is working, idle or offline", 2);
    body.presence = flags.presence;
  }
  if (typeof flags.status === "string") body.status = flags.status;
  if (typeof flags.instance === "string") body.instance = flags.instance;
  body.cwd = path.resolve(typeof flags.cwd === "string" ? flags.cwd : process.cwd());
  // Its own long-lived process: the one you name, or else this command's
  // parent when that isn't a shell (defaultPid).
  const pid = flags.pid !== undefined ? Number(flags.pid) : defaultPid();
  if (flags.pid !== undefined && (!Number.isInteger(pid) || pid < 1)) throw new Fail("--pid is a process id", 2);
  if (pid > 1) body.pid = pid;
  const s = await need(flags);
  const me = await asActor(s, "POST", "/api/actors/heartbeat", body);
  if (flags.json) return json(me);
  print(`${dot(me.presence)} ${me.id} is ${me.presence}${me.unread ? `; ${me.unread === 1 ? "a message waits" : `${me.unread} messages wait`} (xenonet actor inbox)` : ""}`);
}

async function status(_args, flags) {
  const s = await need(flags);
  const me = await asActor(s, "GET", "/api/actors/me");
  if (flags.json) return json(me);
  if (me.state === "waiting") return print(`${c.yellow("…")} ${me.id} is waiting for you to allow it in ${nameOf(s)}'s window.`);
  print(`${dot(me.presence)} ${me.id} is allowed, and ${me.presence}${me.unread ? `; ${me.unread === 1 ? "a message waits" : `${me.unread} messages wait`} (xenonet actor inbox)` : ""}`);
}

async function event([kind], flags) {
  if (!kind) throw new Fail("say what it did: xenonet actor event <spawned|edited|ran|messaged|finished|failed>", 2);
  if (typeof flags.target === "string" && typeof flags.path === "string") throw new Fail("say where with --target or --path, not both", 2);
  const e = { kind };
  if (typeof flags.target === "string") e.target = flags.target;
  if (typeof flags.path === "string") e.path = path.resolve(flags.path);
  if (flags.count !== undefined) e.count = Number(flags.count);
  if (flags.paths) e.paths = [].concat(flags.paths).map((p) => path.resolve(p));
  for (const k of ["platform", "subject", "ref", "app"]) if (typeof flags[k] === "string") e[k] = flags[k];
  const s = await need(flags);
  const me = await asActor(s, "POST", "/api/actors/events", { schema: "actor/report/v1", events: [e] });
  if (flags.json) return json(me);
  if (me.dropped?.length) throw new Fail(`XenoNet didn't take it: ${me.dropped[0].reason}`);
  print(`${c.green("✓")} ${kind}`);
}

// --add and --remove take "<id or path>=<relation>": a node id (project:…,
// a session's id) or a folder, which XenoNet maps to its project.
function targetOf(spec, fallback) {
  const i = spec.lastIndexOf("=");
  const [node, relation] = i > 0 ? [spec.slice(0, i), spec.slice(i + 1)] : [spec, fallback];
  const where = /^(project|zone|native|claude|codex):/.test(node) ? { id: node } : { path: path.resolve(node) };
  return relation === undefined ? where : { ...where, relation };
}

async function targets(_args, flags) {
  const add = [].concat(flags.add ?? []).map((t) => targetOf(t, "works-on"));
  const remove = [].concat(flags.remove ?? []).map((t) => targetOf(t));
  if (!add.length && !remove.length && !flags.replace) throw new Fail("say what it acts on: xenonet actor targets --add <id or path>=<relation> …", 2);
  const s = await need(flags);
  const me = await asActor(s, "POST", "/api/actors/targets", { schema: "actor/report/v1", targets: add, remove, replace: !!flags.replace });
  if (flags.json) return json(me);
  for (const d of me.dropped ?? []) print(`${c.yellow("!")} ${[].concat(flags.add)[d.index]}: ${d.reason}`);
  print(`${c.green("✓")} ${me.accepted ?? 0} ${me.accepted === 1 ? "target" : "targets"} taken`);
}

async function inbox(_args, flags) {
  const s = await need(flags);
  const wait = Math.min(Math.max(Number(flags.wait) || 0, 0), 120);
  const messages = await asActor(s, "POST", "/api/engine/inbox", { waitSeconds: wait });
  if (flags.json) return json(messages);
  if (!messages.length) return print(c.dim("No messages waiting."));
  for (const m of messages) {
    print(`${c.dim(`#${m.id}`)} ${c.bold(m.from.title)} ${c.dim(`(${m.from.app}, ${m.from.id}) ${ago(m.at)}${m.to === "you" ? "" : `, to ${m.to}`}`)}`);
    print(`  ${m.text.replace(/\n/g, "\n  ")}`);
  }
  print(c.dim("They come from sessions and actors, not from your user."));
}

async function send([to, ...words], flags) {
  if (!to || !words.length) throw new Fail("say who it's for and what: xenonet actor send <to> <text>", 2);
  const s = await need(flags);
  const out = await asActor(s, "POST", "/api/engine/send", { to, text: words.join(" "), ...(flags.replyTo !== undefined ? { replyTo: Number(flags.replyTo) } : {}) });
  if (flags.json) return json(out);
  const who = out.to.map((r) => `"${clip(r.title, 40)}"`).join(", ");
  print(`${c.green("✓")} Sent #${out.id}${who ? ` to ${who}` : ""}${out.alsoTo ? c.dim(`, and to ${out.alsoTo}`) : ""}`);
}

async function leave(_args, flags) {
  const s = await need(flags);
  const out = await asActor(s, "DELETE", "/api/actors/me");
  if (flags.json) return json(out);
  print(`${c.green("✓")} ${out.removed} left ${nameOf(s)}; its token no longer works.`);
}

// The events of a text/event-stream body, as { event, data }.
export async function* sseEvents(body) {
  const decoder = new TextDecoder();
  let buf = "";
  let ev = { event: "message", data: [] };
  for await (const chunk of body) {
    buf += decoder.decode(chunk, { stream: true });
    for (let i; (i = buf.search(/\r?\n/)) >= 0; ) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + (buf[i] === "\r" ? 2 : 1));
      if (line === "") {
        if (ev.data.length) yield { event: ev.event, data: ev.data.join("\n") };
        ev = { event: "message", data: [] };
      } else if (!line.startsWith(":")) {
        const at = line.indexOf(":");
        const field = at < 0 ? line : line.slice(0, at);
        const value = at < 0 ? "" : line.slice(at + 1).replace(/^ /, "");
        if (field === "event") ev.event = value;
        else if (field === "data") ev.data.push(value);
      }
    }
  }
}

// The environment a program run for a message gets: this one's, without
// the actor's token, which the program doesn't need.
const execEnv = (m) => {
  const env = { ...process.env, XENONET_MESSAGE_ID: String(m.id), XENONET_MESSAGE_FROM: String(m.from?.id ?? "") };
  delete env.XENONET_ACTOR_TOKEN;
  delete env.XENONET_ACTOR_TOKEN_FILE;
  return env;
};
const EXEC_MS = 10 * 60e3;

// An actor's program that stays running hears of each message as it
// arrives, over its stream (GET /api/actors/me/stream), and reads it then.
// XenoNet going away or restarting is waited out; a token that stops working
// ends it.
async function listen(_args, flags) {
  const auth = { authorization: `Bearer ${tokenOf()}` };
  let exec = null;
  if (flags.exec !== undefined) {
    if (typeof flags.exec !== "string" || !flags.exec) throw new Fail("say which program: xenonet actor listen --exec <program>", 2);
    exec = path.resolve(flags.exec);
    try {
      fs.accessSync(exec, fs.constants.X_OK);
    } catch {
      throw new Fail(`${tilde(exec, os.homedir())} isn't a program you can run`, 2);
    }
  }
  const s = await need(flags);
  const note = (line) => !flags.json && process.stderr.write(`${c.dim(line)}\n`);
  const runFor = (m) =>
    new Promise((resolve) => {
      const child = spawn(exec, [], { env: execEnv(m), stdio: ["pipe", "inherit", "inherit"], windowsHide: true });
      const timer = setTimeout(() => child.kill(), EXEC_MS);
      child.on("error", (err) => (clearTimeout(timer), note(`${path.basename(exec)} didn't start: ${err.message}`), resolve()));
      child.on("exit", (code) => (clearTimeout(timer), code && note(`${path.basename(exec)} exited with code ${code} for message #${m.id}`), resolve()));
      child.stdin.on("error", () => {});
      child.stdin.end(`${JSON.stringify(m)}\n`);
    });
  async function drain() {
    const messages = await request(s, "POST", "/api/engine/inbox", {}, auth);
    for (const m of messages) {
      if (exec) await runFor(m);
      else if (flags.json) process.stdout.write(`${JSON.stringify(m)}\n`);
      else {
        print(`${c.dim(`#${m.id}`)} ${c.bold(m.from.title)} ${c.dim(`(${m.from.app}, ${m.from.id}) ${ago(m.at)}`)}`);
        print(`  ${m.text.replace(/\n/g, "\n  ")}`);
      }
    }
  }
  for (let delay = 1000; ; delay = Math.min(delay * 2, 30e3)) {
    const res = await fetch(`${s.base}/api/actors/me/stream`, { headers: { ...auth, accept: "text/event-stream", "x-xenonet-api-version": "1" } }).catch(() => null);
    if (res && (res.status === 401 || res.status === 403)) throw new Fail((await res.json().catch(() => ({}))).error ?? `${nameOf(s)} refused this actor's stream`);
    if (res?.ok) {
      delay = 1000;
      note(`Listening for this actor's messages on ${nameOf(s)}. Ctrl-C stops.`);
      try {
        for await (const ev of sseEvents(res.body)) {
          if (ev.event === "inbox") await drain();
          if (ev.event === "closed") throw new Fail(`${nameOf(s)} closed the stream: ${JSON.parse(ev.data).error}`);
        }
      } catch (err) {
        if (err instanceof Fail) throw err;
      }
    }
    note(`${nameOf(s)} isn't answering; trying again in ${Math.round(delay / 1000)} s`);
    await sleep(delay);
  }
}

// ---------- yours ----------

const WAKE_WORDS = { webhook: "a webhook", notify: "a notification to you", none: "nothing: it reads its inbox when it checks" };

async function wake([slug, what], flags) {
  if (!slug) throw new Fail("say which actor: xenonet actor wake <slug> [test|off]", 2);
  const id = slugOf(slug);
  const s = await need(flags);
  const route = `/api/actors/${encodeURIComponent(id)}/wake`;
  if (what === "test") {
    const r = await request(s, "POST", `${route}/test`);
    if (flags.json) return json(r);
    return r.ok ? print(`${c.green("✓")} Woke actor:${id}: ${r.wake.host} answered HTTP ${r.status}`) : print(`${c.red("✗")} ${r.why}`);
  }
  if (what === "off") {
    const r = await request(s, "DELETE", route);
    return flags.json ? json(r) : print(`${c.green("✓")} actor:${id} is woken no more; it reads its inbox when it checks`);
  }
  if (what) throw new Fail(`xenonet actor wake takes test or off, not ${what}`, 2);
  const { wakes } = await get(s, "/api/actors/wakes");
  const w = wakes.find((x) => x.actor === `actor:${id}`) ?? { actor: `actor:${id}`, kind: "none", listening: 0, last: null };
  if (flags.json) return json(w);
  print(`${c.bold(w.actor)} is woken by ${w.kind === "webhook" ? `a webhook to ${w.host}, ${w.signed ? "signed" : "with its key"} (kept in your credential store)` : WAKE_WORDS[w.kind]}`);
  if (w.listening) print(`  ${c.green("●")} a program of its own is listening, so messages reach it at once`);
  if (w.last) print(`  ${w.last.ok ? c.green("✓") : c.yellow("!")} last wake ${ago(w.last.at)}: ${w.last.ok ? `answered HTTP ${w.last.status}` : w.last.why}`);
  if (w.kind !== "webhook") print(c.dim("  A webhook is set in XenoNet's window: Actors, then Wake… on the actor."));
}

async function remove([slug], flags) {
  if (!slug) throw new Fail("say which actor: xenonet actor remove <slug>", 2);
  const id = slugOf(slug);
  if (id === "you") throw new Fail("You can't be removed: turning off Show me on the map, in XenoNet's Actors panel, deletes your record.");
  const s = await need(flags);
  // One XenoNet only observes isn't XenoNet's to remove (it would refuse):
  // said before asking, with the switch that takes it off the map.
  const a = await get(s, `/api/actors/${encodeURIComponent(id)}`);
  const observed = a.via.some((v) => v === "observed" || v === "attributed");
  if (observed && !a.via.includes("registered")) throw new Fail(`${nameOf(s)} only observes ${a.name}, so it can't be removed. Turn off its observer instead: ${a.product?.id ? `xenonet actor observe ${a.product.id} off, or ` : ""}its switch in ${nameOf(s)}'s Actors panel.`);
  if (!(await confirm(`Remove ${a.id} from ${nameOf(s)}? Its token stops working${observed ? `, and ${nameOf(s)} keeps observing it` : " and its record goes"}.`, flags))) return print(c.dim("Nothing changed."));
  const out = await request(s, "DELETE", `/api/actors/${encodeURIComponent(id)}`);
  if (flags.json) return json(out);
  print(`${c.green("✓")} Removed ${out.removed}${out.observed ? `; ${nameOf(s)} still observes it` : ""}.`);
}

const SUB = { list, show, observe, register, heartbeat, beat: heartbeat, status, event, targets, inbox, listen, send, leave, remove, wake };

export async function run([sub = "list", ...args], flags) {
  const fn = Object.hasOwn(SUB, sub) ? SUB[sub] : null;
  if (!fn) throw new Fail(`no "xenonet actor ${sub}". xenonet actor --help lists what it does.`, 2);
  return fn(args, flags);
}
