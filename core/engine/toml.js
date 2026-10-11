// Just enough TOML to add and take out one MCP server in Grok Build's
// config.toml (~/.grok/config.toml) without touching anything else in it:
// your other settings, your comments, your blank lines and the order of it
// all stay byte for byte. `grok mcp add` rewrites the whole file and drops
// its comments, so XenoNet edits the text itself (engine/connect.js).
//
// A server is a table, [mcp_servers.<name>], with its sub-tables
// ([mcp_servers.<name>.env]); XenoNet finds those by their headers, takes
// out the lines from each header to the next, and writes its own at the
// end. A server written some other way (an inline table in [mcp_servers],
// dotted keys before any header) is one XenoNet doesn't edit: it says so,
// and leaves the file as it is.

// A key as a header or a key/value line has it: bare, or quoted.
export const tomlKey = (k) => (/^[A-Za-z0-9_-]+$/.test(k) ? k : JSON.stringify(k));
// A string as TOML writes it: JSON's escapes are TOML's basic string's.
const tomlString = (s) => JSON.stringify(String(s));

// The dotted parts of a header or key ("mcp_servers.\"xenonet-dev\".env"),
// unquoted, or null when they don't parse.
export function keyParts(text) {
  const parts = [];
  let i = 0;
  const s = text.trim();
  while (i < s.length) {
    while (s[i] === " " || s[i] === "\t") i++;
    if (s[i] === '"') {
      let j = i + 1, out = "";
      for (; j < s.length && s[j] !== '"'; j++) {
        if (s[j] === "\\") {
          const n = s[++j];
          out += n === "n" ? "\n" : n === "t" ? "\t" : n === "u" ? String.fromCharCode(parseInt(s.slice(j + 1, j + 5), 16)) : n;
          if (n === "u") j += 4;
        } else out += s[j];
      }
      if (j >= s.length) return null;
      parts.push(out);
      i = j + 1;
    } else if (s[i] === "'") {
      const j = s.indexOf("'", i + 1);
      if (j < 0) return null;
      parts.push(s.slice(i + 1, j));
      i = j + 1;
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
    }
  }
  return parts.length ? parts : null;
}

