// What XenoNet can tell someone helping you, safe to share (xenonet diagnose,
// the doctor panel's Copy diagnostics): its versions and which XenoNet it
// is, the doctor's findings that aren't ok, its errors from the last day
// by code, where its hooks point, its launcher, and which XENONET_ settings
// are set. Never a transcript, a session's title, a project's name, an
// identity, a token or a key: what's here is shapes and codes, and the
// home folder is ~. It's scrubbed as it's made and checked once more at
// the end, and refused if anything private is left (unsafe, core/log.js).
//
// With --ask, xenonet diagnose hands it to an agent on this computer: a
// Codex or Claude Code session, in a folder of its own, that reads it with
// XenoNet's own docs and code and talks it through with you. That session is
// told never to file an issue, open a pull request or send anything; what
// it can do for anyone else is write issue-draft.md, for you to read and
// send yourself if you choose.
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import crypto from "node:crypto";
import { scrub, unsafe, summarize } from "./log.js";

export const SCHEMA = "diagnostics/bundle/v1";
// XENONET_ settings whose values say nothing private.
const SHOWN = new Set(["XENONET_PORT", "XENONET_INSTANCE", "XENONET_DEMO", "XENONET_ACTORS", "XENONET_ASSISTANT", "XENONET_HOSTS", "XENONET_IDENTITIES", "XENONET_CLAUDE_USAGE", "XENONET_POLL_MS", "XENONET_STANDALONE", "XENONET_HOT", "XENONET_DEBUG"]);

// The bundle, from what the caller gathered: `xenonet` { version, instance,
// packaged }, `doctor` (a doctor run), `errors` (log entries), `connect`
// (connect's preview), `launcher` (what it says it is), `env`.
export function makeBundle({ xenonet = {}, doctor = null, errors = [], connect = null, launcher = null, env = process.env, home = os.homedir(), now = Date.now(), platform = process.platform, arch = process.arch } = {}) {
  const s = (v) => (typeof v === "string" ? scrub(v, home) : v);
  const bundle = {
    schema: SCHEMA,
    at: new Date(now).toISOString(),
    xenonet: { version: xenonet.version ?? null, instance: xenonet.instance ?? null, packaged: xenonet.packaged ?? null, running: xenonet.running ?? null },
    system: { platform, arch, release: os.release(), node: process.versions.node, electron: process.versions.electron ?? null },
    doctor: doctor ? { at: doctor.at ?? null, summary: doctor.summary ?? null, problems: (doctor.results ?? []).filter((r) => r.status === "warn" || r.status === "fail").map((r) => ({ id: r.id, status: r.status, title: s(r.title), detail: s(r.detail), fix: s(r.fix ?? null) })) } : null,
    errors: summarize(errors).slice(0, 50).map((r) => ({ code: r.code, component: r.component, level: r.level, count: r.count, first: new Date(r.first).toISOString(), last: new Date(r.last).toISOString(), message: s(r.message), fix: s(r.fix), version: r.version })),
    integrations: connect ? { port: connect.port, apps: (connect.apps ?? []).map((a) => ({ app: a.app, changes: (a.changes ?? []).map((c) => ({ id: c.id, state: c.state })), broken: (a.broken ?? []).map(s), notes: (a.notes ?? []).map(s) })) } : null,
    launcher: launcher ? { version: launcher.version ?? null, ships: launcher.ships ?? null, tools: launcher.said?.tools ?? null, node: launcher.said?.node ?? null } : null,
    settings: Object.keys(env).filter((k) => k.startsWith("XENONET_")).sort().map((k) => (SHOWN.has(k) ? `${k}=${env[k]}` : k)),
  };
  const why = unsafe(JSON.stringify(bundle), home);
  if (why) throw new Error(`the diagnostics still held ${why} after scrubbing, so XenoNet won't show them; xenonet logs --json shows what's there`);
  return bundle;
}

