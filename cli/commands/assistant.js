// xenonet assistant: your remote actors' connections, from your own terminal
// (docs/guide/cloud-assistants.md). Pairing one, and turning cloud access
// back on, happen in XenoNet's window. Revoking one and turning them all off
// work with XenoNet stopped too: they leave a tombstone in XenoNet's state
// folder, which every version of XenoNet honors, and XenoNet is told as well
// when it's running.
import { c, print, json, table, ago, Fail } from "../lib/out.js";
import { find, get, post, nameOf } from "../../core/client.js";
import { appDir } from "../../core/dirs.js";
import { instanceDir } from "../../core/workspace.js";
import { assistantDir, markDisabled, markRevoked, deleteCredential, isDisabled, markedRevoked } from "../../core/assistant/credentials.js";
import { GRANT_ID } from "../../core/schema/assistant.js";

export const help = `Remote actors, such as an OpenAI dot, read what's going on here through
connections you pair in XenoNet's window (Actors, then Remote connections).
Each reads only the projects you picked, until its grant expires.

  xenonet assistant                  whether cloud access is on, and each
                                   connection: its actor, projects, expiry
                                   and when it was last used
  xenonet assistant audit [--grant <id>] [--limit <n>]
                                   what was read lately: times, operations,
                                   result codes and sizes, never content
  xenonet assistant revoke <grant>   revokes one connection, for good
  xenonet assistant disable          turns every connection off at once

revoke and disable work with XenoNet stopped. To turn access on again, or to
pair a connection, use XenoNet's window. Neither ever deletes what a remote
actor has already read: revoke its endpoint with its provider too.

  --dev    XenoNet Dev's connections rather than XenoNet's
  --json   answers as JSON`;

// XenoNet's state folder, for the tombstones: XenoNet Dev's with --dev.
const stateDir = (flags) => (flags.dev ? instanceDir("state", "XenoNet Dev") : process.env.XENONET_STATE_DIR || appDir("state"));

const STATE = { active: c.green("active"), expired: c.yellow("expired"), revoked: c.dim("revoked"), disabled: c.yellow("off") };

function show(data, s) {
  print(`${c.bold(nameOf(s))} ${c.dim("·")} cloud access is ${data.disabled ? c.yellow("off") : c.green("on")} ${c.dim(`· device ${data.device.id}`)}`);
  const d = data.diagnostics;
  if (d) print(c.dim(`service ${d.service_version}, interface ${d.assistant_api} · endpoint ${d.endpoint?.listening ? `listening, ${d.endpoint.adapters} adapter${d.endpoint.adapters === 1 ? "" : "s"} connected` : "not listening"} · ${d.sources.map((x) => `${x.source} ${x.state}`).join(", ")}`));
  if (!data.connections.length) return print(c.dim("No remote connections. Pair one in XenoNet's window: Actors, then Remote connections."));
  print("");
  print(
    table(
      data.connections.map((x) => [
        STATE[x.state] ?? x.state,
        x.grant_id,
        x.actor_id,
        x.label,
        `${x.projects.length} project${x.projects.length === 1 ? "" : "s"}${x.labels ? ", labels" : ""}${x.data_mode === "fixture" ? ", made-up data" : ""}`,
        x.state === "active" ? `until ${x.expires_at.slice(0, 10)}` : x.revoked_at ? `revoked ${x.revoked_at.slice(0, 10)}` : "",
        x.last_used_at ? `used ${ago(Date.parse(x.last_used_at))}` : c.dim("never used"),
      ]),
    ),
  );
}

export async function run(args = [], flags = {}) {
  const [what = "list", grant] = args;
  const s = await find(flags);
  const out = (data, words) => (flags.json ? json(data) : print(words));

  if (what === "list" || what === "status") {
    if (!s) {
      const dir = assistantDir(stateDir(flags));
      const data = { running: false, disabled: isDisabled(dir), revoked: markedRevoked(dir) };
      return out(data, `XenoNet isn't running. Cloud access is ${data.disabled ? "off" : "on, for its active grants,"} as XenoNet's state folder says${data.revoked.length ? `; ${data.revoked.length} revoked` : ""}.`);
    }
    const data = await get(s, "/api/assistant");
    return flags.json ? json(data) : show(data, s);
  }

  if (what === "audit") {
    if (!s) throw new Fail("XenoNet isn't running, so its log can't be read: open XenoNet first.");
    if (flags.grant && !GRANT_ID.test(String(flags.grant))) throw new Fail("--grant takes a grant's id (gr_…).", 2);
    const { records } = await get(s, `/api/assistant/audit?limit=${Number(flags.limit) || 50}${flags.grant ? `&grant=${flags.grant}` : ""}`);
    if (flags.json) return json(records);
    if (!records.length) return print(c.dim("Nothing read in the last seven days."));
    return print(table(records.map((r) => [r.at.slice(0, 19).replace("T", " "), r.kind === "owner" ? c.cyan("you") : r.actor ?? "", r.operation, r.result === "ok" ? c.green("ok") : c.yellow(r.result), r.bytes != null ? `${r.bytes} B` : "", r.latency_ms != null ? `${r.latency_ms} ms` : ""])));
  }

  if (what === "disable") {
    // The tombstone first, so it holds whatever happens next.
    markDisabled(assistantDir(stateDir(flags)));
    if (s) await post(s, "/api/assistant/disable").catch((err) => print(c.yellow(`XenoNet didn't take it at once (${err.message}); it will within two seconds.`)));
    return out({ disabled: true }, `Cloud access is off${s ? "" : ", for when XenoNet starts"}. Every remote connection stopped; turn it on again in XenoNet's window. Revoke each endpoint with its provider too: what was read already isn't taken back.`);
  }

  if (what === "revoke") {
    if (!GRANT_ID.test(String(grant))) throw new Fail("xenonet assistant revoke takes a grant's id (gr_…), as xenonet assistant lists them.", 2);
    const dir = assistantDir(stateDir(flags));
    markRevoked(dir, grant);
    deleteCredential(dir, grant);
    if (s) await post(s, `/api/assistant/${grant}/revoke`).catch((err) => { if (err.status !== 404) throw err; });
    return out({ revoked: grant }, `Revoked ${grant}. Revoke its endpoint with its provider too: what was read already isn't taken back.`);
  }

  if (what === "enable") throw new Fail("Turn cloud access on again in XenoNet's window: Actors, then Remote connections. It widens access, so it's the window's to do.");
  if (what === "pair") throw new Fail("Pair a remote actor in XenoNet's window: Actors, then Remote connections. It shows exactly what would leave this computer first.");
  throw new Fail(`no xenonet assistant ${what}. xenonet assistant --help lists what there is.`, 2);
}
