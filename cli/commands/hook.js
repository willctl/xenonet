// xenonet hook <app>: what Claude Code's and Codex's hooks run (XenoNet adds
// them when you connect your agents; see core/engine/connect.js). It passes
// the event on to the running XenoNet and prints what XenoNet wants the agent to
// hear, in the hook output format both read. If XenoNet isn't running, it
// prints nothing and the agent carries on. XENONET_PORT says which XenoNet,
// and --via=plugin marks the event as the copy XenoNet's plugin for Claude
// Code sends, which XenoNet ignores while connect's own hook is there.
import { tailedOr, appRefusal } from "../../core/apps.js";
import { createLog, logDir } from "../../core/log.js";
import { appDir } from "../../core/dirs.js";

export const help = `Passes a Claude Code or Codex hook event (JSON on stdin) to XenoNet and
prints XenoNet's answer for the session. Their settings run it, so you don't
need to.`;

export async function run([given], flags) {
  // Codex when no app is named. Another app is refused, quietly: the event
  // goes nowhere, a line in XenoNet's log says why, and the agent carries on.
  const app = tailedOr(given, "codex");
  const port = Number(flags.port) || Number(process.env.XENONET_PORT) || 47840;
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  let out = "";
  if (!app) createLog({ dir: logDir(process.env.XENONET_STATE_DIR || appDir("state")), component: "cli" }).warn("cli.hook", appRefusal("xenonet hook", given));
  else
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/hook/${app}${flags.via === "plugin" ? "?via=plugin" : ""}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: input || "{}",
        signal: AbortSignal.timeout(4000),
      });
      if (res.status === 200) out = await res.text();
    } catch {}
  // Out and gone: the agent waits on this process.
  process.stdout.write(out, () => process.exit(0));
}
