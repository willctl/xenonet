// xenonet drift: what's changed in the other apps' data XenoNet reads, by
// source, since you last looked (core/drift.js, core/vendor.js): from a
// running XenoNet, or with none running, from drift.json in the cache folder.
// Then it marks that seen, which quiets the doctor's contracts.drift check.
// Only this and the console mark drift seen: npm run contracts reports it
// too, but agents run that, and you'd miss what they saw. With --exit-code
// it's left unmarked, so a script goes on exiting 1 until you've looked.
import path from "node:path";
import { c, print, json } from "../lib/out.js";
import { find, get, post, nameOf } from "../../core/client.js";
import { DriftLog, DRIFT_FILE } from "../../core/drift.js";
import { driftReport, reportLines } from "../../core/vendor.js";

export const help = `What's changed in the data XenoNet reads from Claude Code, Codex, Claude
desktop and gh since you last looked, by source: a field gone or of
another type, a line that isn't JSON or a version XenoNet's schemas weren't
made for (changes, which can break what XenoNet shows), and a field or a kind
of record they don't list yet (additions). Then it marks them seen, which
quiets the doctor until something new turns up.

  --all        every difference XenoNet keeps, not only the new ones
  --json       the report as JSON
  --exit-code  exit 1 when there's drift new since you last looked, for
               scripts, and leave it new: plain xenonet drift or the
               console marks it seen

XenoNet keeps them in drift.json in its cache folder, as shapes, never
values. With no XenoNet running, xenonet drift reads that file.`;

const PAINT = { name: c.bold, changed: c.yellow, added: c.cyan, dim: c.dim };

export async function run(_args, flags) {
  const all = !!flags.all;
  const mark = !flags.exitCode;
  const s = await find(flags);
  let report = null;
  if (s) {
    try {
      report = await get(s, `/api/drift${all ? "?all=1" : ""}`);
    } catch (err) {
      // A XenoNet from before the route: its drift.json is read instead.
      if (err.status !== 404) throw err;
    }
  }
  if (report) report.by = nameOf(s);
  else report = await fromFile(all, flags, mark);

  if (flags.json) json(report);
  else {
    print(report.by ? `${c.bold(report.by)} ${c.dim(`· drift kept in ${report.file}`)}` : c.dim(`XenoNet isn't running, so this is the drift it kept in ${report.file}`));
    print();
    // A running XenoNet says what it has read since it started; from the
    // file nothing was read, so only the sources with drift show.
    const lines = reportLines(report, report.by ? { unread: "nothing since XenoNet started", paint: PAINT } : { unread: null, quiet: true, paint: PAINT });
    for (const line of lines) print(line);
    print();
  }
  // Seen once it's shown, up to when it was made: what turned up since
  // stays new. (The file was marked as it was read.)
  if (report.by && mark) await post(s, "/api/drift/seen", { at: report.at });
  if (!flags.json) print(c.dim(!report.new ? "Nothing new since you last looked." : mark ? "Marked seen: the doctor says nothing more of these until something new turns up." : "Left new (--exit-code): plain xenonet drift or the console marks it seen."));
  // New as of the report, before it was marked seen.
  return flags.exitCode && report.new ? 1 : 0;
}

// The report from drift.json itself, which is then marked seen there
// (with `mark`): in the cache folder of the XenoNet asked about (XenoNet Dev
// keeps its own).
async function fromFile(all, flags, mark) {
  const [{ config }, { instanceDir, standsFor }] = await Promise.all([import("../../core/config.js"), import("../../core/workspace.js")]);
  const name = config.demo ? null : standsFor(flags);
  const log = new DriftLog().open(path.join(name ? instanceDir("cache", name) : config.cacheDir, DRIFT_FILE));
  const report = driftReport({ log, all, readers: [], home: config.homeDir });
  if (mark) {
    log.looked(report.at);
    log.flush();
  }
  return { ...report, by: null };
}
