// What actors' events say (docs/design/actors/interface.md, sections 1 and
// 6). XenoNet writes every detail from facts: ids, counts, and the names of
// projects, apps and platforms, never text an actor sends. The one
// exception is a status, which is the actor's own line, checked first
// (statusLine). Details name projects, never session titles, which come
// from conversations, so a session is only ever "a Claude Code session in
// atlas-api".

export const APPS = { claude: "Claude Code", codex: "Codex" };

// How a message left, by the fixed list of platforms a report may name, so
// no free text gets in.
const PLATFORMS = {
  telegram: "on Telegram",
  discord: "on Discord",
  slack: "on Slack",
  whatsapp: "on WhatsApp",
  signal: "on Signal",
  matrix: "on Matrix",
  mattermost: "on Mattermost",
  email: "by email",
  sms: "by text message",
  imessage: "on iMessage",
  teams: "on Teams",
};
const FINISHED = { session: "Finished a session", task: "Finished a task", job: "A scheduled job ran", turn: "Finished a turn", command: "A command finished" };
const FAILED = { session: "A session failed", task: "A task failed", job: "A scheduled job failed", turn: "A turn failed", command: "A command failed" };

const plural = (n, one, many) => (n === 1 ? one : `${n} ${many}`);
const count = (n) => (Number.isInteger(n) && n > 0 ? n : 1);
export const clip = (s, n) => (s == null ? null : s.length > n ? `${s.slice(0, n - 1)}…` : s);

// "a Claude Code session in atlas-api"
export const sessionWords = (app, place) => `a ${APPS[app] ? `${APPS[app]} ` : ""}session${place ? ` in ${place}` : ""}`;

// A file by its name alone, cut short: the folders above it are the
// project's to name. One that reads like a key is just "a file".
function fileName(p) {
  const name = clip(oneLine(String(p).split(/[\\/]/).filter(Boolean).pop() ?? ""), 60);
  return name && looksLikeKey(name) ? "a file" : name;
}
function fileWords(paths, n) {
  const names = (paths ?? []).map(fileName).filter(Boolean);
  if (names.length) return names.length === 1 ? names[0] : `${names[0]} and ${plural(names.length - 1, "1 more file", "more files")}`;
  return n ? plural(n, "a file", "files") : "files";
}

// A presence that changed, from what it was.
export function presenceWords(from, to) {
  if (to === "away") return "Went away";
  if (to === "offline") return "Went offline";
  if (to === "unknown") return "Out of sight";
  if (!from || from === "offline" || from === "unknown") return "Came online";
  if (from === "away") return "Came back";
  return to === "idle" ? "Went idle" : "Started working";
}

// What an event says, from its kind, its facts (count, paths, platform,
// subject, app, to) and what the map says of its target (`where`: a
// session's app and place, or a place's name; see Actors#where). Status
// and presence events say their own (statusLine, presenceWords).
export function writeDetail(kind, facts = {}, where = null) {
  const place = where?.place ?? null;
  const at = place ? ` in ${place}` : "";
  switch (kind) {
    case "spawned":
      if (facts.to) return `Handed ${sessionWords(facts.app, place)} to ${APPS[facts.to] ?? "another app"}`;
      // A session not on the map yet says its app through the facts.
      if (where?.kind === "session") return `Started ${sessionWords(where.app ?? facts.app, place)}`;
      return `Started ${sessionWords(facts.app, place)}`;
    case "edited":
      return `Edited ${fileWords(facts.paths, facts.count)}${at}`;
    case "ran":
      return `Ran ${plural(count(facts.count), "a command", "commands")}${at}`;
    case "messaged": {
      const how = PLATFORMS[facts.platform];
      return `Sent ${plural(count(facts.count), "a message", "messages")}${how ? ` ${how}` : ""}`;
    }
    case "said":
      return `Said something to ${plural(count(facts.count), "an agent", "agents")}${at}`;
    case "finished":
      return `${FINISHED[facts.subject] ?? "Finished"}${at}`;
    case "failed":
      return `${FAILED[facts.subject] ?? "Something failed"}${at}`;
    default:
      return null;
  }
}

// What the person at the console is doing, as XenoNet words it from what the
// window reported (section 6): looking at a place or a session, reading or
// opening a session, or a terminal opened in a folder.
export function consoleStatus(what, where) {
  const it = where?.kind === "session" ? sessionWords(where.app, where.place) : where?.name;
  if (!it) return null;
  if (what === "looked") return `Looking at ${it}`;
  if (what === "read") return `Reading ${it}`;
  if (what === "opened") return `Opened ${it}`;
  if (what === "terminal") return `Opened a terminal in ${where.place ?? where.name}`;
  return null;
}

// ---------- status lines ----------

// Control characters, and the ones that reorder text, out; any run of
// spaces, tabs or line breaks becomes one space.
const oneLine = (s) =>
  String(s)
    .replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029]+/g, " ")
    .replace(/[\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g, "")
    .replace(/\s+/g, " ")
    .trim();

// Words that start the way keys and tokens do, and a run of 32 or more
// letters and digits that mixes both (hex and base64 keys do; words and
// paths seldom do).
const KEY_PREFIX = /(?:^|[^A-Za-z0-9])(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)/;
const LONG_RUN = /[A-Za-z0-9]{32,}/g;
export function looksLikeKey(s) {
  if (KEY_PREFIX.test(s)) return true;
  for (const run of s.match(LONG_RUN) ?? []) if (/[A-Za-z]/.test(run) && /\d/.test(run)) return true;
  return false;
}

// A place's name for a detail or a status, or what it is when its name
// reads like a key (a folder named for a hash, say), which a status would
// refuse.
const KINDS = { project: "a project", zone: "a folder", native: "a project" };
export const placeName = (name, kind) => (name == null ? null : looksLikeKey(oneLine(name)) ? (KINDS[kind] ?? "a place") : oneLine(name));

export const STATUS_MAX = 140;

// A status as XenoNet keeps it: one line, at most 140 characters (cut), or
// null for an empty one, which clears it. { ok: false, reason } for one
// that looks like it holds a key, which is refused rather than cut.
export function statusLine(text) {
  if (text == null) return { ok: true, status: null };
  if (typeof text !== "string") return { ok: false, reason: "a status is text" };
  const line = oneLine(text);
  if (!line) return { ok: true, status: null };
  if (looksLikeKey(line)) return { ok: false, reason: "that status looks like it holds a key or a token, so XenoNet won't show it" };
  return { ok: true, status: clip(line, STATUS_MAX) };
}
