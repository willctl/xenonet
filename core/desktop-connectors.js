// The connectors Claude desktop has set up, by uuid. Each of its session
// records lists the account's remote MCP servers as {uuid, name, url, tools},
// and the uuid is what the tools' names carry (mcp__<uuid>__search). So this
// is the name the person knows the connector by, declared, which beats any
// guess from transcripts (mcp-names.js). It reads the uuid and the name and
// nothing else: the url and the tools stay in the record.
import fs from "node:fs";
import path from "node:path";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
// The newest records say what the account has now; older ones only repeat
// them, and there are hundreds.
const RECENT = 8;
const MAX_NAME = 48;

// A name as one line to show: no control characters, and never a uuid.
export function cleanName(value) {
  if (typeof value !== "string") return null;
  const name = value.replace(/[\x00-\x1f\x7f]/g, " ").replace(/\s+/g, " ").trim().slice(0, MAX_NAME).trim();
  return name && !UUID.test(name) ? name : null;
}

// Every record's file under claude-code-sessions/<account>/<organization>/,
// newest first.
function records(dir) {
  const root = path.join(dir, "claude-code-sessions");
  const found = [];
  const walk = (folder, depth) => {
    for (const ent of fs.readdirSync(folder, { withFileTypes: true })) {
      const full = path.join(folder, ent.name);
      if (ent.isDirectory() && depth < 3 && !ent.name.startsWith("deleted_")) walk(full, depth + 1);
      else if (ent.isFile() && /^local_.+\.json$/.test(ent.name)) {
        try { found.push({ full, mtime: fs.statSync(full).mtimeMs }); } catch {}
      }
    }
  };
  try { walk(root, 0); } catch {}
  return found.sort((a, b) => b.mtime - a.mtime || (a.full < b.full ? -1 : a.full > b.full ? 1 : 0));
}

// The connectors in the newest records, uuid to name. Where records differ,
// the newest says.
export function readConnectors(dir, { limit = RECENT } = {}) {
  const out = new Map();
  if (!dir) return out;
  for (const { full } of records(dir).slice(0, limit)) {
    let list;
    try { list = JSON.parse(fs.readFileSync(full, "utf8"))?.remoteMcpServersConfig; } catch { continue; }
    if (!Array.isArray(list)) continue;
    for (const c of list) {
      const name = cleanName(c?.name);
      if (name && typeof c?.uuid === "string" && UUID.test(c.uuid) && !out.has(c.uuid)) out.set(c.uuid, name);
    }
  }
  return out;
}

// The connectors, read again at most every `ttl` ms: the map asks on every
// refresh, and the records change when a connector is added, not often.
export function createConnectors({ dir, now = Date.now, ttl = 30000, read = readConnectors } = {}) {
  let at = -Infinity;
  let hit = new Map();
  return () => {
    if (dir && now() - at >= ttl) {
      at = now();
      try { hit = read(dir); } catch { hit = new Map(); }
    }
    return hit;
  };
}
