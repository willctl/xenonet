// A remote actor's account links (docs/design/cloud-assistants.md, section
// 3): how sure XenoNet is of the account an actor works under, kept apart
// from authenticating it.
//
//   verified        a provider assertion its adapter validated
//                   (adapters.js); its subject is kept as a hash, never
//                   as sent
//   owner_asserted  your word, in XenoNet's window: you picked one of the
//                   identities XenoNet knows. Shown as yours, never as proof
//   unverified      neither: the state of every actor without a link
//
// A link never authenticates anything: grants do (connections.js). An
// actor's snapshot keeps identity null for a link that's only your word,
// and says how sure XenoNet is in remote.accountLink instead, which clients
// that don't know it leave out. An external ref names an actor in its
// provider's own ids, only once verified, under the namespace that proved
// it; a ref that would name a second actor is refused for you to review,
// never merged.
import crypto from "node:crypto";

const newId = (prefix) => `${prefix}_${crypto.randomBytes(12).toString("hex")}`;
const hashOf = (...parts) => crypto.createHash("sha256").update(parts.join("\0")).digest("hex");

export class LinkError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

// Your word that actor `slug` works as `identity` (one of the identities
// XenoNet knows, `known`), from XenoNet's window.
export function assertLink(store, { slug, identity, provider, known, now = Date.now() }) {
  if (typeof identity !== "string" || !known?.has(identity)) throw new LinkError("unknown-identity", "pick one of the identities XenoNet knows");
  const link = { link_id: newId("al"), actor_slug: slug, identity_id: identity, issuer: null, provider, workspace: null, proof_status: "owner_asserted", proof_method: "owner_selected", subject_binding: null, checked_at: now, expires_at: null, created_at: now };
  store.addAccountLink(link);
  return link;
}

// What an adapter verified (adapters.js `verified`), kept. It names an
// identity XenoNet knows only when the assertion maps to exactly that one:
// an OpenAI workspace to chatgpt:<workspace>. A verified link under another
// workspace than the one before means the account changed: the old links
// lapse, and what was approved under them is yours to approve again.
export function recordVerified(store, { slug, facts, known = new Set(), now = Date.now() }) {
  if (facts?.state !== "verified") throw new LinkError("unverified", "only a verified assertion is kept as verified");
  const identity = facts.provider === "openai" && facts.workspace && known.has(`chatgpt:${facts.workspace}`) ? `chatgpt:${facts.workspace}` : null;
  const before = store.accountLinks(slug).filter((l) => l.proof_status === "verified" && (l.expires_at == null || l.expires_at > now));
  const switched = before.some((l) => l.issuer !== facts.issuer || l.workspace !== (facts.workspace ?? null));
  store.transaction(() => {
    if (switched) for (const l of before) store.db.prepare("UPDATE actor_account_links SET expires_at = ? WHERE link_id = ?").run(now, l.link_id);
    store.addAccountLink({ link_id: newId("al"), actor_slug: slug, identity_id: identity, issuer: facts.issuer, provider: facts.provider ?? "unknown", workspace: facts.workspace ?? null, proof_status: "verified", proof_method: facts.method ?? "provider_assertion", subject_binding: hashOf(facts.issuer, facts.subject), checked_at: now, expires_at: facts.expiresAt ?? null, created_at: now });
  });
  return { identity, switched };
}

// How sure XenoNet is of an actor's account now: verified while an
// unexpired verified link stands, else owner_asserted while one of yours
// does, else unverified. With the identity it names, if any.
export function linkState(store, slug, now = Date.now()) {
  const live = store.accountLinks(slug).filter((l) => l.expires_at == null || l.expires_at > now);
  const v = live.findLast((l) => l.proof_status === "verified");
  if (v) return { state: "verified", identity: v.identity_id ?? null };
  const o = live.findLast((l) => l.proof_status === "owner_asserted");
  if (o) return { state: "owner_asserted", identity: o.identity_id ?? null };
  return { state: "unverified", identity: null };
}

// An actor's id at its provider, once its adapter verified it. The same id
// under the same namespace for another actor is refused: that's for you to
// look at, and XenoNet never merges actors by it, nor by a name, an email or
// a product.
export function addExternalRef(store, { slug, adapterId, facts, now = Date.now() }) {
  if (facts?.state !== "verified" || !facts.externalActorId) throw new LinkError("unverified", "an external id is kept only once its provider proved it");
  const key = { adapter_id: adapterId, issuer: facts.issuer, account_binding: facts.accountBinding, external_actor_id: facts.externalActorId };
  const had = store.externalRef(key);
  if (had && had.actor_slug !== slug) throw new LinkError("conflict", `that provider id already names actor:${had.actor_slug}; review both in XenoNet's window`);
  if (had) return had;
  const row = { external_ref_id: newId("xr"), actor_slug: slug, ...key, verified_at: now };
  store.addExternalRef(row);
  return row;
}
