// Identities: the accounts work happens under. A Claude account (in the
// organization it ran in), a ChatGPT account (Codex's sign-in), GitHub
// accounts, and the name and email git commits with. A session links to the
// account it ran as; a project to the GitHub account that owns its remote,
// the one a push authenticates as, and the identity its commits carry. A
// session takes its project's links too.
//
// Like project detection, it's all read from what's already on disk, re-read
// only when a file changes, and cached where it costs anything:
//
//   Claude    ~/.claude.json's oauthAccount: the account, its organization and
//             email. Never the name fields. Claude desktop files each session
//             under claude-code-sessions/<account>/<organization>/, and Remote
//             Control sessions name their owner in the transcript, so those
//             links are exact.
//   ChatGPT   ~/.codex/auth.json: the claims in its ID token (email,
//             workspace, plan, when you signed in), decoded without verifying.
//             The tokens are never kept. Codex records no account on threads.
//   GitHub    gh's hosts.yml: the accounts gh is signed in to on each host,
//             and the active one. Never its tokens.
//   git       `git config` in each repo, so includeIf rules count: the name
//             and email commits get, and who a push over HTTPS signs in as.
//
// Where nothing records the account (Codex threads, Claude Code sessions run
// outside the desktop app), a session gets whoever was signed in when it last
// ran. XenoNet keeps a timeline of the sign-ins it has seen (see noteSignin), so
// a session keeps its account after you switch.
//
// Nothing here leaves the machine, and nothing about identities goes into a
// prompt for Codex.
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { gitAvailable } from "./git.js";
import { mapLimit } from "./projects.js";
import { tilde } from "./paths.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, parseJson } from "./vendor.js";

// What each app's file says is read leniently against its schema
// (vendor.js): what differs is kept as drift, as shapes, never values.
const claudeConfigs = reader("anthropic/claude-code/config");
const codexClaims = reader("openai/codex/auth-claims");
const ghHosts = reader("github/gh/hosts");

// How sure a link is, strongest first: "yours" (you said so), "exact" (the
// app recorded it), "likely" (the account was signed in at the time) or
// "assumed" (it's the only one XenoNet has seen).

const CLAUDE_PLAN = { claude_max: "Max", claude_pro: "Pro", claude_team: "Team", claude_enterprise: "Enterprise" };
const HEAD_BYTES = 64 * 1024;
// A sign-in's span is extended this often while it lasts, so state.json
// isn't rewritten on every refresh.
const SIGNIN_STEP_MS = 10 * 60e3;
const MAX_SPANS = 50;

const emailKey = (email) => crypto.createHash("sha256").update(String(email).trim().toLowerCase()).digest("hex").slice(0, 16);
const str = (v) => (typeof v === "string" && v.trim() ? v.trim() : null);

// ---------- what each app says ----------

// The account Claude Code is signed in to, from ~/.claude.json.
export function claudeAccount(data) {
  const a = data?.oauthAccount;
  if (!str(a?.accountUuid) || !str(a?.organizationUuid)) return null;
  const team = /team|enterprise/i.test(a.organizationType ?? "");
  return {
    id: `claude:${a.accountUuid}:${a.organizationUuid}`,
    kind: "claude",
    handle: str(a.emailAddress),
    // A personal organization is named after the email, so only a team's name says anything.
    detail: { plan: CLAUDE_PLAN[a.organizationType] ?? null, org: team ? str(a.organizationName) : null },
  };
}

// Codex's sign-in, from ~/.codex/auth.json. Only the ID token's claims are
// read, and only these; an API key sign-in says nothing about who.
export function chatgptAccount(auth) {
  if (!auth || typeof auth !== "object") return null;
  let claims = null;
  try {
    claims = JSON.parse(Buffer.from(String(auth.tokens?.id_token).split(".")[1], "base64url").toString("utf8"));
  } catch {}
  if (claims != null) claims = codexClaims.check(claims);
  const oa = claims?.["https://api.openai.com/auth"] ?? {};
  const workspace = str(oa.chatgpt_account_id) ?? str(auth.tokens?.account_id);
  if (workspace && (auth.auth_mode ?? "chatgpt") === "chatgpt") {
    return { id: `chatgpt:${workspace}`, kind: "chatgpt", handle: str(claims?.email), detail: { plan: str(oa.chatgpt_plan_type) }, since: Number(claims?.auth_time) * 1000 || null };
  }
  if (auth.OPENAI_API_KEY || auth.auth_mode === "apikey") return { id: "openai:api-key", kind: "openai", handle: null, detail: { plan: null } };
  return null;
}

