// Output for the xenonet command: color when a person is reading (a terminal,
// without NO_COLOR), plain text otherwise, and JSON for scripts (--json).
const tty = process.stdout.isTTY && !process.env.NO_COLOR && process.env.TERM !== "dumb";
let color = tty;
export const setColor = (on) => (color = on && tty);

const paint = (code) => (s) => (color ? `\x1b[${code}m${s}\x1b[0m` : String(s));
export const c = {
  bold: paint("1"),
  dim: paint("2"),
  red: paint("31"),
  green: paint("32"),
  yellow: paint("33"),
  blue: paint("34"),
  magenta: paint("35"),
  cyan: paint("36"),
  gray: paint("90"),
};

export const width = () => Math.max(60, Math.min(process.stdout.columns || 100, 160));

// Characters as they show on screen, for padding.
const visible = (s) => String(s).replace(/\x1b\[[0-9;]*m/g, "");
export const pad = (s, n) => s + " ".repeat(Math.max(0, n - [...visible(s)].length));
export function clip(s, n) {
  const chars = [...String(s ?? "")];
  return chars.length > n ? `${chars.slice(0, Math.max(0, n - 1)).join("")}…` : chars.join("");
}

// Rows of cells as aligned columns. The last column takes what's left of
// the terminal: cut to fit, or with `wrap`, wrapped onto more lines under
// itself. `indent` shifts the whole table right.
export function table(rows, { gap = 2, indent = 0, wrap = false } = {}) {
  if (!rows.length) return "";
  const cols = Math.max(...rows.map((r) => r.length));
  const widths = Array.from({ length: cols - 1 }, (_, i) => Math.max(...rows.map((r) => [...visible(r[i] ?? "")].length)));
  const used = indent + widths.reduce((a, b) => a + b + gap, 0);
  const room = Math.max(16, width() - used);
  const lead = " ".repeat(indent);
  return rows
    .map((r) => {
      const head = r.slice(0, cols - 1).map((cell = "", i) => pad(cell, widths[i] + gap)).join("");
      const last = r[cols - 1] ?? "";
      if (!wrap) return lead + head + (visible(last) === last ? clip(last, room) : last);
      const [first, ...more] = wrapText(last, room);
      return [lead + head + first, ...more.map((l) => lead + " ".repeat(used - indent) + l)].join("\n");
    })
    .join("\n");
}

// Text broken into lines of at most `n` characters, at spaces where it can.
// A cell painted in one color keeps it on every line.
export function wrapText(s, n) {
  const str = String(s);
  const m = /^(\x1b\[[0-9;]*m)([^\x1b]*)\x1b\[0m$/.exec(str);
  if (!m && str.includes("\x1b")) return [str];
  const [open, text] = m ? [m[1], m[2]] : ["", str];
  const lines = [];
  for (const para of text.split("\n")) {
    let line = "";
    for (const word of para.split(" ")) {
      if (!line) line = word;
      else if ([...line].length + 1 + [...word].length <= n) line += ` ${word}`;
      else (lines.push(line), (line = word));
      while ([...line].length > n) (lines.push([...line].slice(0, n).join("")), (line = [...line].slice(n).join("")));
    }
    lines.push(line);
  }
  return open ? lines.map((l) => `${open}${l}\x1b[0m`) : lines;
}

export const json = (value) => process.stdout.write(JSON.stringify(value, null, 2) + "\n");
export const print = (s = "") => process.stdout.write(`${s}\n`);
export { plural } from "../../core/units.js";

export function ago(ts, now = Date.now()) {
  const s = Math.round((now - ts) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

// A failure the command explains itself: printed without a stack trace.
export class Fail extends Error {
  constructor(message, code = 1) {
    super(message);
    this.code = code;
  }
}