// The file's lines, each with the table it's in and whether it's a
// header: { text, header, path, array }. Strings (multi-line ones too) and
// values over several lines (an array, an inline table) are followed, so a
// "[" at a line's start inside one isn't taken for a header.
export function tomlLines(text) {
  const out = [];
  let path = [];
  const st = { quote: null, depth: 0 };
  for (const line of String(text).split("\n")) {
    const inside = !!st.quote || st.depth > 0;
    const header = inside ? null : /^\s*(\[\[?)\s*([^\]#]+?)\s*\]\]?\s*(#.*)?$/.exec(line.replace(/\r$/, ""));
    const parts = header ? keyParts(header[2]) : null;
    if (parts) {
      path = parts;
      out.push({ text: line, header: true, path, array: header[1] === "[[" });
      continue;
    }
    out.push({ text: line, header: false, path });
    scan(line, st, inside);
  }
  return out;
}

// Follows a line's strings and brackets: what's still open at its end (a
// multi-line string's quotes, how deep in brackets) carries on to the
// next. A line that starts a value starts after its "=".
function scan(line, st, inside) {
  let i = 0;
  if (!inside) {
    const eq = /^\s*[^=#\s][^=#]*?=/.exec(line);
    if (!eq) return;
    i = eq[0].length;
  }
  for (; i < line.length; i++) {
    if (st.quote) {
      // Basic strings escape; literal ones don't.
      if (st.quote[0] === '"' && line[i] === "\\") i++;
      else if (line.startsWith(st.quote, i)) {
        i += st.quote.length - 1;
        st.quote = null;
      }
      continue;
    }
    const c = line[i];
    if (c === "#") break;
    if (c === '"' || c === "'") {
      st.quote = line.startsWith(c.repeat(3), i) ? c.repeat(3) : c;
      i += st.quote.length - 1;
    } else if (c === "[" || c === "{") st.depth++;
    else if (c === "]" || c === "}") st.depth = Math.max(0, st.depth - 1);
  }
  // A one-line string can't go on to the next line.
  if (st.quote?.length === 1) st.quote = null;
}

const isServer = (path, name) => path[0] === "mcp_servers" && path[1] === name;

// Whether a file names server `name` in a form XenoNet doesn't edit: an
// inline table or dotted key under [mcp_servers], or a dotted key outside
// any table.
export function foreignServer(text, name) {
  for (const l of tomlLines(text)) {
    if (l.header) continue;
    const m = /^\s*([^=#\s][^=#]*?)\s*=/.exec(l.text);
    const key = m ? keyParts(m[1]) : null;
    if (!key) continue;
    const full = [...l.path, ...key];
    if (full[0] === "mcp_servers" && full[1] === name && !isServer(l.path, name)) return true;
  }
  return false;
}

// The file without server `name`'s tables (its own and its sub-tables'),
// each from its header to the next table's; the rest as it was. The same
// text when there's no such table.
export function withoutServer(text, name) {
  const kept = [];
  let dropping = false;
  for (const l of tomlLines(text)) {
    if (l.header) dropping = isServer(l.path, name);
    if (!dropping) kept.push(l.text);
  }
  return kept.join("\n");
}

// Server `name` as XenoNet writes it: its command, its arguments and its
// environment, in one table.
export function serverToml(name, { command, args = [], env = {} }) {
  const envText = Object.entries(env).map(([k, v]) => `${tomlKey(k)} = ${tomlString(v)}`).join(", ");
  return [`[mcp_servers.${tomlKey(name)}]`, `command = ${tomlString(command)}`, `args = [${args.map(tomlString).join(", ")}]`, `env = { ${envText} }`].join("\n");
}

// The file with server `name` written as given, in place of the one there
// was: at the end, after a blank line. Throws when the file names it in a
// form XenoNet doesn't edit.
export function withServer(text, name, server) {
  if (foreignServer(text, name)) throw new Error(`it names mcp_servers.${name} in a form XenoNet doesn't edit`);
  const rest = withoutServer(text, name).replace(/\s*$/, "");
  return `${rest ? `${rest}\n\n` : ""}${serverToml(name, server)}\n`;
}

// ---------- reading a server back ----------

// Server `name` as the file has it: { command, args, env } from its tables
// (a missing part is null), or null when there's none. Strings, arrays of
// strings and inline tables of strings are read, over several lines too;
// anything else XenoNet doesn't write is left as undefined.
export function readServer(text, name) {
  const lines = tomlLines(text);
  if (!lines.some((l) => l.header && l.path.length === 2 && isServer(l.path, name))) return null;
  const server = { command: null, args: null, env: null };
  let pending = null; // a value that goes on to the next lines
  for (const l of lines) {
    if (l.header || !isServer(l.path, name)) {
      pending = null;
      continue;
    }
    const sub = l.path.slice(2);
    if (pending) {
      pending.text += `\n${l.text}`;
      if (!balanced(pending.text)) continue;
      assign(server, pending.key, value(pending.text));
      pending = null;
      continue;
    }
    const m = /^\s*([^=#\s][^=#]*?)\s*=\s*(.*)$/.exec(l.text);
    const key = m ? keyParts(m[1]) : null;
    if (!key) continue;
    const full = [...sub, ...key];
    if (!balanced(m[2])) {
      pending = { key: full, text: m[2] };
      continue;
    }
    assign(server, full, value(m[2]));
  }
  return server;
}

function assign(server, key, v) {
  if (key.length === 1 && (key[0] === "command" || key[0] === "args")) server[key[0]] = v;
  else if (key.length === 1 && key[0] === "env") server.env = v && typeof v === "object" && !Array.isArray(v) ? v : undefined;
  else if (key.length === 2 && key[0] === "env") (server.env ??= {})[key[1]] = v;
}

// Whether a value's brackets and braces close, outside its strings.
function balanced(text) {
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"' || c === "'") {
      const end = endOfString(text, i);
      if (end < 0) return false;
      i = end;
    } else if (c === "#") break;
    else if (c === "[" || c === "{") depth++;
    else if (c === "]" || c === "}") depth--;
  }
  return depth <= 0;
}

function endOfString(text, i) {
  const q = text[i];
  for (let j = i + 1; j < text.length; j++) {
    if (q === '"' && text[j] === "\\") j++;
    else if (text[j] === q) return j;
  }
  return -1;
}

// A value: a string, an array or inline table of them (nested), true or
// false, or a number; undefined for anything else.
function value(text) {
  let i = 0;
  const s = text;
  const ws = () => {
    while (i < s.length && /[\s,]/.test(s[i])) i++;
    if (s[i] === "#") while (i < s.length && s[i] !== "\n") i++;
    if (i < s.length && /[\s,#]/.test(s[i])) ws();
  };
  const one = () => {
    ws();
    const c = s[i];
    if (c === '"' || c === "'") {
      const end = endOfString(s, i);
      if (end < 0) throw new Error("open string");
      const raw = s.slice(i, end + 1);
      i = end + 1;
      return c === '"' ? JSON.parse(raw.replace(/\\U([0-9A-Fa-f]{8})/g, (_, h) => String.fromCodePoint(parseInt(h, 16)))) : raw.slice(1, -1);
    }
    if (c === "[") {
      i++;
      const out = [];
      for (ws(); s[i] !== "]"; ws()) {
        if (i >= s.length) throw new Error("open array");
        out.push(one());
      }
      i++;
      return out;
    }
    if (c === "{") {
      i++;
      const out = {};
      for (ws(); s[i] !== "}"; ws()) {
        if (i >= s.length) throw new Error("open table");
        const m = /^([^=]+?)\s*=/.exec(s.slice(i));
        const key = m ? keyParts(m[1]) : null;
        if (!key || key.length !== 1) throw new Error("bad key");
        i += m[0].length;
        out[key[0]] = one();
      }
      i++;
      return out;
    }
    const m = /^(true|false|[-+]?\d[\d_]*(\.\d+)?)/.exec(s.slice(i));
    if (!m) throw new Error("not a value XenoNet reads");
    i += m[0].length;
    return m[1] === "true" ? true : m[1] === "false" ? false : Number(m[1].replace(/_/g, ""));
  };
  try {
    return one();
  } catch {
    return undefined;
  }
}