// gh's hosts.yml: each host's signed-in users and the active one. A line-by-
// line read of the little YAML gh writes; values other than user names are
// never read, so a token stored in the file stays where it is.
export function parseGhHosts(text) {
  const hosts = [];
  let host = null;
  let usersAt = -1; // indent of the `users:` key while inside it
  let userAt = -1; // indent of the user names under it
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    if (!raw.trim() || raw.trim().startsWith("#")) continue;
    const indent = raw.length - raw.trimStart().length;
    const m = /^\s*([^:#\s][^:]*?)\s*:(?:\s+(.*?))?\s*$/.exec(raw);
    if (!m) continue;
    const [, key, value = ""] = m;
    const unquote = (v) => v.replace(/^(['"])(.*)\1$/, "$2");
    if (indent === 0) {
      host = { host: unquote(key).toLowerCase(), users: [], active: null };
      hosts.push(host);
      usersAt = userAt = -1;
      continue;
    }
    if (!host) continue;
    if (usersAt >= 0 && indent > usersAt) {
      if (userAt < 0) userAt = indent;
      if (indent === userAt) host.users.push(unquote(key));
      continue;
    }
    usersAt = userAt = -1;
    if (key === "users") usersAt = indent;
    else if (key === "user" && value) host.active = unquote(value);
  }
  for (const h of hosts) if (h.active && !h.users.some((u) => u.toLowerCase() === h.active.toLowerCase())) h.users.push(h.active);
  return hosts.filter((h) => h.users.length);
}

// hosts.yml as JSON, keys only, for its schema (vendor/github/gh/hosts): a
// key with a value has "" for it, one with keys under it an object, one with
// a list under it an array, one with nothing null. No value is kept.
export function ghHostsShape(text) {
  const root = {};
  const stack = [{ indent: -1, obj: root }];
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const indent = raw.length - raw.trimStart().length;
    while (stack.length > 1 && indent <= stack.at(-1).indent) stack.pop();
    const top = stack.at(-1);
    if (line.startsWith("-")) {
      if (top.obj == null) top.parent[top.key] = top.obj = [];
      continue;
    }
    const m = /^([^:#\s][^:]*?)\s*:(?:\s+(.*?))?\s*$/.exec(line);
    if (!m || Array.isArray(top.obj)) continue;
    if (top.obj == null) top.parent[top.key] = top.obj = {};
    const key = m[1].replace(/^(['"])(.*)\1$/, "$2");
    top.obj[key] = m[2] ? "" : null;
    if (!m[2]) stack.push({ indent, parent: top.obj, key, obj: null });
  }
  return root;
}

// SSH host aliases (`Host gh-work` with `HostName github.com`), so a remote
// like git@gh-work:me/app counts as GitHub.
export function parseSshAliases(text) {
  const aliases = new Map();
  let names = [];
  for (const raw of String(text ?? "").split(/\r?\n/)) {
    const m = /^\s*(\w+)\s*(?:=\s*|\s+)(.+?)\s*$/.exec(raw);
    if (!m) continue;
    const key = m[1].toLowerCase();
    if (key === "host") names = m[2].split(/\s+/).filter((n) => !/[*?!]/.test(n)).map((n) => n.toLowerCase());
    else if (key === "match") names = [];
    else if (key === "hostname") for (const n of names) if (!aliases.has(n)) aliases.set(n, m[2].toLowerCase());
  }
  return aliases;
}

// A remote URL's host, owner and repo, with SSH aliases resolved. `user` is
// a user name written into an HTTPS URL; anything after it (a token) is
// dropped.
export function parseRemote(url, aliases = new Map()) {
  const s = String(url ?? "").trim();
  let m = /^([a-z][a-z+.-]*):\/\/(?:([^@/:]+)(?::[^@/]*)?@)?([^/:]+)(?::\d+)?\/(.+)$/i.exec(s);
  let scheme, user = null, host, rest;
  if (m) [, scheme, user, host, rest] = m;
  else if ((m = /^(?:[^@/]+@)?([^:/\\]{2,}):(?!\/\/)(.+)$/.exec(s))) [scheme, host, rest] = ["ssh", m[1], m[2]];
  else return null;
  scheme = scheme.toLowerCase();
  const [owner, repo] = rest.replace(/^\/+/, "").replace(/\.git\/?$/, "").split("/");
  if (!owner || !repo) return null;
  return { host: aliases.get(host.toLowerCase()) ?? host.toLowerCase(), owner, repo, https: scheme === "https" || scheme === "http", user: (scheme.startsWith("http") && user) || null };
}

// `git config --get-regexp` output: the identity commits get, and the
// credential settings that say who a push signs in as. Helper commands are
// kept only long enough to see whether gh is one of them.
export function parseGitConfig(out) {
  const cfg = { name: null, email: null, helpers: [], usernames: [] };
  for (const line of String(out ?? "").split(/\r?\n/)) {
    if (!line) continue;
    const sp = line.indexOf(" ");
    const key = sp < 0 ? line : line.slice(0, sp);
    const value = sp < 0 ? "" : line.slice(sp + 1).trim();
    const lower = key.toLowerCase();
    if (lower === "user.name") cfg.name = value || null;
    else if (lower === "user.email") cfg.email = value || null;
    else if (lower.startsWith("credential.")) {
      const v = lower.slice(lower.lastIndexOf(".") + 1);
      const scope = key.slice("credential.".length, key.length - v.length - 1) || null;
      if (v === "helper") cfg.helpers.push({ scope, value });
      else if (v === "username" && value) cfg.usernames.push({ scope, user: value });
    }
  }
  return cfg;
}

// Does a credential section's URL (https://github.com, or a path under it)
// cover this remote?
function covers(scope, r) {
  const m = /^(https?):\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?(\/.*)?$/i.exec(scope);
  if (!m || m[2].toLowerCase() !== r.host) return false;
  const prefix = (m[3] ?? "").replace(/\/+$/, "").toLowerCase();
  return !prefix || `/${r.owner}/${r.repo}`.toLowerCase().startsWith(prefix);
}

// Who a push to `url` signs in as: a user name in the URL or in git's
// credential settings, or, with gh as the credential helper, gh's active
// account for the host. SSH pushes use a key, which names no account.
export function pushAccount(url, cfg, aliases) {
  const r = parseRemote(url, aliases);
  if (!r?.https) return null;
  if (r.user) return { host: r.host, user: r.user, how: "the remote URL" };
  const named = cfg.usernames.filter((u) => u.scope && covers(u.scope, r)).pop() ?? cfg.usernames.filter((u) => !u.scope).pop();
  if (named) return { host: r.host, user: named.user, how: "git config" };
  let helpers = [];
  for (const h of cfg.helpers) if (!h.scope || covers(h.scope, r)) helpers = h.value ? [...helpers, h.value] : [];
  if (helpers.some((h) => /\bgh(\.exe)?["']?\s+auth\s+git-credential\b/.test(h))) return { host: r.host, user: null, how: "gh" };
  return null;
}

// ---------- the sign-in timeline ----------

// Records that `id` is signed in at `now`. `since` is when the app says
// the sign-in happened, if it says. Returns whether the timeline changed.
export function noteSignin(spans, id, now, since = null) {
  const last = spans[spans.length - 1];
  if (last?.id === id) {
    if (now - last.to < SIGNIN_STEP_MS) return false;
    last.to = now;
    return true;
  }
  spans.push({ id, from: Math.max(since ?? now, last?.to ?? -Infinity), to: now });
  if (spans.length > MAX_SPANS) spans.splice(0, spans.length - MAX_SPANS);
  return true;
}

// Which account an app was signed in to at `t`: the current one from when
// it signed in, an earlier span that covers t, or, failing those, the only
// account XenoNet knows of for the app.
export function accountAt(spans, current, t, known = new Set()) {
  if (t != null) {
    const last = spans[spans.length - 1];
    if (current && t >= Math.min(current.since ?? Infinity, last?.id === current.id ? last.from : Infinity)) return { id: current.id, confidence: "likely", via: "signed in when it ran" };
    for (let i = spans.length - 1; i >= 0; i--) {
      const s = spans[i];
      // A gap while XenoNet wasn't running belongs to the account on both sides of it.
      const next = spans[i + 1];
      if (t >= s.from && (t <= s.to || (next && next.id === s.id && t < next.from))) return { id: s.id, confidence: "likely", via: "signed in when it ran" };
    }
  }
  const ids = new Set([...known, ...spans.map((s) => s.id), ...(current ? [current.id] : [])]);
  return ids.size === 1 ? { id: [...ids][0], confidence: "assumed", via: "the only account XenoNet has seen" } : null;
}

// ---------- linking ----------

// The saved state this works from (see store.js): your names, hidden
// identities and pins, plus what XenoNet has seen before.
export const emptySaved = () => ({ names: {}, hidden: {}, pins: {}, known: {}, signins: {} });

export const githubId = (host, login) => `github:${host}/${String(login).toLowerCase()}`;
export const gitId = (email) => `git:${emailKey(email)}`;
export const claudeId = (account, org) => `claude:${account}:${org}`;

// Puts identity links on the graph: `ranAs` on each session, `identities` on
// projects, and `graph.identities`, the records with how many sessions and
// projects each has. Mutates `saved.known` and `saved.signins`, and says
// whether they changed.
export function linkIdentities(graph, { accounts = {}, repos = new Map(), owners = new Map(), saved = emptySaved(), home = null, now = Date.now() } = {}) {
  const records = new Map();
  let dirty = false;
  const source = (s) => (home ? tilde(s, home) : s);
  const record = (r, src, extra = {}) => {
    let rec = records.get(r.id);
    if (!rec) records.set(r.id, (rec = { id: r.id, kind: r.kind, handle: r.handle ?? null, detail: r.detail ?? {}, signedIn: false, active: false, sources: [] }));
    if (!rec.handle && r.handle) rec.handle = r.handle;
    if (r.detail) for (const [k, v] of Object.entries(r.detail)) if (v != null) rec.detail[k] = v;
    if (src && !rec.sources.includes(src)) rec.sources.push(src);
    Object.assign(rec, extra);
    return rec;
  };

  // Signed in now.
  const gh = accounts.gh ?? [];
  if (accounts.claude) record(accounts.claude, source(accounts.claudeFile ?? "~/.claude.json"), { signedIn: true });
  if (accounts.chatgpt) record(accounts.chatgpt, source(accounts.chatgptFile ?? "~/.codex/auth.json"), { signedIn: true });
  for (const h of gh) for (const u of h.users) record({ id: githubId(h.host, u), kind: "github", handle: u, detail: { host: h.host } }, "gh", { signedIn: true, active: u.toLowerCase() === h.active?.toLowerCase() });
  const ghLogin = (host, login) => gh.find((h) => h.host === host)?.users.find((u) => u.toLowerCase() === String(login).toLowerCase()) ?? null;
  const isForge = (host) => host === "github.com" || host.endsWith(".ghe.com") || gh.some((h) => h.host === host);

  // The sign-in timelines.
  const signins = (saved.signins ??= {});
  for (const [app, acc] of [["claude", accounts.claude], ["chatgpt", accounts.chatgpt]]) {
    if (acc) dirty = noteSignin((signins[app] ??= []), acc.id, now, acc.since ?? null) || dirty;
  }

  // Sessions: the account each ran as.
  const nodes = graph.nodes;
  const byId = new Map(nodes.map((n) => [n.id, n]));
  // Claude accounts the app's own records name, besides the signed-in one.
  const recorded = new Set([...owners.values()].filter(Boolean).map((o) => claudeId(o.account, o.org)));
  for (const n of nodes) if (n.kind === "session" && n.app === "claude" && n.claudeAccount) recorded.add(claudeId(n.claudeAccount.account, n.claudeAccount.org));
  const knownOf = (kind) => new Set(Object.entries(saved.known ?? {}).filter(([, k]) => k.kind === kind).map(([id]) => id));
  const claudeKnown = new Set([...knownOf("claude"), ...recorded]);
  const chatgptKnown = new Set([...knownOf("chatgpt"), ...knownOf("openai")]);
  const current = (acc) => (acc ? { id: acc.id, since: acc.since ?? null } : null);
  for (const n of nodes) {
    if (n.kind !== "session") continue;
    const pin = saved.pins?.[n.id];
    let link = null;
    if (pin) link = { id: pin.identity, confidence: "yours", via: "your pin" };
    else if (n.app === "claude") {
      if (n.claudeAccount) link = { id: claudeId(n.claudeAccount.account, n.claudeAccount.org), confidence: "exact", via: "Claude desktop's session folder" };
      else if (owners.get(n.id)) link = { id: claudeId(owners.get(n.id).account, owners.get(n.id).org), confidence: "exact", via: "its transcript" };
      else link = accountAt(signins.claude ?? [], current(accounts.claude), n.updatedAt ?? n.createdAt, claudeKnown);
    } else if (n.app === "codex" && (!n.provider || n.provider === "openai")) {
      link = accountAt(signins.chatgpt ?? [], current(accounts.chatgpt), n.updatedAt ?? n.createdAt, chatgptKnown);
    }
    n.ranAs = link;
    if (link) record({ id: link.id, kind: link.id.split(":")[0], handle: saved.known?.[link.id]?.handle ?? null }, link.via === "Claude desktop's session folder" ? "Claude desktop" : null);
  }

  // Projects: who owns the remote, who pushes, who commits.
  const parent = new Map();
  for (const l of graph.links) if (l.kind !== "lineage-cross") parent.set(typeof l.target === "object" ? l.target.id : l.target, typeof l.source === "object" ? l.source.id : l.source);
  const projects = nodes.filter((n) => n.kind === "project");
  for (const p of projects) {
    const links = [];
    const remote = parseRemote(p.git?.origin, accounts.ssh);
    if (remote && isForge(remote.host)) {
      const login = ghLogin(remote.host, remote.owner) ?? remote.owner;
      const id = githubId(remote.host, login);
      record({ id, kind: "github", handle: login, detail: { host: remote.host } }, "remote URLs");
      links.push({ id, role: "owner", confidence: "exact", via: "its origin remote" });
    }
    const repo = repos.get(p.key);
    if (repo?.email) {
      const id = gitId(repo.email);
      record({ id, kind: "git", handle: repo.email, detail: { name: repo.name ?? null } }, "git config");
      links.push({ id, role: "commits-as", confidence: "exact", via: "git config" });
    }
    if (repo?.push) {
      const login = repo.push.user ?? gh.find((h) => h.host === repo.push.host)?.active;
      if (login) {
        const id = githubId(repo.push.host, ghLogin(repo.push.host, login) ?? login);
        record({ id, kind: "github", handle: ghLogin(repo.push.host, login) ?? login, detail: { host: repo.push.host } }, repo.push.user ? "git config" : "gh");
        links.push({ id, role: "pushes-as", confidence: repo.push.user ? "exact" : "likely", via: repo.push.user ? repo.push.how : "gh's active account" });
      }
    }
    p.identities = links;
  }
  // A group takes the owner all its projects share, and a project with no
  // remote of its own takes its group's.
  const ownerOf = (p) => p.identities.find((l) => l.role === "owner")?.id;
  for (const g of projects.filter((p) => p.group).sort((a, b) => b.key.length - a.key.length)) {
    const owners = new Set(projects.filter((p) => p !== g && parent.get(p.id) === g.id).map(ownerOf).filter(Boolean));
    if (owners.size === 1 && !ownerOf(g)) g.identities.push({ id: [...owners][0], role: "owner", confidence: "likely", via: "the projects in it" });
  }
  // (A satellite, like atlas-api-local-db beside atlas-api, takes its
  // project's. Two passes, for a satellite of a project in a group.)
  for (let pass = 0; pass < 2; pass++) {
    for (const p of projects) {
      if (p.group || ownerOf(p)) continue;
      const up = byId.get(parent.get(p.id));
      const inherited = up?.kind === "project" ? up.identities?.find((l) => l.role === "owner") : null;
      if (inherited) p.identities.push({ id: inherited.id, role: "owner", confidence: inherited.confidence === "exact" ? "likely" : inherited.confidence, via: up.group ? `its group, ${up.name}` : `the project it's grouped with, ${up.name}` });
    }
  }
  for (const p of projects) {
    const pin = saved.pins?.[p.id];
    if (pin) p.identities = [{ id: pin.identity, role: "owner", confidence: "yours", via: "your pin" }, ...p.identities.filter((l) => l.role !== "owner")];
  }

  // Remember what was seen, so an account you've switched away from keeps
  // its email, and every identity keeps when XenoNet first saw it.
  const known = (saved.known ??= {});
  for (const rec of records.values()) {
    const k = known[rec.id];
    if (!rec.handle && k?.handle) rec.handle = k.handle;
    if (k?.detail) rec.detail = { ...k.detail, ...rec.detail };
    if (!k) {
      known[rec.id] = { kind: rec.kind, handle: rec.handle, detail: rec.detail, firstSeen: now, lastSeen: now };
      dirty = true;
    } else if ((rec.handle && rec.handle !== k.handle) || now - (k.lastSeen ?? 0) >= SIGNIN_STEP_MS) {
      Object.assign(k, { handle: rec.handle ?? k.handle, detail: { ...k.detail, ...rec.detail }, lastSeen: now });
      dirty = true;
    }
    rec.firstSeen = known[rec.id].firstSeen;
    rec.seenAt = now;
  }
  for (const pin of Object.values(saved.pins ?? {})) {
    if (!records.has(pin.identity) && known[pin.identity]) record({ id: pin.identity, ...known[pin.identity] }, null, { firstSeen: known[pin.identity].firstSeen });
  }

  // How many sessions and projects each one reaches; a session reaches its
  // project's identities too.
  const homeProject = (id) => {
    for (let cur = parent.get(id), i = 0; cur && i < 12; cur = parent.get(cur), i++) if (byId.get(cur)?.kind === "project") return byId.get(cur);
    return null;
  };
  const count = new Map();
  const bump = (id, key) => {
    const c = count.get(id) ?? { sessions: 0, projects: 0 };
    c[key]++;
    count.set(id, c);
  };
  for (const p of projects) for (const id of new Set(p.identities.map((l) => l.id))) bump(id, "projects");
  for (const n of nodes) {
    if (n.kind !== "session") continue;
    const ids = new Set([n.ranAs?.id, ...(homeProject(n.id)?.identities ?? []).map((l) => l.id)].filter(Boolean));
    for (const id of ids) bump(id, "sessions");
  }

  graph.identities = [...records.values()]
    .map((r) => ({ ...r, name: saved.names?.[r.id] ?? null, hidden: !!saved.hidden?.[r.id], ...(count.get(r.id) ?? { sessions: 0, projects: 0 }) }))
    .sort((a, b) => b.sessions - a.sessions || b.projects - a.projects || a.id.localeCompare(b.id));
  return { identities: graph.identities, dirty };
}

// ---------- reading it from disk ----------

const mtimeOf = (file) => {
  try {
    return fs.statSync(file).mtimeMs;
  } catch {
    return null;
  }
};

// The config file a repo's settings live in: .git/config, or for a worktree
// or submodule, the one its .git file points at.
function repoConfigFile(dir) {
  let gitDir = path.join(dir, ".git");
  try {
    if (fs.statSync(gitDir).isFile()) {
      const m = /gitdir:\s*(.+)/.exec(fs.readFileSync(gitDir, "utf8"));
      if (!m) return null;
      gitDir = path.resolve(dir, m[1].trim());
    }
    const common = fs.existsSync(path.join(gitDir, "commondir")) ? fs.readFileSync(path.join(gitDir, "commondir"), "utf8").trim() : null;
    return path.join(common ? path.resolve(gitDir, common) : gitDir, "config");
  } catch {
    return null;
  }
}

function gitConfig(dir, env) {
  return new Promise((resolve) => {
    execFile("git", ["-C", dir, "config", "--get-regexp", "^(user\\.(name|email)|credential\\..*)$"], { env, timeout: 8000, windowsHide: true, maxBuffer: 1 << 20 }, (err, stdout) => {
      // Exit status 1 just means nothing matched.
      resolve(err && err.code !== 1 ? null : parseGitConfig(stdout));
    });
  });
}

// Reads the identity sources and keeps what they said until they change.
// Each source is a gatherer (core/blocks/gatherer.js) read again only when
// its stamp changes: an account file by its mtime, and a repo's git
// identity by its config's mtime, your git config files' and its remote.
// The watch is a block itself, "identities", degraded while a source can't
// be read.
export class IdentityWatch extends Block {
  constructor({ home, claudeConfigFile, codexHome, ghConfigDir, claudeProjectsDir, clock }) {
    super({ name: "identities", kind: "gatherer", clock });
    this.home = home;
    this.claudeConfigFile = claudeConfigFile;
    this.codexHome = codexHome;
    this.ghConfigDir = ghConfigDir;
    this.claudeProjectsDir = claudeProjectsDir;
    // Away from your own home folder (the demo, tests), git reads that
    // folder's config rather than yours.
    const sandboxed = path.resolve(home) !== path.resolve(os.homedir());
    this.gitEnv = sandboxed ? { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^GIT_CONFIG/.test(k))), HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: path.join(home, ".config"), GIT_CONFIG_NOSYSTEM: "1" } : process.env;
    this.forget();
  }

  // "Detect identities again": nothing cached.
  forget() {
    this.files = new Map(); // file → its source
    this.repoSources = new Map(); // project key → { p, pass, source }
    this.owners = new Map(); // session id → { owner, size }
    this.transcripts = null; // session id → transcript file
    this.transcriptsAt = 0;
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
    const main = path.join(this.home, ".gitconfig");
    const files = [main, path.join(this.gitEnv.XDG_CONFIG_HOME ?? path.join(this.home, ".config"), "git", "config")];
    try {
      for (const m of fs.readFileSync(main, "utf8").matchAll(/^\s*path\s*=\s*(.+?)\s*$/gm)) files.push(m[1].replace(/^~(?=[\\/])/, this.home));
    } catch {}
    return files.map(mtimeOf).join("|");
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
        stamp: () => `${r.pass.global}|${mtimeOf(repoConfigFile(r.p.path))}|${r.p.git.origin ?? ""}`,
        gather: async () => {
          const cfg = await gitConfig(r.p.path, this.gitEnv);
          if (!cfg) throw new Error(`git couldn't read the config of ${tilde(r.p.path, this.home)}`);
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
    const problem = file ? tilde(`${file[0]}: ${file[1].last.error}`, this.home) : (repo?.source.last.error ?? null);
    if (!problem) this.ok();
    else if (this.state !== "degraded" || this.lastError?.message !== problem) this.problem(problem);
  }

  health() {
    const h = super.health();
    const runs = (sources) => sources.reduce((n, s) => n + (s.counters.runs ?? 0), 0);
    h.counters = { ...h.counters, files: this.files.size, fileReads: runs([...this.files.values()]), repos: this.repoSources.size, repoReads: runs([...this.repoSources.values()].map((r) => r.source)) };
    return h;
  }

  // Where each Claude Code transcript is, by session id.
  #transcript(id) {
    if (!this.transcripts?.has(id) && Date.now() - this.transcriptsAt > 60e3) {
      this.transcripts = new Map();
      this.transcriptsAt = Date.now();
      try {
        for (const dir of fs.readdirSync(this.claudeProjectsDir)) {
          try {
            for (const f of fs.readdirSync(path.join(this.claudeProjectsDir, dir))) if (f.endsWith(".jsonl")) this.transcripts.set(`claude:${f.slice(0, -6)}`, path.join(this.claudeProjectsDir, dir, f));
          } catch {}
        }
      } catch {}
    }
    return this.transcripts?.get(id) ?? null;
  }

  // The account a Remote Control session names in its transcript's first
  // lines. Read once per session, again only while the file is still short
  // enough for the entry to turn up.
  transcriptOwner(id) {
    const hit = this.owners.get(id);
    if (hit && (hit.owner || hit.size >= HEAD_BYTES)) return hit.owner;
    const file = this.#transcript(id);
    const size = file ? (fs.statSync(file, { throwIfNoEntry: false })?.size ?? 0) : 0;
    if (hit && hit.size === size) return null;
    let owner = null;
    if (file) {
      try {
        const fd = fs.openSync(file, "r");
        const buf = Buffer.alloc(Math.min(size, HEAD_BYTES));
        try {
          fs.readSync(fd, buf, 0, buf.length, 0);
        } finally {
          fs.closeSync(fd);
        }
        for (const line of buf.toString("utf8").split("\n")) {
          if (!line.includes('"ownerAccountUuid"')) continue;
          const o = JSON.parse(line);
          if (str(o.ownerAccountUuid) && str(o.ownerOrganizationUuid)) owner = { account: o.ownerAccountUuid, org: o.ownerOrganizationUuid };
          break;
        }
      } catch {}
    }
    this.owners.set(id, { owner, size });
    return owner;
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
