// Hosts as this XenoNet keeps them (docs/design/hosts.md): hosts.json in the
// state folder, a host/list/v1 with this XenoNet's own host id and key and
// the hosts it shows, and XenoNet's own ssh config beside it, written from
// that list. Links run ssh -F with that file, so your ~/.ssh/config is
// never read or changed (the owner's call at merge point A).
//
// hosts.json is a file of its own rather than a part of state.json, so the
// prototype doesn't touch what the core and root lanes share; it can move
// in once hosts graduate. A file XenoNet can't read is moved aside, and one
// from a newer XenoNet is read and never written, as state.json's are (see
// core/blocks/store.js).
import crypto from "node:crypto";
import fs from "node:fs";
import { protectFile } from "../private-file.js";
import path from "node:path";
import { JsonStore } from "../blocks/store.js";
import { systemClock } from "../blocks/clock.js";
import { isHostId, parseSchemaId } from "../schema/ids.js";
import { HOST_KEY, SSH_HOSTNAME, SSH_USER, SSH_FILE, XENONET_COMMAND, SINCE, HostEntry } from "../schema/host.js";
import { check, explain } from "../schema/check.js";

export const HOSTS_FILE = "hosts.json";
export const SSH_CONFIG_FILE = "ssh_config";
export const DEFAULT_PORT = 47840;
export const DEFAULT_SINCE = "7d";

const fail = (message, status = 400) => Object.assign(new Error(message), { status });
const say = (v) => JSON.stringify(String(v));

// "lab@vm1.local" → { user: "lab", hostname: "vm1.local" }. Only what ssh
// takes as it is: nothing that starts with a dash (ssh would read an
// option), no spaces, no second @.
export function parseTarget(target) {
  const m = /^(?:([^@\s]+)@)?([^@\s]+)$/.exec(String(target ?? "").trim());
  if (!m) throw fail(`${say(target)} isn't an ssh target: give [user@]host, such as lab@192.168.5.15 or vm1.local`);
  const [, user = null, hostname] = m;
  if (!SSH_HOSTNAME.test(hostname)) throw fail(`${say(hostname)} isn't a host name or address ssh can take as it is`);
  if (user !== null && !SSH_USER.test(user)) throw fail(`${say(user)} isn't a user name ssh can take as it is`);
  return { user, hostname };
}

const sshFile = (value, what) => {
  if (value == null || value === "") return null;
  if (!SSH_FILE.test(String(value))) throw fail(`${what} has to be a full path (or one under ~/) with no quotes, backslashes or % in it, not ${say(value)}`);
  return String(value);
};

