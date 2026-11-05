// xenonet refusals: the provider refusals XenoNet kept (core/refusals), from
// the files in its state folder, so it works with XenoNet stopped: list
// them, show one, export them for a dataset, add context to ones kept
// without it, turn keeping context on or off, and scan transcripts for
// refusals from before XenoNet was watching.
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { c, print, Fail } from "../lib/out.js";
import { appDir } from "../../core/dirs.js";
import { config } from "../../core/config.js";
import { listFiles } from "../../core/paths.js";
import { createRefusals, contextOn, setContext } from "../../core/refusals/store.js";
import { claudeSignal, codexSignal } from "../../core/refusals/detect.js";
import { sinceOf } from "./logs.js";

export const help = `The provider refusals XenoNet kept: each Claude Code or Codex turn its
provider refused, with the provider's category and words, the model and
the request's id, and, if you keep context, the transcript up to it.

  xenonet refusals [list]          the records, newest first
  xenonet refusals show <id>       one record; --context prints its context
  xenonet refusals export          JSON lines for a dataset, to stdout or --out
                                 <file>; --with-context adds each one's request
                                 (messages, ready to tokenize); --scrub takes
                                 home folders and keys out
  xenonet refusals capture <id>    add context to a record kept without it, while
                                 its transcript is still there (--all for every one)
  xenonet refusals context on|off  keep context with each new record (off unless
                                 you turn it on: transcripts can hold secrets)
  xenonet refusals scan            look through transcripts for refusals XenoNet
                                 hasn't kept yet

  --since <time>   30m, 6h, 30d (list, export, scan; default everything, and
                   scan 90d)
  --provider <p>   anthropic, openai, claude or codex
  --dev            XenoNet Dev's records
  --json           as JSON

Records stay in XenoNet's state folder until you delete them; nothing is
sent anywhere.`;

const KIND_WORDS = { refusal: "refused", "refusal-recovered": "refused, another model answered", reroute: "moved to another model", "possible-refusal": "rejected the prompt" };
const when = (at) => new Date(at).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });

// What every subcommand shares: the store, the --since and --provider
// filters, and the state folder the records are in.
function context(flags) {
  const dir = path.join(process.env.XENONET_STATE_DIR || appDir("state", flags.dev ? { name: "XenoNet Dev" } : {}), "refusals");
  return {
    dir,
    flags,
    store: createRefusals({ dir, context: () => contextOn(dir) }),
    since: flags.since ? sinceOf(flags.since) : 0,
    provider: flags.provider ? ({ claude: "anthropic", codex: "openai" }[flags.provider] ?? flags.provider) : null,
  };
}

const keeping = (dir) => (contextOn(dir) ? "on" : "off");

function setKeeping({ dir }, id) {
  if (id !== "on" && id !== "off") return print(`Keeping context is ${contextOn(dir) ? c.green("on") : "off"}. xenonet refusals context on|off changes it.`);
  setContext(dir, id === "on");
  return print(id === "on" ? `${c.green("✓")} XenoNet keeps each new refusal's context: the transcript up to it, in its state folder. Transcripts can hold secrets; export --scrub takes home folders and keys out.` : `${c.green("✓")} XenoNet keeps only the refusal itself from now on.`);
}

function list({ dir, store, since, provider, flags }) {
  const records = store.list({ since, provider });
  if (flags.json) return print(records.map(({ source, key, ...r }) => JSON.stringify({ ...r, source: source ? { file: source.file } : null })).join("\n"));
  if (!records.length) return print(c.dim(`No refusals kept${since ? " in that time" : ""}. Keeping context is ${keeping(dir)}.`));
  for (const r of records) print(`${c.dim(when(r.at))}  ${c.bold(r.id)}  ${r.app ?? r.provider} ${KIND_WORDS[r.kind] ?? r.kind}${r.signal.category ? ` · ${r.signal.category}` : ""}${r.session.model ? c.dim(` · ${r.session.model}`) : ""}${r.context ? c.cyan(" · context") : ""}\n  ${c.dim(r.signal.explanation ?? r.signal.text?.split("\n")[0] ?? "")}`);
  return print(c.dim(`\n${records.length} kept. Keeping context is ${keeping(dir)}.`));
}

