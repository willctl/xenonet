// xenonet doctor: XenoNet's checks (core/doctor), run by the running XenoNet or,
// with none running, here (checkup.js).
import { c, print, json, table, plural } from "../lib/out.js";
import { checkup } from "../../core/doctor/checkup.js";

export const help = `Checks what XenoNet needs: Claude Code and Codex installed and signed in,
the folders XenoNet reads, git and the runtime, XenoNet itself, whether your
SIs are connected, and actors: tokens left in repos or expired, and
what XenoNet's observers read. Each problem comes with a fix.

  --only <groups>  only some checks: xenonet, claude, codex, system, agents,
                   actors (comma-separated)
  --json           the results as JSON

XenoNet runs the same checks every half hour and tells you about new
problems. Exits 1 if a check fails.`;

const MARK = { ok: "✓", info: "·", warn: "!", fail: "✗", skip: "–" };
const PAINT = { ok: "green", info: "blue", warn: "yellow", fail: "red", skip: "gray" };

export async function run(_args, flags) {
  const only = typeof flags.only === "string" ? flags.only.split(",").map((s) => s.trim()).filter(Boolean) : null;
  const result = await checkup({ ...flags, only });
  const by = result.by;
  const count = (status) => result.results.filter((r) => r.status === status).length;
  const fails = count("fail");
  if (flags.json) {
    json(result);
    return fails ? 1 : 0;
  }

  // One width for every group's titles, so the details line up.
  const titles = Math.max(...result.results.map((r) => r.title.length));
  for (const group of [...new Set(result.results.map((r) => r.group))]) {
    print(c.bold(group));
    const rows = [];
    for (const r of result.results.filter((x) => x.group === group)) {
      const paint = c[PAINT[r.status]];
      rows.push([paint(MARK[r.status]), r.title.padEnd(titles), r.status === "skip" ? c.dim(r.detail) : r.detail]);
      if (r.fix && r.status !== "ok" && r.status !== "skip") rows.push(["", "", c.dim(`↳ ${r.fix}`)]);
    }
    print(table(rows, { indent: 2, wrap: true }));
    print();
  }
  const warns = count("warn");
  const verdict = fails || warns ? [fails && c.red(plural(fails, "problem")), warns && c.yellow(plural(warns, "warning"))].filter(Boolean).join(" and ") : c.green("Everything XenoNet needs is in place");
  print(`${verdict} ${c.dim(`· ${plural(result.results.length, "check")} in ${(result.ms / 1000).toFixed(1)} s${by ? `, run by ${by}` : ""}`)}`);
  return fails ? 1 : 0;
}
