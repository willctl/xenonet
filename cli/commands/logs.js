// xenonet logs: what went wrong in XenoNet lately, from its error log
// (core/log.js), read from the files themselves, so it works with XenoNet
// stopped or failing to start.
import { c, print, json } from "../lib/out.js";
import { appDir } from "../../core/dirs.js";
import { logDir, readLogs, summarize, LEVELS, COMPONENTS } from "../../core/log.js";

export const help = `What went wrong in XenoNet lately: the background service, the desktop app
and the xenonet command, each line with a code that says what it is and the
fix when there's one. Home folders show as ~ and anything like a key is
taken out, so it's safe to paste into an issue.

  --errors          errors only (default: errors and warnings)
  --all             info lines too
  --since <time>    how far back: 30m, 6h, 2d (default 24h)
  --component <c>   service, desktop or cli
  --code <code>     codes starting with this, such as doctor. or service.port
  --summary         each code once, with how often and when last
  --dev             XenoNet Dev's log, not the installed XenoNet's
  --json            as JSON lines`;

const UNITS = { m: 60e3, h: 3600e3, d: 86400e3 };
export const sinceOf = (text, now = Date.now()) => {
  const m = /^(\d+)\s*([mhd])$/.exec(String(text ?? "24h").trim());
  if (!m) throw new Error(`--since takes a time such as 30m, 6h or 2d, not ${text}`);
  return now - Number(m[1]) * UNITS[m[2]];
};
const PAINT = { error: c.red, warn: c.yellow, info: c.dim };
const when = (at) => new Date(at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" });

export async function run(_args, flags) {
  const dir = logDir(process.env.XENONET_STATE_DIR || appDir("state", flags.dev ? { name: "XenoNet Dev" } : {}));
  if (flags.component && !COMPONENTS.includes(flags.component)) throw new Error(`--component is ${COMPONENTS.join(", ")}`);
  const level = flags.errors ? "error" : flags.all ? "info" : "warn";
  const entries = readLogs(dir, { since: sinceOf(flags.since), level, component: flags.component || null, code: flags.code || null, limit: 2000 });
  if (flags.summary) {
    const rows = summarize(entries);
    if (flags.json) return print(rows.map((r) => JSON.stringify(r)).join("\n"));
    if (!rows.length) return print(c.dim(`Nothing at ${level} or worse since ${when(sinceOf(flags.since))}.`));
    for (const r of rows) print(`${PAINT[r.level](r.level.padEnd(5))} ${c.bold(r.code)} ${c.dim(`${r.component} · ${r.count}× · last ${when(r.last)}`)}\n  ${r.message}${r.fix ? `\n  ${c.cyan("fix:")} ${r.fix}` : ""}`);
    return;
  }
  if (flags.json) return print(entries.map((e) => JSON.stringify(e)).join("\n"));
  if (!entries.length) return print(c.dim(`Nothing at ${level} or worse since ${when(sinceOf(flags.since))}. (${LEVELS.join(", ")})`));
  for (const e of entries.slice().reverse()) print(`${c.dim(when(e.at))} ${PAINT[e.level](e.level.padEnd(5))} ${c.dim(e.component)} ${c.bold(e.code)}${e.repeats ? c.dim(` (+${e.repeats})`) : ""}\n  ${e.message}${e.fix ? `\n  ${c.cyan("fix:")} ${e.fix}` : ""}`);
  void json;
}
