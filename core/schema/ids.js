// Ids, as docs/design/contracts.md fixes them:
//   - a schema's $id, https://xenonet.dev/schemas/<path>/v<major>.json,
//     and its short form, <path>/v<major>, which data carries as "schema";
//   - host-qualified ids: an unqualified id is this XenoNet's own host's, and
//     one from another host is @<host>:<local id>;
//   - actors, actor:<slug> (the person at the console is actor:you), and
//     projects merged across hosts by their git remote, repo:<remote>.
// It imports only core/apps.js, which has no imports, so the page can use it
// as well as the server.
import { APPS } from "../apps.js";

export const SCHEMA_BASE = "https://xenonet.dev/schemas";

// A path is lowercase segments of letters, digits and dashes:
// session, live/snapshot, vendor/openai/codex/rollout-line.
const PATH = /^[a-z0-9][a-z0-9-]*(?:\/[a-z0-9][a-z0-9-]*)*$/;

export const isSchemaPath = (path) => typeof path === "string" && PATH.test(path) && !/(^|\/)v\d+$/.test(path);

// "<path>/v<major>", the form data and XenoNet's registry use.
export function shortId(path, major) {
  if (!isSchemaPath(path)) throw new Error(`not a schema path: ${path}`);
  if (!Number.isInteger(major) || major < 1) throw new Error(`not a major version: ${major}`);
  return `${path}/v${major}`;
}

// The $id of a schema, from its path and major or from its short form.
export const schemaId = (path, major) => schemaUri(major === undefined ? path : shortId(path, major));
export function schemaUri(short) {
  const parsed = parseSchemaId(short);
  if (!parsed) throw new Error(`not a schema id: ${short}`);
  return `${SCHEMA_BASE}/${parsed.id}.json`;
}

// Either form, to { id: "<path>/v<major>", path, major }; null when it's
// neither.
export function parseSchemaId(id) {
  let s = String(id ?? "");
  if (s.startsWith(`${SCHEMA_BASE}/`) && s.endsWith(".json")) s = s.slice(SCHEMA_BASE.length + 1, -".json".length);
  const m = /^(.+)\/v([1-9]\d*)$/.exec(s);
  if (!m || !isSchemaPath(m[1])) return null;
  return { id: s, path: m[1], major: Number(m[2]) };
}

// ---------- hosts ----------

// A host id is a name you choose when hosts are first connected (studio,
// vm1), never the machine's own name, which often holds a person's.
export const HOST_ID = /^[a-z][a-z0-9-]{1,31}$/;
export const isHostId = (host) => typeof host === "string" && HOST_ID.test(host);

// "@vm1:claude:4f1c…" → { host: "vm1", local: "claude:4f1c…" }, and an
// unqualified id → { host: null, local: id }. The host runs from the @ to
// the first colon. Null for something that isn't an id: empty, or an @
// without a valid host and a local id after it.
export function parseQualified(id) {
  if (typeof id !== "string" || !id) return null;
  if (!id.startsWith("@")) return { host: null, local: id };
  const colon = id.indexOf(":");
  const host = colon < 0 ? "" : id.slice(1, colon);
  const local = colon < 0 ? "" : id.slice(colon + 1);
  return isHostId(host) && local && !local.startsWith("@") ? { host, local } : null;
}

export const isQualified = (id) => parseQualified(id)?.host != null;
export const hostOf = (id) => parseQualified(id)?.host ?? null;
export const localOf = (id) => parseQualified(id)?.local ?? null;

// An id as this XenoNet writes it: a local id from `host` gets the host's
// prefix, unless `host` is null or this XenoNet's own (`self`), whose ids stay
// as they are. An id that's already qualified keeps its host, except that
// this XenoNet's own comes off.
export function qualify(id, host = null, { self = null } = {}) {
  const parsed = parseQualified(id);
  if (!parsed) throw new Error(`not an id: ${JSON.stringify(id)}`);
  if (host != null && !isHostId(host)) throw new Error(`not a host id: ${JSON.stringify(host)}`);
  const from = parsed.host ?? host;
  return from == null || from === self ? parsed.local : `@${from}:${parsed.local}`;
}

// The other way round: { host, local } as an id.
export const formatQualified = ({ host = null, local }, opts) => qualify(local, host, opts);

// ---------- sessions ----------

