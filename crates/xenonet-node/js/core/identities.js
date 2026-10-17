// core/identities.js in Rust (crates/xenonet-state/src/identities.rs): the
// accounts work happens under. Rust reads what each app's files say (the
// Claude and ChatGPT sign-ins, gh's hosts.yml, ssh aliases, git config's
// output), keeps the sign-in timeline, links sessions and projects to the
// accounts on the map, finds each repo's config file and your git config's
// files, and reads the account a Remote Control session's transcript
// names. The lenient checks of the apps' files (vendor.js's readers),
// running git config, and the watch's gatherers and their reads of the
// sign-in files stay here. Nothing here leaves the machine.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { gitAvailable } from "./git.js";
import { mapLimit } from "./projects.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, parseJson } from "./vendor.js";
import { native } from "../index.js";

// What each app's file says is read leniently against its schema: what
// differs is kept as drift, as shapes, never values.
const claudeConfigs = reader("anthropic/claude-code/config");
const codexClaims = reader("openai/codex/auth-claims");
const ghHosts = reader("github/gh/hosts");

// A Map's (or any list of pairs') entries, for Rust.
const pairs = (m) => (m == null ? [] : [...m]);

// ---------- what each app says ----------

// The account Claude Code is signed in to, from ~/.claude.json.
export const claudeAccount = (data) => native.idClaudeAccount(data);

// Codex's sign-in, from ~/.codex/auth.json: only the ID token's claims
// are read, and only these.
export function chatgptAccount(auth) {
  let claims = native.idTokenClaims(auth);
  if (claims != null) claims = codexClaims.check(claims);
  return native.idChatgptAccount(auth, claims);
}

// gh's hosts.yml: each host's signed-in users and the active one, never a token.
export const parseGhHosts = (text) => native.idParseGhHosts(String(text ?? ""));

// hosts.yml as JSON, keys only, for its schema (vendor/github/gh/hosts).
export const ghHostsShape = (text) => native.idGhHostsShape(String(text ?? ""));

// SSH host aliases (`Host gh-work` with `HostName github.com`).
export const parseSshAliases = (text) => new Map(native.idParseSshAliases(String(text ?? "")));

// A remote URL's host, owner and repo, with SSH aliases resolved.
export const parseRemote = (url, aliases = new Map()) => native.idParseRemote(String(url ?? ""), pairs(aliases));

// `git config --get-regexp` output: the identity commits get, and the
// credential settings that say who a push signs in as.
export const parseGitConfig = (out) => native.idParseGitConfig(String(out ?? ""));

// Who a push to `url` signs in as.
export const pushAccount = (url, cfg, aliases) => native.idPushAccount(url, cfg, pairs(aliases));

// ---------- the sign-in timeline ----------

// What Rust says noting a sign-in does, done to the timeline itself.
function applySignin(spans, op) {
  if (op.extend != null) {
    spans[spans.length - 1].to = op.extend;
    return true;
  }
  if (!op.span) return false;
  spans.push(op.span);
  if (op.drop) spans.splice(0, op.drop);
  return true;
}

// Records that `id` is signed in at `now`. Returns whether the timeline changed.
export const noteSignin = (spans, id, now, since = null) => applySignin(spans, native.idNoteSignin(spans, id, now, since));

// Which account an app was signed in to at `t`.
export const accountAt = (spans, current, t, known = new Set()) => native.idAccountAt(spans, current, t, [...known]);

// ---------- linking ----------

// The saved state this works from (see store.js).
export const emptySaved = () => native.idEmptySaved();

export const githubId = (host, login) => native.idGithubId(`${host}`, String(login));
export const gitId = (email) => native.idGitId(String(email));
export const claudeId = (account, org) => native.idClaudeId(`${account}`, `${org}`);

// A link's end: a node, once the page's layout has put it there, or its id.
const endOf = (x) => (typeof x === "object" ? x.id : x);

