// xenonet hosts: other machines' XenoNets on this one's map (docs/design/hosts.md),
// through a running XenoNet's API: this XenoNet's own host name, and the
// hosts it shows, which it reaches over SSH with an ssh config of its own.
// A prototype: the console shows hosts only with XENONET_HOSTS=1, and only
// reads from them.
import os from "node:os";
import { createInterface } from "node:readline/promises";
import { c, print, json, table, ago, Fail } from "../lib/out.js";
import { need, get, post, del, nameOf } from "../../core/client.js";
import { tilde } from "../../core/paths.js";
import { trustCommand } from "../../core/hosts/link.js";

export const help = `Other machines' XenoNets on this one's map, read over SSH (a prototype: XenoNet
shows them with XENONET_HOSTS=1). Each machine runs its own XenoNet; this one
runs ssh <host> xenonet relay for each host, with an ssh config of its own,
so your ~/.ssh/config is never read or changed.

  xenonet hosts                      the hosts, and how each is doing
  xenonet hosts self [name]          what this XenoNet calls itself as a host,
                                   or name it (a name you choose: studio, vm1);
                                   --clear takes the name away
  xenonet hosts add <id> [user@host] add one; the host is <id> when not given
      --ssh-port <n>               ssh's port, when it isn't 22
      --identity <file>            a key of its own for this host
      --known-hosts <file>         a known_hosts file of its own
      --host-key-alias <name>      the name its host key is kept under
      --xenonet-port <n>             the XenoNet to read there (47840)
      --command <path>             the xenonet command there, when ssh's
                                   PATH doesn't find it
      --since <age>                sessions this recent: 7d, 12h, 90m
      --detail                     live detail too, not only titles,
                                   states and counts
      --start                      start a XenoNet there when none is
                                   running, for as long as the relay runs
      --local                      another XenoNet on this machine instead
      --replace                    replace one of the same id
  xenonet hosts remove <id>          forget it (--forget also forgets where you
                                   placed its nodes; --yes doesn't ask)
  xenonet hosts test <id>            one try, saying how far it got
  xenonet hosts reconnect <id>       try now
  --json                           the answer as JSON

Before the first connection to a machine, accept its host key once in a
terminal, with the command xenonet hosts add prints. Adding and removing
hosts are yours to do: XenoNet refuses them from under an agent.`;

const DOT = { live: c.green("●"), connecting: c.cyan("◌"), stale: c.yellow("◐"), offline: c.dim("○"), error: c.red("✗") };
const shown = (p) => tilde(p, os.homedir());
const time = (ms) => new Date(ms).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
const offset = (ms) => (ms == null || Math.abs(ms) < 2000 ? "" : `clock ${Math.abs(ms) < 90e3 ? `${Math.round(Math.abs(ms) / 1000)} s` : `${Math.round(Math.abs(ms) / 60e3)} min`} ${ms > 0 ? "ahead" : "behind"}`);

// How a host is doing, in a line, and what to do about it, if anything.
function describe(h, now = Date.now()) {
  const what = [h.version && `XenoNet ${h.version}${h.platform ? ` on ${h.platform}` : ""}`, h.demo && "made-up data", h.calls && `calls itself ${h.calls}`, offset(h.offsetMs)].filter(Boolean);
  if (h.state === "live") return { line: what.join(" · ") || "live", fix: null };
  if (h.state === "connecting") return { line: "connecting", fix: null };
  if (h.state === "stale") return { line: `no word since ${time(h.lastHeard ?? now)}`, fix: null };
  const since = h.lastHeard ? `last heard ${ago(h.lastHeard, now)}` : "never reached";
  const retry = h.nextTryAt ? `, trying again in ${Math.max(1, Math.round((h.nextTryAt - now) / 1000))} s` : "";
  return { line: `${h.error?.message ?? h.state} · ${since}${retry}${h.cached ? " · its last snapshot is on the map" : ""}`, fix: h.error?.fix ?? null };
}

