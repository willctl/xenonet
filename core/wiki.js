// The shared context's wiki (Xenolith's) as a constellation: pages, their
// [[links]], the projects they mention, recent writes from the activity log,
// and notes still waiting in the inbox for the curator.
import fsp from "node:fs/promises";
import path from "node:path";
import { dayKey } from "./live.js";
import { formatProblem, readMarker, SPACE_KINDS, spaceOf } from "./xenolith.js";

export function parseFrontmatter(text) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  const meta = {};
  if (!m) return { meta, body: text };
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line);
    if (!kv) continue;
    let v = kv[2].trim();
    if (v.startsWith("[")) {
      try {
        v = JSON.parse(v);
      } catch {}
    } else v = v.replace(/^"(.*)"$/, "$1");
    meta[kv[1]] = v;
  }
  return { meta, body: text.slice(m[0].length) };
}

// The pages a text links to, as [[slug]] or [[slug|text]] (Xenolith's
// links_in, xenolith/rust/crates/core/src/activity.rs), each once, in
// lowercase.
export const wikiLinks = (text) => [...new Set([...String(text).matchAll(/\[\[([a-z0-9][a-z0-9-]*)(?:\|[^\]]*)?\]\]/gi)].map((m) => m[1].toLowerCase()))];

// A page's first "# " heading, its title when the frontmatter gives none
// (page_from_text in xenolith/rust/crates/core/src/wiki.rs).
const firstHeading = (body) => /^#\s+(.+)$/m.exec(body)?.[1].trim() ?? "";

