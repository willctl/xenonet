// Just enough TOML to add and take out one MCP server in Grok Build's
// config.toml (~/.grok/config.toml) and leave the rest of the file as it
// is: your other settings, your comments, your blank lines and their
// order. `grok mcp add` rewrites the whole file and drops its comments, so
// XenoNet edits the text itself (engine/connect.js). The Rust has a twin,
// crates/xenonet-engine/src/toml.rs.
//
// XenoNet's server is two tables, [mcp_servers.<name>] and
// [mcp_servers.<name>.env], each line of them a string or a list of
// strings, written as JSON writes them (which TOML reads the same). It
// finds its tables by their headers, takes out the lines from each header
// to the next, and writes its own at the end. A server of that name written
// another way (an inline table in [mcp_servers], dotted keys before any
// header) is one XenoNet doesn't edit: it says so, and leaves the file as
// it is.

// A key as a header or a key line has it: bare, or quoted.
const tomlKey = (k) => (/^[A-Za-z0-9_-]+$/.test(k) ? k : JSON.stringify(k));

// The dotted parts of a header or a key (mcp_servers."xenonet-dev".env),
// unquoted, or null when they don't read as one.
export function keyParts(text) {
  const parts = [];
  const s = text.trim();
  let i = 0;
  while (i < s.length) {
    while (s[i] === " " || s[i] === "\t") i++;
    if (s[i] === '"') {
      const end = s.indexOf('"', i + 1);
      if (end < 0) return null;
      parts.push(s.slice(i + 1, end));
      i = end + 1;
    } else if (s[i] === "'") {
      const end = s.indexOf("'", i + 1);
      if (end < 0) return null;
      parts.push(s.slice(i + 1, end));
      i = end + 1;
    } else {
      const m = /^[A-Za-z0-9_-]+/.exec(s.slice(i));
      if (!m) return null;
      parts.push(m[0]);
      i += m[0].length;
    }
    while (s[i] === " " || s[i] === "\t") i++;
    if (i < s.length) {
      if (s[i] !== ".") return null;
      i++;
      if (i >= s.length) return null;
    }
  }
  return parts.length ? parts : null;
}

const HEADER = /^\s*\[\[?([^\[\]#]+)\]\]?\s*(#.*)?$/;
const KEY = /^\s*([^=#\s][^=#]*?)\s*=\s*(.*)$/;

// The file's lines, each with the table it's in, whether it's a header,
// and its key and value when it has one: { text, header, path, key, value }.
// A line inside a multi-line string (""" or ''') is one of its value's.
export function tomlLines(text) {
  const out = [];
  let path = [];
  let open = null;
  for (const line of String(text).split("\n")) {
    const bare = line.replace(/\r$/, "");
    if (open) {
      out.push({ text: line, header: false, path, key: null, value: null });
      if (bare.split(open).length % 2 === 0) open = null;
      continue;
    }
    const h = HEADER.exec(bare);
    const parts = h ? keyParts(h[1]) : null;
    if (parts) {
      path = parts;
      out.push({ text: line, header: true, path, key: null, value: null });
      continue;
    }
    const m = KEY.exec(bare);
    const key = m ? keyParts(m[1]) : null;
    out.push({ text: line, header: false, path, key, value: key ? m[2] : null });
    for (const q of ['"""', "'''"]) if (key && m[2].split(q).length % 2 === 0) open = q;
  }
  return out;
}

const isServer = (path, name) => path[0] === "mcp_servers" && path[1] === name;

// Whether a file names server `name` in a form XenoNet doesn't edit: a key
// for it outside its own tables.
export function foreignServer(text, name) {
  return tomlLines(text).some((l) => !l.header && l.key && !isServer(l.path, name) && isServer([...l.path, ...l.key], name));
}

// The file without server `name`'s tables (its own and its sub-tables),
// each from its header to the next table's; the rest as it was.
export function withoutServer(text, name) {
  const kept = [];
  let dropping = false;
  for (const l of tomlLines(text)) {
    if (l.header) dropping = isServer(l.path, name);
    if (!dropping) kept.push(l.text);
  }
  return kept.join("\n");
}

// Server `name` as XenoNet writes it: its command and arguments, then its
// environment.
export function serverToml(name, { command, args = [], env = {} }) {
  const head = `mcp_servers.${tomlKey(name)}`;
  return [
    `[${head}]`,
    `command = ${JSON.stringify(String(command))}`,
    `args = ${JSON.stringify(args.map(String)).replace(/","/g, '", "')}`,
    "",
    `[${head}.env]`,
    ...Object.entries(env).map(([k, v]) => `${tomlKey(k)} = ${JSON.stringify(String(v))}`),
  ].join("\n");
}

// The file with server `name` written as given, in place of the one there
// was, at the end after a blank line. Throws when the file names it in a
// form XenoNet doesn't edit.
export function withServer(text, name, server) {
  if (foreignServer(text, name)) throw new Error(`it names mcp_servers.${name} in a form XenoNet doesn't edit`);
  const rest = withoutServer(text, name).replace(/\s+$/, "");
  return `${rest ? `${rest}\n\n` : ""}${serverToml(name, server)}\n`;
}

// A value as XenoNet writes them: a string, or a list of strings, as JSON
// reads them; undefined for any other.
function value(text) {
  try {
    const v = JSON.parse(text.trim());
    return typeof v === "string" || (Array.isArray(v) && v.every((x) => typeof x === "string")) ? v : undefined;
  } catch {
    return undefined;
  }
}

// Server `name` as the file has it: { command, args, env } from its tables
// (a part it doesn't have is null; one it has in a form XenoNet doesn't
// write is undefined), or null when it has none.
export function readServer(text, name) {
  const lines = tomlLines(text).filter((l) => isServer(l.path, name));
  if (!lines.some((l) => l.header && l.path.length === 2)) return null;
  const server = { command: null, args: null, env: null };
  for (const l of lines) {
    if (l.header || !l.key) continue;
    const key = [...l.path.slice(2), ...l.key];
    if (key.length === 1 && (key[0] === "command" || key[0] === "args")) server[key[0]] = value(l.value);
    else if (key.length === 2 && key[0] === "env") (server.env ??= {})[key[1]] = value(l.value);
    else if (key.length === 1 && key[0] === "env") server.env = undefined;
  }
  return server;
}
