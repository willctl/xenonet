// A name for a shared system whose id says nothing. Claude desktop reaches
// its connectors (Spotify, PostHog...) by uuid, and nothing on disk maps the
// uuid to the service. What does say it is the server itself, in the
// transcripts: the instructions it hands the session, and its own results.
// Reading them is all this does; it never calls a server or reads a file
// besides the lines it is given.

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const HEX = /^[0-9a-f]{24,}$/i;

// An id that is a random token rather than a name: only these get detected,
// so a server someone named keeps the name they gave it.
export const isOpaque = (id) => typeof id === "string" && (UUID.test(id) || HEX.test(id));

// What the map shows for an opaque id nothing named, in place of 36 characters.
export const shortName = (id) => `Connector ${id.slice(0, 8)}`;

// Hosts that name a platform, not the service on it.
const INFRA = new Set(["amazonaws", "cloudfront", "cloudflare", "googleusercontent", "googleapis", "gstatic", "githubusercontent", "azureedge", "example", "w3", "schema", "claude", "anthropic", "modelcontextprotocol"]);
const SECOND_LEVEL = new Set(["co", "com", "org", "net", "gov", "edu", "ac"]);

// The part of a host that names its owner: open.spotify.com is "spotify",
// news.bbc.co.uk is "bbc". Null for an address or a name that can't be one.
export function labelOf(host) {
  const parts = String(host).toLowerCase().split(".").filter(Boolean);
  if (parts.length < 2 || parts.every((p) => /^\d+$/.test(p))) return null;
  let i = parts.length - 2;
  if (parts.length > 2 && parts[i + 1].length === 2 && SECOND_LEVEL.has(parts[i])) i--;
  const label = parts[i];
  return /^[a-z][a-z0-9]{2,23}$/.test(label) && !INFRA.has(label) ? label : null;
}

const WORD = "[A-Z][A-Za-z0-9]*";
const GENERIC_LEAD = new Set(["Note", "Notes", "Important", "Usage", "Tool", "Tools", "Use", "Warning", "Instructions", "Rules", "Overview", "Server", "You", "This", "The"]);
const GENERIC_TAIL = new Set(["Tool", "Tools", "MCP", "Server", "Connector", "Integration", "API", "Plugin"]);

// The name an instructions text opens with: its heading ("# PostHog `exec`
// covers"), or the words before a colon ("Claude Docs: living docs..."). Null
// when it opens with a sentence, which says what to do, not who it is.
export function leadOf(text) {
  const line = String(text).split("\n").find((l) => l.trim())?.trim() ?? "";
  let words;
  if (line.startsWith("#")) {
    words = [];
    for (const w of line.replace(/^#+\s*/, "").split(/\s+/)) {
      if (!new RegExp(`^${WORD}$`).test(w) || words.length === 3) break;
      words.push(w);
    }
  } else {
    const m = new RegExp(`^(${WORD}(?: ${WORD}){0,2})\\s*[:—–-]\\s`).exec(line);
    words = m ? m[1].split(" ") : [];
  }
  while (words.length && GENERIC_TAIL.has(words.at(-1))) words.pop();
  if (!words.length || GENERIC_LEAD.has(words[0])) return null;
  const name = words.join(" ");
  return name.length >= 3 && name.length <= 28 ? name : null;
}

const MIN_PROSE = 3;
const MAX_LABELS = 24;
const MAX_FORMS = 4;
const URL_RE = /https?:\/\/([A-Za-z0-9.-]+)[^\s"'<>)\]\\]*/g;

// The name the evidence comes to, and where from, or null. Instructions that
// open with a name are the strongest. Failing that, a service's own name shows
// up in its own results, in prose and in the links to it; its CDN's name shows
// up only in links, so a domain counts only when its label is also a word in
// the text around the links.
export function detect(ev) {
  const lead = [...(ev?.lead ?? [])].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))[0];
  if (lead) return { name: lead[0], source: "instructions" };
  let best = null;
  for (const [label, c] of ev?.labels ?? []) {
    if (c.prose < MIN_PROSE || c.urls < 1) continue;
    const score = c.prose + c.urls;
    if (!best || score > best.score || (score === best.score && label < best.label)) best = { label, c, score };
  }
  if (!best) return null;
  // The spelling it was written with, when it has capitals inside ("PostHog").
  const forms = [...best.c.forms].filter(([f]) => f !== f.toLowerCase() && f !== f.toUpperCase()).sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  return { name: forms[0]?.[0] ?? best.label[0].toUpperCase() + best.label.slice(1), source: "results" };
}