function show({ store, flags }, id) {
  const r = id && store.get(id);
  if (!r) throw new Error(`no refusal ${id ?? ""}; xenonet refusals lists them`);
  if (flags.context) {
    if (!r.context) throw new Error("it was kept without context; xenonet refusals capture adds it while its transcript is still there");
    return process.stdout.write(zlib.gunzipSync(fs.readFileSync(store.contextFile(r.id))));
  }
  const { key, ...rest } = r;
  return print(JSON.stringify(rest, null, 2));
}

function exportRecords({ store, since, provider, flags }) {
  const lines = store.exportLines({ since, provider, withContext: !!flags.withContext, scrubbed: !!flags.scrub });
  const text = lines.map((l) => JSON.stringify(l)).join("\n") + (lines.length ? "\n" : "");
  if (!flags.out) return process.stdout.write(text);
  fs.writeFileSync(flags.out, text);
  return print(`${c.green("✓")} Wrote ${lines.length} refusal${lines.length === 1 ? "" : "s"} to ${flags.out}${flags.scrub ? ", scrubbed" : ""}.`);
}

function capture({ store, flags }, id) {
  const ids = flags.all ? store.list().filter((r) => !r.context).map((r) => r.id) : id ? [id] : [];
  if (!ids.length) throw new Fail("say which: xenonet refusals capture <id>, or --all", 2);
  let added = 0;
  for (const x of ids) {
    const r = store.capture(x);
    if (r.ok && !r.already) added++;
    else if (!r.ok && !flags.all) throw new Error(`${x}: ${r.why}`);
  }
  return print(`${c.green("✓")} Added context to ${added} of ${ids.length}.`);
}

const sessionOf = (app, file) => `${app}:${/([0-9a-f-]{36})\.jsonl$/i.exec(file)?.[1] ?? path.basename(file, ".jsonl")}`;
const MAYBE_REFUSAL = /refusal|codex_error_info|model_reroute/;

// The refusals in one transcript's lines. Most lines can't be one; only
// those the pattern matches are parsed.
function keepFrom(store, root, file, from) {
  const text = fs.readFileSync(file, "utf8");
  if (!MAYBE_REFUSAL.test(text)) return 0;
  let found = 0;
  for (const line of text.split("\n")) {
    if (!MAYBE_REFUSAL.test(line)) continue;
    let raw;
    try {
      raw = JSON.parse(line);
    } catch {
      continue;
    }
    const sig = root.signal(raw);
    if (!sig || (sig.at && sig.at < from)) continue;
    const rec = store.record(sig, { sessionId: sessionOf(root.app, file), app: root.app, agentId: /subagents/.test(file) ? path.basename(file, ".jsonl") : null, file, cwd: typeof raw.cwd === "string" ? raw.cwd : null });
    if (rec) found++;
  }
  return found;
}

async function scan({ dir, store, flags }) {
  const from = flags.since ? sinceOf(flags.since) : Date.now() - 90 * 86400e3;
  const roots = [
    { app: "claude", dir: config.claudeProjectsDir, signal: claudeSignal },
    { app: "codex", dir: path.join(config.codexHome, "sessions"), signal: codexSignal },
  ];
  let files = 0, found = 0;
  for (const root of roots) {
    for (const file of await listFiles(root.dir, (n) => n.endsWith(".jsonl"), 6)) {
      let st;
      try {
        st = fs.statSync(file);
      } catch {
        continue;
      }
      if (st.mtimeMs < from) continue;
      files++;
      found += keepFrom(store, root, file, from);
    }
  }
  return print(`${c.green("✓")} Looked through ${files} transcript${files === 1 ? "" : "s"}; kept ${found} refusal${found === 1 ? "" : "s"} XenoNet didn't have${found && !contextOn(dir) ? c.dim(" (without context: xenonet refusals context on, then capture --all)") : ""}.`);
}

const SUBCOMMANDS = { context: setKeeping, list, show, export: exportRecords, capture, scan };

export async function run([sub = "list", id], flags) {
  const handler = Object.hasOwn(SUBCOMMANDS, sub) ? SUBCOMMANDS[sub] : null;
  if (!handler) throw new Fail(`no such: xenonet refusals ${sub}. xenonet refusals --help lists them.`, 2);
  return handler(context(flags), id);
}