// Other names a page goes by, as Xenolith reads its aliases (norm_tags in
// xenolith/rust/crates/core/src/wiki.rs): a list, or words split at
// commas, each on one line, lowercase, without a leading #, once.
function aliasesOf(v) {
  const list = Array.isArray(v) ? v.map(String) : v ? String(v).split(",") : [];
  return [...new Set(list.map((a) => a.replace(/\s*[\r\n]\s*/g, " ").trim().replace(/^#+/, "").trim().toLowerCase()).filter(Boolean))];
}

const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// Projects a page talks about: explicit paths into the projects folder,
// <folder>/<path> where <folder> is the folder's own name (Projects for
// ~/Projects, code for ~/code), or tags. A path into a project inside a
// group names that project, not the group too. Without `root`, the folder
// is taken to be called Projects.
export function mentionedProjects(text, tags, projects, root = null) {
  const hay = String(text).replace(/\\/g, "/").toLowerCase();
  const tagset = new Set((Array.isArray(tags) ? tags : []).map((t) => String(t).toLowerCase()));
  const folder = escape((String(root ?? "").split(/[\\/]+/).filter(Boolean).at(-1) ?? "projects").toLowerCase());
  // The folder's name starts a path, or a word: unicode/x isn't code/x.
  // The project's ends one: ledger isn't ledger-old or ledger.old, but a
  // period that ends a sentence is no part of it.
  const named = (p) => [p.rel, p.name].filter(Boolean).some((r) => new RegExp(`(?<![\\w.-])${folder}/${escape(r.toLowerCase())}(?![\\w-]|\\.[\\w-])`).test(hay));
  const hits = projects.filter((p) => named(p) || tagset.has(p.name.toLowerCase()));
  return hits.filter((p) => !hits.some((q) => q !== p && q.key.startsWith(p.key + "/"))).map((p) => p.key);
}

// Log entries look like "## 13:27 · codex · Title" with a "pages: [[a]]" line,
// or curator one-liners "- 13:53 · curator · Page updated: Title [[slug]]".
export function parseLog(text, date) {
  const out = [];
  let cur = null;
  for (const line of String(text).split(/\r?\n/)) {
    const h = /^##\s+(\d{1,2}):(\d{2})\s+·\s+([^·]+?)\s+·\s+(.+)$/.exec(line);
    const b = /^-\s+(\d{1,2}):(\d{2})\s+·\s+([^·]+?)\s+·\s+(.+)$/.exec(line);
    const hit = h ?? b;
    if (hit) {
      const [y, mo, d] = date.split("-").map(Number);
      cur = { ts: new Date(y, mo - 1, d, +hit[1], +hit[2]).getTime(), app: hit[3].trim(), title: hit[4].replace(/\s*\[\[[^\]]+\]\]\s*$/, "").trim(), pages: wikiLinks(hit[4]) };
      out.push(cur);
      if (b) cur = null;
      continue;
    }
    if (cur && /^pages:/i.test(line)) cur.pages = [...new Set([...cur.pages, ...wikiLinks(line)])];
  }
  return out;
}

// Xenolith refuses a link anywhere below the wiki's folder (confined_path in
// xenolith/rust/crates/core/src/wiki.rs), so a page or log linked in from
// elsewhere isn't the wiki's, and XenoNet passes it over too. The folder
// itself may be a link.
const linked = async (p) => (await fsp.lstat(p).catch(() => null))?.isSymbolicLink() ?? false;

// A folder's names, none when it's a link or can't be read.
const namesIn = async (dir) => ((await linked(dir)) ? [] : await fsp.readdir(dir).catch(() => []));

// A file's text, null when it's a link or can't be read.
const textOf = async (file) => ((await linked(file)) ? null : await fsp.readFile(file, "utf8").catch(() => null));

const isDir = async (p) => (await fsp.lstat(p).catch(() => null))?.isDirectory() ?? false;

// The spaces besides this machine's own, as Xenolith lists them (list in
// xenolith/rust/crates/core/src/spaces.rs): each a wiki laid out as the
// folder is, in spaces/<kind>/<name>/, by kind and then name. A folder
// that isn't a space's name, or is a link, isn't one.
async function spacesIn(dir) {
  const base = path.join(dir, "spaces");
  if (!(await isDir(base))) return [];
  const out = [];
  for (const kind of SPACE_KINDS) {
    if (!(await isDir(path.join(base, kind)))) continue;
    for (const name of await fsp.readdir(path.join(base, kind)).catch(() => [])) {
      if (spaceOf(`${kind}:${name}`) === `${kind}:${name}` && (await isDir(path.join(base, kind, name)))) out.push({ id: `${kind}:${name}`, kind, name, dir: path.join(base, kind, name) });
    }
  }
  return out.sort((a, b) => (a.kind !== b.kind ? (a.kind < b.kind ? -1 : 1) : a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

// One space's pages and the last two days of its log, each page with the
// writes the log names it in. `space` is null for this machine's own, the
// wiki's folder. A project's space is that project's context, so its pages
// are tied to the projects of its name.
async function readSpace({ id: space, kind, name, dir }, projects, { root, now }) {
  const pages = [];
  const files = await namesIn(path.join(dir, "pages"));
  for (const f of files.filter((f) => f.endsWith(".md"))) {
    if (await linked(path.join(dir, "pages", f))) continue;
    const text = await fsp.readFile(path.join(dir, "pages", f), "utf8").catch(() => "");
    const { meta, body } = parseFrontmatter(text);
    const slug = f.slice(0, -3).toLowerCase();
    // The title, then the first heading, then the slug; the type in
    // lowercase: as Xenolith reads a page.
    const page = {
      slug,
      title: meta.title || firstHeading(body) || slug,
      type: (String(meta.type ?? "") || "topic").toLowerCase(),
      summary: meta.summary || "",
      tags: Array.isArray(meta.tags) ? meta.tags : [],
      aliases: aliasesOf(meta.aliases),
      updated: Date.parse(meta.updated) || null,
      updatedBy: meta.updated_by || null,
      links: wikiLinks(body).filter((l) => l !== slug),
      projects: mentionedProjects(body, meta.tags, projects, root),
      path: path.join(dir, "pages", f),
      space,
    };
    if (kind === "project") for (const p of projects.filter((p) => p.name.toLowerCase() === name)) if (!page.projects.includes(p.key)) page.projects.push(p.key);
    pages.push(page);
  }
  const log = [];
  for (const offset of [1, 0]) {
    const date = dayKey(now - offset * 86400e3);
    const year = path.join(dir, "log", date.slice(0, 4));
    const text = (await linked(path.join(dir, "log"))) || (await linked(year)) ? null : await textOf(path.join(year, `${date}.md`));
    if (text) log.push(...parseLog(text, date));
  }
  const bySlug = new Map(pages.map((p) => [p.slug, Object.assign(p, { writes: [] })]));
  for (const e of log) for (const slug of e.pages) bySlug.get(slug)?.writes.push(e);
  return { pages, log };
}

// The shared context's folder (Xenolith's, or Agent Wiki's), read: the pages
// of this machine's space and of every other space in it (with the projects
// they mention, paths under `root`, the projects folder), the last two days
// of their logs, and the inbox. `server` is the MCP server that owns it.
// For a folder in a format XenoNet doesn't read, none of those, and the
// `problem` that says why. Null without a folder.
export async function readWiki(dir, projects, { root = null, now = Date.now(), server = "agent-wiki" } = {}) {
  if (!dir) return null;
  // A folder in a later format than XenoNet reads isn't parsed: the map
  // keeps its node and says why it shows nothing in it.
  const problem = formatProblem(readMarker(dir));
  if (problem) return { dir, server, pages: [], inbox: [], log: [], problem };
  const pages = [];
  const log = [];
  for (const space of [{ id: null, kind: "machine", name: "local", dir }, ...(await spacesIn(dir))]) {
    const read = await readSpace(space, projects, { root, now });
    pages.push(...read.pages);
    log.push(...read.log);
  }
  const inbox = [];
  for (const f of (await namesIn(path.join(dir, "inbox"))).filter((f) => f.endsWith(".md"))) {
    if (await linked(path.join(dir, "inbox", f))) continue;
    const text = await fsp.readFile(path.join(dir, "inbox", f), "utf8").catch(() => "");
    const { meta, body } = parseFrontmatter(text);
    inbox.push({ id: f.slice(0, -3), title: meta.title || body.split(/\r?\n/).find((l) => l.trim())?.replace(/^#+\s*/, "") || f, app: meta.app || null });
  }
  return { dir, server, pages, inbox, log: log.sort((a, b) => a.ts - b.ts) };
}
