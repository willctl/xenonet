// core/hosts/config.js in Rust (crates/xenonet-hosts/src/config.rs): hosts
// as this XenoNet keeps them, in hosts.json, and its own ssh config written
// from them. What an entry may hold, the ssh config's text and what's kept
// of a hand-edited file are the Rust's. The schema (zod's) checks an entry
// through a callback, and the store, the files and the key are here.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { settle } from "../../settle.js";

const { protectFile } = await import(peer("private-file.js"));
const { JsonStore } = await import(peer("blocks/store.js"));
const { systemClock } = await import(peer("blocks/clock.js"));
const { HostEntry } = await import(peer("schema/host.js"));
const { check, explain } = await import(peer("schema/check.js"));

const C = native.hostsConfigConstants();

export const HOSTS_FILE = C.hostsFile;
export const SSH_CONFIG_FILE = C.sshConfigFile;
export const DEFAULT_PORT = C.defaultPort;
export const DEFAULT_SINCE = C.defaultSince;

// The schema's word on an entry: null, or what's wrong with it.
const checkEntry = (entry) => {
  const checked = check(HostEntry, entry);
  return checked.ok ? null : explain(checked);
};

export const parseTarget = (target) => settle(native.hostsParseTarget(target));

export const hostEntry = (input = {}, { now = Date.now() } = {}) => settle(native.hostsEntry(input, Number(now), checkEntry));

export const sshConfigText = (hosts) => native.hostsSshConfigText(hosts, checkEntry);

const versionOf = (doc) => native.hostsVersionOf(doc) ?? null;

export class HostsConfig {
  // `stateDir`: the folder, or a function giving it.
  constructor({ stateDir, clock = systemClock, newKey = () => crypto.randomBytes(16).toString("hex") }) {
    this.stateDir = typeof stateDir === "function" ? stateDir : () => stateDir;
    this.clock = clock;
    this.newKey = newKey;
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
    const kept = native.hostsPrepare(doc, this.clock.now(), checkEntry);
    doc.schema = "host/list/v1";
    doc.self = kept.self;
    doc.hosts = kept.hosts;
    this.dropped = kept.dropped;
  }

  #doc() {
    return this.store.load();
  }

  #save() {
    if (this.store.loadReport().readOnly) throw Object.assign(new Error(`XenoNet can't change ${this.file}: ${this.store.loadReport().problem}`), { status: 409 });
    this.store.save();
    this.store.flush();
  }

  // This XenoNet as a host: the name it gave itself and its key, made the
  // first time anything asks.
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

  setSelf(id) {
    settle(native.hostsCheckSelfId(id));
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

  add(input, { replace = false } = {}) {
    const entry = hostEntry(input, { now: this.clock.now() });
    const doc = this.#doc();
    doc.hosts = settle(native.hostsAdd(doc.hosts, entry, replace === true || (replace !== false && Boolean(replace))));
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

  learn(id, { key, name } = {}) {
    const doc = this.#doc();
    const at = doc.hosts.findIndex((x) => x.id === id);
    if (at < 0) return null;
    doc.hosts[at] = native.hostsLearn(doc.hosts[at], key, name);
    this.#save();
    return structuredClone(doc.hosts[at]);
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
