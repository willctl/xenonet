// xenonet doctor: XenoNet's checks (core/doctor), run by the running XenoNet or,
// with none running, here (checkup.js).
import { c, print, json, table, plural } from "../lib/out.js";
import { checkup } from "../../core/doctor/checkup.js";

export const help = `Checks what XenoNet needs: Claude Code and Codex installed and signed in,
the folders XenoNet reads, git and the runtime, XenoNet itself, whether
Claude Code and Codex are connected, and actors: tokens left in repos or
expired, and what XenoNet's observers read. Each problem comes with a fix.

  --only <groups>  only some checks: xenonet, claude, codex, system, agents,
                   actors (comma-separated)
  --json           the results as JSON
  --live           what live delivery needs, and how to start Claude Code
                   sessions that get messages as they're sent

XenoNet runs the same checks every half hour and tells you about new
problems. Exits 1 if a check fails.`;

const MARK = { ok: "✓", info: "·", warn: "!", fail: "✗", skip: "–" };
const PAINT = { ok: "green", info: "blue", warn: "yellow", fail: "red", skip: "gray" };

async function live(flags) {
  const { liveSetup, writeListenSettings, CHANNELS_FROM, LISTEN_SECONDS } = await import("../../core/delivery/setup.js");
  const { config } = await import("../../core/config.js");
  const port = flags.port ?? (flags.dev ? 47842 : flags.demo ? 47841 : 47840);
  let listenFile = null;
  try {
    listenFile = writeListenSettings({ dir: config.stateDir, port });
  } catch {}
  const r = await liveSetup({ port, listenFile });
  if (flags.json) return json(r), 0;
  const ok = (b) => (b ? c.green("✓") : c.yellow("!"));
  print(c.bold("Live delivery"));
  print(`  ${ok(r.claude.channels)} Claude Code ${r.claude.version ?? "not found"}${r.claude.channels ? " has channels" : ` needs ${CHANNELS_FROM.join(".")} or later for channels`}`);
  print(`  ${ok(r.tmux)} tmux ${r.tmux ?? "not found: XenoNet can't type into terminals, which only matters for sessions you allow it to"}`);
  print(`  ${c.blue("·")} OpenCode servers running: ${r.opencode.length ? r.opencode.join(", ") : "none"}`);
  print();
  print("Start Claude Code with XenoNet as a channel, so messages reach the session as they're sent:");
  print(`  ${r.command}`);
  print(c.dim("Claude Code asks you to confirm development channels each time it starts, since XenoNet's isn't on Anthropic's list yet. To make it the default, add to your shell's profile:"));
  print(`  ${r.alias}`);
  print(c.dim("Team and Enterprise organizations have to turn channels on first (channelsEnabled). If a message doesn't arrive, claude --debug logs why, and the session gets it on its next turn."));
  print();
  if (r.listen) {
    print(`Or have a session listen for ${LISTEN_SECONDS / 60} minutes after each turn, which works in headless runs (claude -p) too:`);
    print(`  ${r.listen}`);
    print(c.dim("While it listens, the session waits at the end of its turn, and a message that comes keeps it going. It goes with the channel flag above, if you want both."));
    print();
  }
  print("For OpenCode, its TUI serves what XenoNet posts to only when told where to listen:");
  print(`  ${r.opencodeAlias}`);
  print();
  print(c.dim("OpenCode sessions and tmux panes take messages only once you allow it, per session: xenonet live allow <session>. xenonet live pause stops everything."));
  return r.claude.channels ? 0 : 1;
}

export async function run(_args, flags) {
  if (flags.live) return live(flags);
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