async function confirm(question, flags) {
  if (flags.yes) return true;
  if (!process.stdin.isTTY) return false;
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return /^y(es)?$/i.test((await rl.question(`${question} ${c.dim("[y/N]")} `)).trim());
  } finally {
    rl.close();
  }
}

// The hosts list, or null when hosts are off in that XenoNet.
async function hostsOf(s) {
  try {
    return await get(s, "/api/hosts");
  } catch (err) {
    if (err.status === 404) return null;
    throw err;
  }
}
const off = (s) => `Hosts are off in ${nameOf(s)}: start it with XENONET_HOSTS=1 to show other machines' XenoNets (a prototype).`;

export async function run([sub = "list", ...rest], flags) {
  const s = await need(flags);

  if (sub === "self") {
    const [name] = rest;
    const change = name !== undefined || flags.clear === true;
    const self = change ? await post(s, "/api/hosts/self", { id: flags.clear ? null : name }) : await get(s, "/api/hosts/self");
    if (flags.json) return json(self);
    const key = `key ${self.key.slice(0, 8)}…`;
    if (change) return print(self.id ? `${c.green("✓")} ${nameOf(s)} calls itself ${c.bold(self.id)} as a host now ${c.dim(`(${key})`)}` : `${c.green("✓")} ${nameOf(s)} has no host name now ${c.dim(`(${key})`)}`);
    return print(self.id ? `${nameOf(s)} calls itself ${c.bold(self.id)} as a host ${c.dim(`(${key})`)}` : `${nameOf(s)} hasn't named itself as a host ${c.dim(`(${key})`)}. xenonet hosts self <name> names it: a name you choose, such as studio or vm1.`);
  }

  if (sub === "list") {
    const list = await hostsOf(s);
    if (!list) {
      if (flags.json) return json({ on: false, self: await get(s, "/api/hosts/self") });
      print(off(s));
      const self = await get(s, "/api/hosts/self");
      return print(c.dim(self.id ? `As a host, it calls itself ${self.id}.` : "As a host, it hasn't named itself: xenonet hosts self <name>."));
    }
    if (flags.json) return json({ on: true, ...list });
    print(`${c.green("●")} ${c.bold(list.self.id ?? "this XenoNet")} ${c.dim(`· ${nameOf(s)}, here · key ${list.self.key.slice(0, 8)}…`)}`);
    if (!list.hosts.length) return print(c.dim(`  No hosts yet. xenonet hosts add <id> [user@host] adds one.`));
    const rows = [];
    for (const h of list.hosts) {
      const { line, fix } = describe(h);
      rows.push([h.id, c.dim(h.target ?? ""), `${DOT[h.state] ?? "?"} ${h.state}`, line]);
      if (fix) rows.push(["", "", "", c.dim(`↳ ${fix}`)]);
    }
    print(table(rows, { indent: 2, wrap: true }));
    return print(c.dim(`\nXenoNet reaches them with its own ssh config, ${shown(list.sshConfig ?? "ssh_config")}.`));
  }

  if (sub === "add") {
    const [id, target] = rest;
    if (!id) throw new Fail("say which host: xenonet hosts add <id> [user@host]", 2);
    const body = { id, replace: flags.replace === true };
    if (flags.local) Object.assign(body, { transport: "local" });
    else Object.assign(body, { target: target ?? id, sshPort: flags.sshPort, identity: flags.identity, knownHosts: flags.knownHosts, hostKeyAlias: flags.hostKeyAlias, command: flags.command });
    if (flags.xenonetPort !== undefined) body.port = flags.xenonetPort;
    if (flags.since !== undefined) body.since = flags.since;
    if (flags.detail) body.detail = true;
    if (flags.start) body.start = true;
    for (const k of Object.keys(body)) if (body[k] === undefined) delete body[k];
    let h;
    try {
      h = await post(s, "/api/hosts", body);
    } catch (err) {
      if (err.status === 404) throw new Fail(off(s));
      throw err;
    }
    // A moment for the first connection, so what it found can be said.
    for (let i = 0; i < 25 && h.state === "connecting"; i++) {
      await new Promise((r) => setTimeout(r, 200));
      h = (await hostsOf(s))?.hosts.find((x) => x.id === id) ?? h;
    }
    if (flags.json) return json(h);
    const list = await hostsOf(s);
    print(`${c.green("✓")} Added ${c.bold(id)} ${c.dim(`(${h.target})`)}`);
    const { line, fix } = describe(h);
    print(`  ${DOT[h.state] ?? "?"} ${h.state} · ${line}`);
    if (fix) print(c.dim(`  ↳ ${fix}`));
    if (h.transport === "ssh" && list?.sshConfig) print(c.dim(`  XenoNet connects with its own ssh config. The first time, check ${id}'s host key in a terminal: ${trustCommand(id, list.sshConfig, shown)}`));
    return;
  }

  if (sub === "remove") {
    const [id] = rest;
    if (!id) throw new Fail("say which host: xenonet hosts remove <id>", 2);
    const forget = flags.forget === true || (flags.forget !== false && !flags.json && (await confirm(`Also forget where you placed ${id}'s nodes on the map?`, flags)));
    let r;
    try {
      r = await del(s, `/api/hosts/${encodeURIComponent(id)}${forget ? "?forget=1" : ""}`);
    } catch (err) {
      if (err.status === 404 && /XENONET_HOSTS/.test(err.message)) throw new Fail(off(s));
      throw err;
    }
    if (flags.json) return json(r);
    return print(`${c.green("✓")} Removed ${c.bold(id)}${r.forgot ? `, and where you placed ${r.forgot} of its nodes` : ""}. Its last snapshot is gone from the cache.`);
  }

  if (sub === "test") {
    const [id] = rest;
    if (!id) throw new Fail("say which host: xenonet hosts test <id>", 2);
    const t = await post(s, `/api/hosts/${encodeURIComponent(id)}/test`);
    if (flags.json) {
      json(t);
      return t.ok ? 0 : 1;
    }
    if (!t.ok) {
      print(`${c.red("✗")} ${id}: ${t.error?.message ?? `no answer in ${Math.round(t.ms / 1000)} s`}`);
      if (t.error?.fix) print(c.dim(`  ↳ ${t.error.fix}`));
      return 1;
    }
    const h = t.hello ?? {};
    print(`${c.green("✓")} ${id} answered in ${(t.ms / 1000).toFixed(1)} s: XenoNet ${h.version} on ${h.platform}${h.demo ? ", made-up data" : ""}${h.id && h.id !== id ? `, calling itself ${h.id}` : ""}`);
    print(`${c.green("✓")} its relay speaks protocol ${h.protocol}, and sends ${h.detail ? "live detail too" : "titles, states and counts"}${h.sinceMs ? ` for sessions of the last ${Math.round(h.sinceMs / 86400e3) || 1} d` : ""}`);
    print(`${c.green("✓")} a map of ${t.snapshot.nodes} places and sessions, ${t.snapshot.sessions} of them sessions, ${t.snapshot.live} live`);
    const skew = t.offsetMs;
    print(skew == null ? `${c.dim("·")} its clock: no answer yet` : Math.abs(skew) > 120e3 ? `${c.yellow("!")} its clock is ${offset(skew).replace(/^clock /, "")}: have it set its clock by itself` : `${c.green("✓")} its clock: ${offset(skew).replace(/^clock /, "") || "within 2 s of this one's"}`);
    return 0;
  }

  if (sub === "reconnect") {
    const [id] = rest;
    if (!id) throw new Fail("say which host: xenonet hosts reconnect <id>", 2);
    const h = await post(s, `/api/hosts/${encodeURIComponent(id)}/reconnect`);
    return flags.json ? json(h) : print(`${c.green("✓")} Connecting to ${id} now.`);
  }

  throw new Fail(`hosts takes self, add, remove, test or reconnect, not "${sub}". xenonet hosts --help says more.`, 2);
}