// Puts identity links on the graph: `ranAs` on each session, `identities`
// on projects, and `graph.identities`. Mutates `saved.known` and
// `saved.signins`, and says whether they changed.
export function linkIdentities(graph, { accounts = {}, repos = new Map(), owners = new Map(), saved = emptySaved(), home = null, now = Date.now() } = {}) {
  const nodes = graph.nodes;
  const r = native.idLinkIdentities({
    nodes: nodes.map((n) => ({ id: n.id, kind: n.kind, app: n.app, claudeAccount: n.claudeAccount, provider: n.provider, updatedAt: n.updatedAt, createdAt: n.createdAt, origin: n.git?.origin, key: n.key, group: n.group, name: n.name })),
    links: graph.links.map((l) => [endOf(l.source), endOf(l.target), l.kind]),
    accounts: { claude: accounts.claude, chatgpt: accounts.chatgpt, gh: accounts.gh, claudeFile: accounts.claudeFile, chatgptFile: accounts.chatgptFile },
    ssh: pairs(accounts.ssh),
    repos: pairs(repos),
    owners: pairs(owners),
    saved,
    home,
    now,
  });
  for (const [i, link] of r.ranAs) nodes[i].ranAs = link;
  for (const [i, links] of r.links) nodes[i].identities = links;
  const signins = (saved.signins ??= {});
  for (const [app, op] of r.signins) applySignin((signins[app] ??= []), op);
  const known = (saved.known ??= {});
  for (const [id, whole, value] of r.known) {
    if (whole) known[id] = value;
    else Object.assign(known[id], value);
  }
  graph.identities = r.identities;
  return { identities: graph.identities, dirty: r.dirty };
}

// ---------- reading it from disk ----------

const mtimeOf = (file) => {
  try {
    return fs.statSync(file).mtimeMs;
  } catch {
    return null;
  }
};

function gitConfig(dir, env) {
  return new Promise((resolve) => {
    execFile("git", ["-C", dir, "config", "--get-regexp", "^(user\\.(name|email)|credential\\..*)$"], { env, timeout: 8000, windowsHide: true, maxBuffer: 1 << 20 }, (err, stdout) => {
      // Exit status 1 just means nothing matched.
      resolve(err && err.code !== 1 ? null : parseGitConfig(stdout));
    });
  });
}

// Reads the identity sources and keeps what they said until they change.
// Each source is a gatherer read again only when its stamp changes: an
// account file by its mtime, and a repo's git identity by its config's
// mtime, your git config files' and its remote. The watch is a block
// itself, "identities", degraded while a source can't be read.
export class IdentityWatch extends Block {
  #owners = null;

  constructor({ home, claudeConfigFile, codexHome, ghConfigDir, claudeProjectsDir, clock }) {
    super({ name: "identities", kind: "gatherer", clock });
    this.home = home;
    this.claudeConfigFile = claudeConfigFile;
    this.codexHome = codexHome;
    this.ghConfigDir = ghConfigDir;
    this.claudeProjectsDir = claudeProjectsDir;
    // Away from your own home folder (the demo, tests), git reads that
    // folder's config rather than yours.
    this.gitEnv = native.idGitEnv(process.env, String(home), os.homedir()) ?? process.env;
    this.forget();
  }

  // "Detect identities again": nothing cached.
  forget() {
    this.files = new Map(); // file → its source
    this.repoSources = new Map(); // project key → { p, pass, source }
    this.#owners = new native.TranscriptOwners(String(this.claudeProjectsDir ?? ""));
  }

  // What `file` says, parsed again only when its mtime changes. A file
  // that isn't there says nothing, and so does one that can't be read or
  // parsed, until it's written again.
  file(file, parse) {
    let source = this.files.get(file);
    if (!source) {
      source = new Gatherer({
        name: `identities:${path.basename(file)}`,
        stamp: () => mtimeOf(file),
        gather: () => {
          let text;
          try {
            text = fs.readFileSync(file, "utf8");
          } catch (err) {
            if (err.code === "ENOENT") return null;
            throw err;
          }
          return parse(text);
        },
        clock: this.clock,
      });
      this.files.set(file, source);
    }
    const value = source.get();
    this.#report();
    return value;
  }

  // A sign-in file that isn't JSON says where it breaks, never its text,
  // which can hold a token (parseJson).
  accounts() {
    const chatgptFile = path.join(this.codexHome, "auth.json");
    return {
      claude: this.file(this.claudeConfigFile, (t) => claudeAccount(claudeConfigs.check(parseJson(t)))),
      chatgpt: this.file(chatgptFile, (t) => chatgptAccount(parseJson(t))),
      gh: this.file(path.join(this.ghConfigDir, "hosts.yml"), (t) => (ghHosts.check(ghHostsShape(t)), parseGhHosts(t))) ?? [],
      ssh: this.file(path.join(this.home, ".ssh", "config"), parseSshAliases) ?? new Map(),
      claudeFile: this.claudeConfigFile,
      chatgptFile,
    };
  }

