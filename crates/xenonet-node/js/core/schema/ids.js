// core/schema/ids.js in Rust (crates/xenonet-base/src/ids.rs): schema ids,
// host-qualified ids, sessions, actors and repos, as the contracts fix
// them. What's checked of a JS value (that it's a string, a whole number)
// and how an error writes one stay here, as the JS wrote them.
import { native } from "../../index.js";

const P = native.idPatterns();

export const SCHEMA_BASE = P.schemaBase;

export const isSchemaPath = (path) => typeof path === "string" && native.isSchemaPath(path);

export function shortId(path, major) {
  if (!isSchemaPath(path)) throw new Error(`not a schema path: ${path}`);
  if (!Number.isInteger(major) || major < 1) throw new Error(`not a major version: ${major}`);
  return native.shortId(path, major);
}

export const schemaId = (path, major) => schemaUri(major === undefined ? path : shortId(path, major));
export function schemaUri(short) {
  const uri = native.schemaUri(String(short ?? ""));
  if (uri == null) throw new Error(`not a schema id: ${short}`);
  return uri;
}

export const parseSchemaId = (id) => native.parseSchemaId(String(id ?? "")) ?? null;

// ---------- hosts ----------

export const HOST_ID = new RegExp(P.hostId);
export const isHostId = (host) => typeof host === "string" && native.isHostId(host);

export function parseQualified(id) {
  if (typeof id !== "string" || !id) return null;
  const q = native.parseQualified(id);
  return q ? { host: q.host ?? null, local: q.local } : null;
}

export const isQualified = (id) => parseQualified(id)?.host != null;
export const hostOf = (id) => parseQualified(id)?.host ?? null;
export const localOf = (id) => parseQualified(id)?.local ?? null;

export function qualify(id, host = null, { self = null } = {}) {
  const r = native.qualify(typeof id === "string" ? id : "", host == null ? null : String(host), self == null ? null : String(self));
  if (r.error === "id") throw new Error(`not an id: ${JSON.stringify(id)}`);
  if (r.error === "host") throw new Error(`not a host id: ${JSON.stringify(host)}`);
  return r.id;
}

export const formatQualified = ({ host = null, local }, opts) => qualify(local, host, opts);

// ---------- sessions ----------

export const SESSION_APPS = Object.fromEntries(native.sessionAppNames());
export const isSessionApp = (app) => typeof app === "string" && native.isSessionApp(app);
export const SESSION_LOCAL = new RegExp(`^(${P.sessionApps}):[0-9A-Za-z][0-9A-Za-z-]{6,}$`);
export const SESSION_PREFIX = new RegExp(`^(${P.sessionApps}):`);
export const isSessionId = (id) => native.isSessionId(String(id ?? ""));
export const appOfSession = (id) => native.appOfSession(String(id ?? "")) ?? null;

// ---------- actors ----------

export const ACTOR_SLUG = new RegExp(P.actorSlug);
export const YOU = P.you;

export function actorId(slug, host = null) {
  if (typeof slug !== "string" || !native.isActorSlug(slug)) throw new Error(`not an actor slug: ${JSON.stringify(slug)}`);
  return qualify(`actor:${slug}`, host);
}

export function parseActorId(id) {
  if (typeof id !== "string") return null;
  const a = native.parseActorId(id);
  return a ? { host: a.host ?? null, slug: a.slug } : null;
}

export const isActorId = (id) => parseActorId(id) != null;

// ---------- repos ----------

export const normalizeRemote = (url) => (url ? native.normalizeRemote(url) : null);

export function repoId(remote) {
  const r = normalizeRemote(remote);
  return r ? `repo:${r}` : null;
}

// A project's repo key, which its own host computes, with `aliases` (alias
// → HostName, as parseSshAliases reads them) resolved.
export function repoKey(url, aliases = null) {
  const parts = native.remoteParts(String(url ?? ""));
  if (!parts) return null;
  const [h, repo] = parts;
  return normalizeRemote(`${aliases?.get?.(h) ?? h}/${repo}`);
}

export const parseRepoId = (id) => (typeof id === "string" ? (native.parseRepoId(id) ?? null) : null);