// The resources with the name Claude desktop declares for each connector
// (desktop-connectors.js) in place of what the lines suggested.
export function withDeclared(resources, declared) {
  return resources.map((r) => (declared.has(r.server) ? { ...r, name: declared.get(r.server), nameSource: "declared" } : r));
}

// What the transcripts' lines show of each opaque server, line by line.
export class ServerNames {
  #servers = new Map();
  #calls = new Map();

  #evidence(server) {
    let ev = this.#servers.get(server);
    if (!ev) this.#servers.set(server, (ev = { lead: new Map(), labels: new Map() }));
    return ev;
  }

  // A Claude Code transcript's line, parsed. `serverOf` says which server a
  // tool's name is of, as live.js does.
  observe(raw, serverOf) {
    if (!raw || typeof raw !== "object") return;
    if (raw.type === "attachment" && raw.attachment?.type === "mcp_instructions_delta") {
      for (const block of Array.isArray(raw.attachment.addedBlocks) ? raw.attachment.addedBlocks : []) {
        const m = /^## (.+)\n([\s\S]*)$/.exec(String(block));
        const lead = m && isOpaque(m[1].trim()) ? leadOf(m[2]) : null;
        if (lead) {
          const ev = this.#evidence(m[1].trim());
          ev.lead.set(lead, (ev.lead.get(lead) ?? 0) + 1);
        }
      }
    }
    const content = raw.message?.content;
    if (!Array.isArray(content)) return;
    for (const b of content) {
      if (b?.type === "tool_use") {
        const server = serverOf(b.name);
        if (!server || !isOpaque(server) || typeof b.id !== "string") continue;
        this.#calls.set(b.id, server);
        if (this.#calls.size > 200) this.#calls.delete(this.#calls.keys().next().value);
      } else if (b?.type === "tool_result" && this.#calls.has(b.tool_use_id)) {
        const server = this.#calls.get(b.tool_use_id);
        this.#calls.delete(b.tool_use_id);
        const text = typeof b.content === "string" ? b.content : Array.isArray(b.content) ? b.content.map((x) => (typeof x?.text === "string" ? x.text : "")).join("\n") : "";
        this.#results(server, text.slice(0, 40000));
      }
    }
  }

  #results(server, text) {
    const urls = new Map();
    for (const m of text.matchAll(URL_RE)) {
      const label = labelOf(m[1]);
      if (label) urls.set(label, (urls.get(label) ?? new Set()).add(m[0]));
    }
    if (!urls.size) return;
    const prose = text.replace(URL_RE, " ");
    const ev = this.#evidence(server);
    for (const [label, set] of [...urls].slice(0, 16)) {
      const c = ev.labels.get(label) ?? { urls: 0, prose: 0, forms: new Map() };
      c.urls += set.size;
      for (const m of prose.matchAll(new RegExp(`(?<![a-z0-9])${label}(?![a-z0-9])`, "gi"))) {
        c.prose++;
        if (c.forms.has(m[0]) || c.forms.size < MAX_FORMS) c.forms.set(m[0], (c.forms.get(m[0]) ?? 0) + 1);
      }
      ev.labels.set(label, c);
    }
    if (ev.labels.size > MAX_LABELS) {
      const weakest = [...ev.labels].sort((a, b) => a[1].prose + a[1].urls - (b[1].prose + b[1].urls))[0][0];
      ev.labels.delete(weakest);
    }
  }

  // The server's name and where it came from ("instructions" or "results"),
  // or null while the lines haven't said.
  name(server) {
    const ev = this.#servers.get(server);
    return ev ? detect(ev) : null;
  }
}