  // Your git config's files, so a change to any of them (an includeIf
  // target included) reads every repo's identity again.
  globalStamp() {
    return native.idGlobalStamp(String(this.home), this.gitEnv.XDG_CONFIG_HOME ?? null);
  }

  // Each repo's git identity, as the last read of it found it.
  get repos() {
    return new Map([...this.repoSources].filter(([, r]) => r.source.good).map(([key, r]) => [key, { stamp: r.source.last?.stamp, value: r.source.value }]));
  }

  // A repo's source: its git identity, read again when its config or yours
  // changes, and tried again on the next pass when git couldn't read it.
  #repo(p, pass) {
    let r = this.repoSources.get(p.key);
    if (!r) {
      r = {};
      r.source = new Gatherer({
        name: "identities:repo",
        stamp: () => `${r.pass.global}|${native.idRepoConfigMtime(String(r.p.path))}|${r.p.git.origin ?? ""}`,
        gather: async () => {
          const cfg = await gitConfig(r.p.path, this.gitEnv);
          if (!cfg) throw new Error(native.idUnreadableConfig(String(r.p.path), String(this.home ?? "")));
          return { name: cfg.name, email: cfg.email, push: pushAccount(r.p.git.origin, cfg, r.pass.ssh) };
        },
        retry: true,
        keep: true,
        clock: this.clock,
      });
      this.repoSources.set(p.key, r);
    }
    Object.assign(r, { p, pass });
    return r;
  }

  // Each repo's git identity, read again when its config or yours changes.
  // Returns whether anything changed.
  async refreshRepos(projects) {
    if (!gitAvailable()) return false;
    const pass = { global: this.globalStamp(), ssh: this.accounts().ssh };
    let changed = false;
    const repos = projects.filter((p) => p.git && p.role !== "group");
    await mapLimit(repos, 4, async (p) => {
      const { source } = this.#repo(p, pass);
      if (!source.due()) return;
      const before = source.good ? JSON.stringify(source.value) : undefined;
      const value = await source.run();
      if (source.last.ok && JSON.stringify(value) !== before) changed = true;
    });
    for (const key of this.repoSources.keys()) if (!repos.some((p) => p.key === key)) this.repoSources.delete(key);
    this.#report();
    return changed;
  }

  // Degraded while a source couldn't be read the last time it was, and
  // running again once they all can.
  #report() {
    const file = [...this.files].find(([, s]) => s.last && !s.last.ok);
    const repo = [...this.repoSources.values()].find((r) => r.source.last && !r.source.last.ok);
    const fileError = file ? [String(file[0]), String(file[1].last.error)] : null;
    const repoError = repo?.source.last.error;
    const { say, problem } = native.idReport(fileError, repoError == null ? null : String(repoError), String(this.home ?? ""), this.state, this.lastError?.message ?? null);
    if (say === "ok") this.ok();
    else if (say === "problem") this.problem(problem);
  }

  health() {
    const h = super.health();
    const runs = (sources) => sources.reduce((n, s) => n + (s.counters.runs ?? 0), 0);
    h.counters = { ...h.counters, files: this.files.size, fileReads: runs([...this.files.values()]), repos: this.repoSources.size, repoReads: runs([...this.repoSources.values()].map((r) => r.source)) };
    return h;
  }

  // The account a Remote Control session names in its transcript's first
  // lines. Read once per session, again only while the file is still short
  // enough for the entry to turn up.
  transcriptOwner(id) {
    return this.#owners.owner(String(id), Date.now());
  }

  // Links the graph's sessions and projects to identities (see
  // linkIdentities).
  link(graph, saved, now = Date.now()) {
    const owners = new Map();
    for (const n of graph.nodes) if (n.kind === "session" && n.app === "claude" && !n.claudeAccount && !saved.pins?.[n.id]) owners.set(n.id, this.transcriptOwner(n.id));
    const repos = new Map([...this.repos].map(([k, v]) => [k, v.value]));
    return linkIdentities(graph, { accounts: this.accounts(), repos, owners, saved, home: this.home, now });
  }
}
