// What an export of the day may say. Posters and films of the day are made
// to share, so they carry no project, repo, session, branch, path, host or
// prompt names, and no clock (docs/guide/privacy.md). Projects show as
// star names made from a keyed hash of each one's id: the same all day, so
// a day's poster and film agree, and new the next day, so no one can follow
// one star from day to day. The key is a salt kept in XenoNet's state
// folder. Without it a name can't be checked against a guess, and with it
// a name still can't be turned back into an id.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { LABELLED_KINDS } from "./export-labels.js";

// The syllables a star name is made of: an onset, a vowel and, at the end,
// sometimes a coda. Three bytes of the hash pick each one.
const ONSETS = ["b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "th", "sh", "kr", "dr", "vr"];
const VOWELS = ["a", "e", "i", "o", "u", "ae", "ai", "ei", "io", "ou"];
const CODAS = ["", "", "n", "r", "s", "l", "th", "x"];

// The day a name holds for, as the local date.
export const dayKey = (now = Date.now()) => new Date(now).toLocaleDateString("en-CA");

const digest = (salt, day, id) => crypto.createHmac("sha256", String(salt)).update(`${day}\u0000${id}`).digest();

// A name spelt from the bytes of a hash, `syllables` long.
function spell(bytes, syllables) {
  let s = "";
  for (let i = 0; i < syllables; i++) {
    const [a, b, c] = [bytes[i * 3], bytes[i * 3 + 1], bytes[i * 3 + 2]];
    s += ONSETS[a % ONSETS.length] + VOWELS[b % VOWELS.length];
    if (i === syllables - 1) s += CODAS[c % CODAS.length];
  }
  return s[0].toUpperCase() + s.slice(1);
}

// One star name, two or three syllables, from the hash of the id under the
// salt and the day.
export function starName(id, { salt, day = dayKey() }) {
  const bytes = digest(salt, day, id);
  return spell(bytes, 2 + (bytes[31] % 2));
}

// Star names for a set of ids, each one different: a name two ids share
// grows a syllable for the one whose hash sorts later, so nothing about an
// id's own order decides it.
export function starNames(ids, { salt, day = dayKey() }) {
  const names = new Map();
  const taken = new Set();
  const hashed = [...new Set(ids)].map((id) => ({ id, bytes: digest(salt, day, id) })).sort((a, b) => Buffer.compare(a.bytes, b.bytes));
  for (const { id, bytes } of hashed) {
    let n = 2 + (bytes[31] % 2);
    let name = spell(bytes, n);
    for (let k = 2; taken.has(name); k++) name = n < 5 ? spell(bytes, ++n) : `${spell(bytes, 5)} ${k}`;
    taken.add(name);
    names.set(id, name);
  }
  return names;
}

// The salt, kept in XenoNet's state folder as poster-salt: made the first
// time an export needs it, readable by you alone, and never part of an
// export. Without a folder, a salt for this run alone.
let runSalt = null;
export function posterSalt(dir) {
  if (!dir) return (runSalt ??= crypto.randomBytes(32).toString("hex"));
  const file = path.join(dir, "poster-salt");
  try {
    const kept = fs.readFileSync(file, "utf8").trim();
    if (/^[0-9a-f]{64}$/.test(kept)) return kept;
  } catch {}
  const salt = crypto.randomBytes(32).toString("hex");
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, salt + "\n", { mode: 0o600 });
  return salt;
}

// The day's archetype, the label a poster gives the day: what its sessions
// made together. One session is a lone star and two a binary. With three
// or more, a pulsar when the busiest hour holds two fifths of a day of at
// least twenty tool calls (the work came in a burst), a swarm at six or
// more sessions, else a cluster. A day without sessions is a quiet sky.
// Nothing here says which hour.
export function archetypeOf({ totals, hours } = {}) {
  const n = totals?.sessions ?? 0;
  if (n === 0) return "quiet sky";
  if (n === 1) return "lone star";
  if (n === 2) return "binary";
  const calls = (hours ?? []).reduce((a, b) => a + b, 0);
  const peak = Math.max(0, ...(hours ?? []));
  if (calls >= 20 && peak >= calls * 0.4) return "pulsar";
  return n >= 6 ? "swarm" : "cluster";
}

// What a poster or film gets of the day's log (digest.js): the date, the
// archetype, the totals, the tokens, the brightest stars by star name and
// how many sessions ran in each app. No ids, no names, no hours and no
// times. With `names`, each star also carries its project's name, for an
// export that reveals them.
// The ids a day's names cover: the brightest projects of the log and every
// labelled node on the map (export-labels.js), named together so the
// overlay of a film or a poster and the map behind it agree on each name.
const namedIds = (day, nodes) => [...(day?.topProjects ?? []).map((p) => p.id), ...nodes.filter((n) => LABELLED_KINDS.has(n?.kind)).map((n) => n.id)];

export function constellationOf(day, { salt, now = Date.now(), names = false, nodes = [] } = {}) {
  const projects = day?.topProjects ?? [];
  const stars = starNames(namedIds(day, nodes), { salt, day: dayKey(now) });
  const apps = {};
  for (const s of day?.sessions ?? []) apps[s.app || "other"] = (apps[s.app || "other"] ?? 0) + 1;
  const t = day?.totals ?? {};
  return {
    date: day?.date ?? "",
    archetype: archetypeOf(day ?? {}),
    totals: { sessions: t.sessions ?? 0, turns: t.turns ?? 0, toolCalls: t.toolCalls ?? 0, files: t.files ?? 0 },
    tokens: day?.tokens ?? 0,
    stars: projects.map((p) => ({ name: stars.get(p.id), toolCalls: p.toolCalls ?? 0, ...(names ? { project: p.name } : {}) })),
    apps,
  };
}

// The star name of each labelled node on the map, by id, from the same
// names the day's constellation gets: what the map's own labels show while
// a film plays or a poster renders (export-labels.js). The keys are ids
// the page already holds, so this is a lookup for the page and never part
// of an export, which carries the names alone.
export function labelsOf(nodes, day, { salt, now = Date.now() } = {}) {
  const stars = starNames(namedIds(day, nodes), { salt, day: dayKey(now) });
  const labels = {};
  for (const n of nodes) if (LABELLED_KINDS.has(n?.kind)) labels[n.id] = stars.get(n.id);
  return labels;
}
