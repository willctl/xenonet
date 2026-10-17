// Selectors: which sessions a message goes to, or which ones a caller may
// see at all. A selector is one or more key:value terms, all of which must
// hold (AND), separated by spaces or "+":
//
//   project:xenonet            the project's name (or the repo's folder)
//   identity:adal           an account the session ran as or belongs to:
//                              a GitHub login, a git email, a Claude account
//   app:opencode               the harness (claude, codex, opencode, …)
//   machine:local              the machine (local for this one, or a host's name)
//   status:idle                idle, working or live (live = working or idle, not ended)
//   by:hermes                  sessions an actor started or works on, or (in
//                              a grant) the actor itself
//
// (Not actor:…, which is an actor's own id and still names that actor.)
//
// A value may end in * for a prefix. "all" is the empty selector. Anything
// that isn't made of these terms (a session's id like claude:abc…, words
// from a title, "repo") isn't a selector, so what `to` took before still
// means what it meant.
export const KEYS = ["project", "identity", "app", "machine", "status", "by"];
const TERM = new RegExp(`^(${KEYS.join("|")}):([^\\s+]+)$`, "i");

export function parseSelector(text) {
  const s = String(text ?? "").trim();
  if (!s) return null;
  if (s === "all") return { terms: [] };
  const parts = s.split(/[\s+]+/).filter(Boolean);
  const terms = [];
  for (const p of parts) {
    const m = TERM.exec(p);
    if (!m) return null;
    terms.push({ key: m[1].toLowerCase(), value: m[2].toLowerCase() });
  }
  return { terms };
}

export const isSelector = (text) => !!parseSelector(text) && String(text).trim() !== "all";
export const formatSelector = (sel) => (sel?.terms?.length ? sel.terms.map((t) => `${t.key}:${t.value}`).join(" ") : "all");

const like = (value, want) => {
  if (value == null) return false;
  const v = String(value).toLowerCase();
  return want.endsWith("*") ? v.startsWith(want.slice(0, -1)) : v === want;
};

// The facts a selector is matched against, for one session or actor (see
// facts() in policy.js): { app, project, projectNames[], identities[],
// machine, status, actors[] }.
export function matches(sel, f) {
  if (!sel) return false;
  for (const { key, value } of sel.terms) {
    const ok =
      key === "app" ? like(f.app, value) :
      key === "project" ? (f.projectNames ?? []).some((n) => like(n, value)) :
      key === "identity" ? (f.identities ?? []).some((n) => like(n, value)) :
      key === "machine" ? like(f.machine ?? "local", value) :
      key === "status" ? (value === "live" ? f.status !== "ended" : like(f.status, value)) :
      key === "by" ? (f.actors ?? []).some((n) => like(n, value)) :
      false;
    if (!ok) return false;
  }
  return true;
}

// Whether everything `inner` can select lies inside `outer`: every term of
// outer appears in inner. Used to check that an actor asks only within the
// scope its token was granted.
export function within(inner, outer) {
  if (!outer) return false;
  return outer.terms.every((o) => inner.terms.some((i) => i.key === o.key && (i.value === o.value || (o.value.endsWith("*") && i.value.startsWith(o.value.slice(0, -1))))));
}