// The bundle as Markdown, for an issue or an agent.
export function bundleMarkdown(b) {
  const lines = [`# XenoNet diagnostics`, ``, `${b.at} · XenoNet ${b.xenonet.version ?? "?"}${b.xenonet.instance ? ` (${b.xenonet.instance})` : ""}${b.xenonet.running === false ? ", not running" : ""} · ${b.system.platform} ${b.system.arch} ${b.system.release} · Node ${b.system.node}${b.system.electron ? ` · Electron ${b.system.electron}` : ""}`, ``];
  lines.push(`## Doctor`, ``);
  if (!b.doctor) lines.push(`Not run.`);
  else if (!b.doctor.problems.length) lines.push(`Every check passed.`);
  else for (const p of b.doctor.problems) lines.push(`- **${p.status}** \`${p.id}\` ${p.title}: ${p.detail}${p.fix ? `\n  Fix: ${p.fix}` : ""}`);
  lines.push(``, `## Errors in the last day`, ``);
  if (!b.errors.length) lines.push(`None.`);
  else for (const e of b.errors) lines.push(`- **${e.level}** \`${e.code}\` (${e.component}, ${e.count}×, last ${e.last}${e.version ? `, XenoNet ${e.version}` : ""}): ${e.message}${e.fix ? `\n  Fix: ${e.fix}` : ""}`);
  lines.push(``, `## Integrations`, ``);
  if (!b.integrations) lines.push(`Unknown: XenoNet isn't running.`);
  else for (const a of b.integrations.apps) lines.push(`- ${a.app} (port ${b.integrations.port}): ${a.changes.map((c) => `${c.id} ${c.state}`).join(", ")}${a.broken.length ? `; ${a.broken.join("; ")}` : ""}${a.notes.length ? `\n  ${a.notes.join(" ")}` : ""}`);
  if (b.launcher) lines.push(b.launcher.version < b.launcher.ships ? `- launcher ${b.launcher.version}, older than this XenoNet's ${b.launcher.ships}, which replaces it when it next starts` : `- launcher ${b.launcher.version}, Tools ${b.launcher.tools ?? "missing"}, Node ${b.launcher.node ?? "?"}`);
  lines.push(``, `## Settings`, ``, b.settings.length ? b.settings.map((s) => `- \`${s}\``).join("\n") : `None of XENONET_ set.`, ``);
  return lines.join("\n");
}

// What the agent is told, in the folder XenoNet makes for it: AGENTS.md for
// Codex, CLAUDE.md for Claude Code.
export function agentBrief({ sources = [] } = {}) {
  return [
    `# Helping with XenoNet`,
    ``,
    `The person here runs XenoNet, a desktop console that maps their Claude Code and Codex sessions. Something isn't working, and bundle.md in this folder is what XenoNet could tell about it: its doctor's findings, its errors from the last day by code, where its hooks point, and its versions. It's scrubbed: the home folder is ~, and there are no transcripts, titles or keys.`,
    ``,
    `What to do:`,
    ``,
    `1. Read bundle.md, then explain in plain words what most likely went wrong and why, starting with what affects them most.`,
    `2. Walk them through fixing it, one step at a time, checking with them as you go. Prefer the fixes the doctor and the log already name. \`xenonet doctor\`, \`xenonet logs --errors\` and \`xenonet status\` say more, and are safe to run.`,
    ...(sources.length ? [`3. XenoNet's own docs and code are here to read, if you need to know how something works: ${sources.map((s) => `\`${s}\``).join(", ")}. Start with docs/guide/troubleshooting.md and docs/guide/development.md.`] : []),
    ``,
    `Rules:`,
    ``,
    `- Never file an issue, open a pull request, push, post or send anything anywhere, and don't run gh. XenoNet's maintainers want reports a person chose to send.`,
    `- If it looks like a bug in XenoNet rather than something to fix here, write issue-draft.md in this folder: what happened, what the bundle shows, and how to reproduce it, with no personal details. Tell them it's there; sending it is up to them.`,
    `- Don't change XenoNet's files, its settings, or Claude Code's or Codex's settings without asking first, and say what each change does.`,
    ``,
  ].join("\n");
}

// The folder for one look: <state>/diagnostics/<id>, with bundle.md and
// the brief, kept a week. Returns its path.
export function writeLook(stateDir, bundle, { sources = [], now = Date.now() } = {}) {
  const root = path.join(stateDir, "diagnostics");
  fs.mkdirSync(root, { recursive: true, mode: 0o700 });
  for (const name of fs.readdirSync(root)) {
    const at = Number(/^(\d+)-/.exec(name)?.[1]);
    if (at && now - at > 7 * 86400e3) fs.rmSync(path.join(root, name), { recursive: true, force: true });
  }
  const dir = path.join(root, `${now}-${crypto.randomBytes(3).toString("hex")}`);
  fs.mkdirSync(dir, { mode: 0o700 });
  fs.writeFileSync(path.join(dir, "bundle.md"), bundleMarkdown(bundle));
  fs.writeFileSync(path.join(dir, "bundle.json"), `${JSON.stringify(bundle, null, 2)}\n`);
  const brief = agentBrief({ sources });
  fs.writeFileSync(path.join(dir, "AGENTS.md"), brief);
  fs.writeFileSync(path.join(dir, "CLAUDE.md"), brief);
  return dir;
}

// The command that starts the agent in that folder: Codex read-only,
// asking before it does anything, or Claude Code in plan mode; with the
// model you chose, and XenoNet's docs and code to read.
export function askCommand({ app = "codex", model = null, dir, sources = [] }) {
  const prompt = "Read bundle.md and AGENTS.md here, then tell me what's wrong with XenoNet and how to fix it.";
  if (app === "claude") return { file: "claude", args: ["--permission-mode", "plan", ...(model ? ["--model", model] : []), ...sources.flatMap((s) => ["--add-dir", s]), prompt.replace("AGENTS.md", "CLAUDE.md")], cwd: dir };
  return { file: "codex", args: ["-C", dir, "-s", "read-only", "-a", "on-request", ...(model ? ["-m", model] : []), ...sources.flatMap((s) => ["--add-dir", s]), prompt], cwd: dir };
}