// A host as someone asked for it (xenonet hosts add, POST /api/hosts),
// checked and filled in; an error with a status says what's wrong. ssh
// hosts take [user@]host as `target` (the id itself when there's none),
// or the fields of `ssh`.
export function hostEntry(input = {}, { now = Date.now() } = {}) {
  const { id } = input;
  if (!isHostId(id)) throw fail(`a host's id is a name you choose: 2 to 32 lowercase letters, digits and dashes, starting with a letter, such as vm1 or studio${id == null || id === "" ? "" : `, not ${say(id)}`}`);
  const transport = input.transport ?? (input.local ? "local" : "ssh");
  if (transport !== "ssh" && transport !== "local") throw fail(`a host is reached over ssh, or locally (another XenoNet on this machine), not ${say(transport)}`);
  const port = input.port == null || input.port === "" ? DEFAULT_PORT : Number(input.port);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw fail(`the port of the XenoNet to read there is a number from 1 to 65535, not ${say(input.port)}`);
  const since = String(input.since ?? DEFAULT_SINCE);
  if (!SINCE.test(since)) throw fail(`how far back sessions go is a number and m, h or d, such as 7d, 12h or 90m, not ${say(since)}`);
  let ssh = null;
  let command = null;
  if (transport === "ssh") {
    const given = input.ssh ?? {};
    const target = parseTarget(given.hostname ? `${given.user ? `${given.user}@` : ""}${given.hostname}` : (input.target ?? id));
    const sshPort = given.port ?? input.sshPort;
    if (sshPort != null && sshPort !== "" && !(Number.isInteger(Number(sshPort)) && Number(sshPort) >= 1 && Number(sshPort) <= 65535)) throw fail(`ssh's port is a number from 1 to 65535, not ${say(sshPort)}`);
    const alias = given.hostKeyAlias ?? input.hostKeyAlias ?? null;
    if (alias != null && alias !== "" && !SSH_HOSTNAME.test(String(alias))) throw fail(`${say(alias)} isn't a name ssh can keep a host key under`);
    ssh = {
      hostname: target.hostname,
      user: target.user,
      port: sshPort == null || sshPort === "" ? null : Number(sshPort),
      identity: sshFile(given.identity ?? input.identity, "the key file"),
      knownHosts: sshFile(given.knownHosts ?? input.knownHosts, "the known_hosts file"),
      hostKeyAlias: alias || null,
    };
    command = String(input.command ?? "xenonet");
    if (!XENONET_COMMAND.test(command)) throw fail(`the xenonet command on the host is xenonet, or its full path with no spaces or shell characters in it, not ${say(command)}`);
  }
  const entry = { id, transport, ssh, port, command, since, detail: input.detail === true, ...(input.start === true ? { start: true } : {}), key: null, name: null, addedAt: now };
  // The schema again, so nothing written here can be what it refuses.
  const checked = check(HostEntry, entry);
  if (!checked.ok) throw fail(`that host can't be kept: ${explain(checked)}`);
  return entry;
}

// XenoNet's own ssh config, one Host block per ssh host, named by its id.
// Every value has been through hostEntry, so none can break a line, and
// files go in quotes since they may hold spaces.
export function sshConfigText(hosts) {
  const lines = [
    "# XenoNet's own ssh config, for the hosts added with xenonet hosts add.",
    "# XenoNet writes it from hosts.json and runs ssh -F with it, so your",
    "# ~/.ssh/config is neither read nor changed. Edits here are written over.",
  ];
  for (const h of hosts) {
    if (h.transport !== "ssh" || !h.ssh || !check(HostEntry, h).ok) continue;
    lines.push("", `Host ${h.id}`, `  HostName ${h.ssh.hostname}`);
    if (h.ssh.user) lines.push(`  User ${h.ssh.user}`);
    if (h.ssh.port) lines.push(`  Port ${h.ssh.port}`);
    if (h.ssh.identity) lines.push(`  IdentityFile "${h.ssh.identity}"`, "  IdentitiesOnly yes");
    if (h.ssh.knownHosts) lines.push(`  UserKnownHostsFile "${h.ssh.knownHosts}"`);
    if (h.ssh.hostKeyAlias) lines.push(`  HostKeyAlias ${h.ssh.hostKeyAlias}`);
  }
  return `${lines.join("\n")}\n`;
}

// hosts.json says what it is in its schema field; one without it is taken
// as the first version.
function versionOf(doc) {
  if (doc.schema === undefined) return 1;
  const parsed = parseSchemaId(doc.schema);
  return parsed?.path === "host/list" ? parsed.major : null;
}

export class HostsConfig {
  // `stateDir`: the folder, or a function giving it.
  constructor({ stateDir, clock = systemClock, newKey = () => crypto.randomBytes(16).toString("hex") }) {
    this.stateDir = typeof stateDir === "function" ? stateDir : () => stateDir;
    this.clock = clock;
    this.newKey = newKey;
    // Entries a hand edit left that XenoNet can't use: dropped as it reads
    // the file, and said here.
    this.dropped = [];
    this.store = new JsonStore({
      name: "hosts",
      file: () => this.file,
      version: 1,
      versionOf,
      fresh: () => ({ schema: "host/list/v1", self: null, hosts: [] }),
      prepare: (doc) => this.#prepare(doc),
      debounceMs: 50,
      what: "its hosts",
      clock,
    });
  }

  get file() {
    return path.join(this.stateDir(), HOSTS_FILE);
  }