// A session is the state around one run of a harness: a Claude Code
// session, a Codex thread, a Gemini CLI run. Its id is "<app>:<the run's
// id>". An actor (below) is something that lasts across runs. The apps XenoNet
// knows sessions of, and what each is called, from core/apps.js. claude and
// codex XenoNet reads itself; the others are observed (numbat's records, or a
// run an actor reports through the gateway) until XenoNet has a reader of its
// own.
export const SESSION_APPS = Object.fromEntries(APPS.map((a) => [a.id, a.name]));
export const isSessionApp = (app) => typeof app === "string" && Object.hasOwn(SESSION_APPS, app);
const APP_ALT = Object.keys(SESSION_APPS).sort((a, b) => b.length - a.length).join("|");
// A session's id, unqualified: the app, then the run's own id.
export const SESSION_LOCAL = new RegExp(`^(${APP_ALT}):[0-9A-Za-z][0-9A-Za-z-]{6,}$`);
export const SESSION_PREFIX = new RegExp(`^(${APP_ALT}):`);
export const isSessionId = (id) => SESSION_LOCAL.test(String(id ?? ""));
export const appOfSession = (id) => SESSION_PREFIX.exec(String(id ?? ""))?.[1] ?? null;

// ---------- actors ----------

export const ACTOR_SLUG = /^[a-z0-9][a-z0-9-]{0,31}$/;
export const YOU = "actor:you";

export function actorId(slug, host = null) {
  if (typeof slug !== "string" || !ACTOR_SLUG.test(slug)) throw new Error(`not an actor slug: ${JSON.stringify(slug)}`);
  return qualify(`actor:${slug}`, host);
}

// actor:<slug> or @<host>:actor:<slug> → { host, slug }; null for any other id.
export function parseActorId(id) {
  const q = parseQualified(id);
  const m = q && /^actor:(.+)$/.exec(q.local);
  return m && ACTOR_SLUG.test(m[1]) ? { host: q.host, slug: m[1] } : null;
}

export const isActorId = (id) => parseActorId(id) != null;

// ---------- repos ----------

// A remote as its host and path, the same for every way of writing it:
// git@github.com:willctl/xenonet.git and https://user@github.com/willctl/xenonet
// are both github.com/willctl/xenonet.
export function normalizeRemote(url) {
  if (!url) return null;
  return url
    .toLowerCase()
    .replace(/^git@([^:]+):/, "$1/")
    .replace(/^[a-z+]+:\/\//, "")
    .replace(/^[^@/]+@/, "")
    .replace(/\.git$/, "");
}

// A project merged across hosts by its git remote: repo:github.com/willctl/xenonet.
export function repoId(remote) {
  const r = normalizeRemote(remote);
  return r ? `repo:${r}` : null;
}

// A project's repo key, which its own host computes (contracts, section 2),
// since only that machine knows its SSH aliases: the remote's host with
// `aliases` resolved (alias → HostName, as parseSshAliases reads them from
// ~/.ssh/config), without ssh://, a port, a user name or a password, then
// normalizeRemote. So git@gh-work:willctl/xenonet on one machine and
// ssh://git@github.com:22/willctl/xenonet.git on another are both
// github.com/willctl/xenonet. A merged view's id is repo:<key>. Null for a
// remote that's a folder on disk (file://, /srv/git/x), which means
// something only on its own machine.
export function repoKey(url, aliases = null) {
  const s = String(url ?? "").trim();
  let host, rest;
  let m = /^([a-z][a-z0-9+.-]*):\/\/(?:[^@/]*@)?(\[[^\]/]+\]|[^/:]+)(?::\d*)?(\/.*)?$/i.exec(s);
  if (m) {
    if (m[1].toLowerCase() === "file") return null;
    [host, rest] = [m[2], m[3] ?? ""];
  } else if ((m = /^(?:[^@/]+@)?([^:/\\]{2,}):(?!\/\/)(.+)$/.exec(s))) [host, rest] = [m[1], m[2]];
  else return null;
  const repo = rest.replace(/^\/+/, "").replace(/\/+$/, "").replace(/\.git$/i, "");
  if (!repo) return null;
  const h = host.toLowerCase();
  return normalizeRemote(`${aliases?.get?.(h) ?? h}/${repo}`);
}

// repo:<remote> → the normalized remote; null for any other id.
export const parseRepoId = (id) => (typeof id === "string" && /^repo:./.test(id) ? id.slice("repo:".length) : null);