  get sshConfig() {
    return path.join(this.stateDir(), SSH_CONFIG_FILE);
  }

  #prepare(doc) {
    doc.schema = "host/list/v1";
    const self = doc.self;
    doc.self = self && typeof self === "object" && HOST_KEY.test(self.key ?? "") ? { id: isHostId(self.id) ? self.id : null, key: self.key, createdAt: Number.isFinite(self.createdAt) ? self.createdAt : this.clock.now() } : null;
    const seen = new Set();
    this.dropped = [];
    doc.hosts = (Array.isArray(doc.hosts) ? doc.hosts : []).filter((h) => {
      const ok = check(HostEntry, h).ok && !seen.has(h.id);
      if (ok) seen.add(h.id);
      else this.dropped.push(isHostId(h?.id) ? h.id : "(no id)");
      return ok;
    });
  }

  #doc() {
    return this.store.load();
  }

  #save() {
    if (this.store.loadReport().readOnly) throw fail(`XenoNet can't change ${this.file}: ${this.store.loadReport().problem}`, 409);
    this.store.save();
    this.store.flush();
  }

  // This XenoNet as a host: the name it gave itself and its key. The key is
  // made the first time anything asks, once.
  self() {
    const doc = this.#doc();
    if (!doc.self) {
      doc.self = { id: null, key: this.newKey(), createdAt: this.clock.now() };
      try {
        this.#save();
      } catch {} // A read-only file: the key lasts until XenoNet quits.
    }
    return { ...doc.self };
  }

  // Names this XenoNet as a host (null takes the name away). Never the
  // machine's own name: it's a name you choose.
  setSelf(id) {
    if (id !== null && !isHostId(id)) throw fail(`a host's id is a name you choose: 2 to 32 lowercase letters, digits and dashes, starting with a letter, such as studio or vm1${id ? `, not ${say(id)}` : ""}`);
    this.self();
    this.#doc().self.id = id;
    this.#save();
    return this.self();
  }

  list() {
    return this.#doc().hosts.map((h) => structuredClone(h));
  }

  get(id) {
    const h = this.#doc().hosts.find((x) => x.id === id);
    return h ? structuredClone(h) : null;
  }

  // Adds a host (see hostEntry). An id that's taken is refused unless
  // `replace`, which also forgets the key it learned: adding a host again
  // is how a changed key is accepted.
  add(input, { replace = false } = {}) {
    const entry = hostEntry(input, { now: this.clock.now() });
    const doc = this.#doc();
    const at = doc.hosts.findIndex((h) => h.id === entry.id);
    if (at >= 0 && !replace) throw fail(`there's already a host called ${entry.id}: remove it first, or replace it (xenonet hosts add ${entry.id} … --replace)`, 409);
    if (at >= 0) doc.hosts[at] = entry;
    else doc.hosts.push(entry);
    this.#save();
    this.writeSshConfig();
    return structuredClone(entry);
  }

  remove(id) {
    const doc = this.#doc();
    const at = doc.hosts.findIndex((h) => h.id === id);
    if (at < 0) return null;
    const [gone] = doc.hosts.splice(at, 1);
    this.#save();
    this.writeSshConfig();
    return gone;
  }

  // What a host said about itself the first time it answered (or again,
  // when a demo world was made afresh): its key, and its own name when
  // that isn't the id here.
  learn(id, { key, name } = {}) {
    const h = this.#doc().hosts.find((x) => x.id === id);
    if (!h) return null;
    if (key !== undefined && (key === null || HOST_KEY.test(key))) h.key = key;
    if (name !== undefined) h.name = isHostId(name) && name !== id ? name : null;
    this.#save();
    return structuredClone(h);
  }

  // Writes XenoNet's ssh config from the list, readable by you alone (ssh
  // refuses a config others can write).
  writeSshConfig() {
    const file = this.sshConfig;
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const tmp = `${file}.tmp`;
    fs.writeFileSync(tmp, sshConfigText(this.#doc().hosts), { mode: 0o600 });
    protectFile(tmp);
    fs.renameSync(tmp, file);
    return file;
  }
}
