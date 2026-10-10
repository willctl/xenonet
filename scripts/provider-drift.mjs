// The provider drift check: installs the newest versions of the apps whose
// data XenoNet reads (and the versions XenoNet pins), runs each the way XenoNet
// reads it, with no account and in a made-up home, and reports where their
// output and published types differ from XenoNet's vendor schemas. The
// scheduled workflow (.github/workflows/provider-drift.yml) runs it daily;
// docs/guide/provider-drift.md says how to act on its report.
//
//   npm run provider-drift                     inspect every provider, write the report
//   npm run provider-drift -- --providers openai/codex,anthropic/agent-sdk
//   npm run provider-drift -- --version openai/codex=0.161.0-alpha.13
//   npm run provider-drift -- --latest-only    only each provider's latest version
//   npm run provider-drift -- --no-inspect     look up versions and ranges only
//   npm run provider-drift -- versions         the latest versions, and XenoNet's pins
//   npm run provider-drift -- fix --report <report.json>    widen the ranges it allows
//   npm run provider-drift -- issue --report <report.json>  open, update or close the tracking issue (gh)
//
// Options: --out <dir> (where report.json and report.md go; .xenonet/provider-
// drift by default), --work <dir> (where packages are installed and the
// made-up homes go; a temporary folder by default), --json (print the report
// as JSON), --exit-code (exit 1 when there's drift or a check couldn't run),
// --dry-run (fix and issue: say what would change, and change nothing).
//
// Nothing it runs reads or writes ~/.claude, ~/.codex or the desktop apps'
// folders: every CLI runs in a made-up home (provider-drift/sandbox.mjs),
// against a stand-in model API on 127.0.0.1 (provider-drift/fake-api.mjs).
// It needs no secrets. In GitHub Actions it also writes the job summary and
// step outputs (drift, attention, fixable, changes, additions, errors).
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { PROVIDERS, providerById, targetsOf, pinsOf } from "./provider-drift/registry.mjs";
import { latestVersions } from "./provider-drift/releases.mjs";
import { REPO, xenonetSchema } from "./provider-drift/common.mjs";
import { buildReport, rangeFindings, fixPlan, renderMarkdown, hasDrift, needsAttention, issueAction } from "./provider-drift/report.mjs";
import { setVersionsInSource, setVersionsInDocs } from "./provider-drift/ranges.mjs";

const COMMANDS = new Set(["inspect", "versions", "fix", "issue"]);
const LABEL = "provider-drift";
const BRANCH = "provider-drift/ranges";

function parseArgs(argv) {
  const opts = { command: "inspect", providers: null, versions: {}, latestOnly: false, inspect: true, out: path.join(REPO, ".xenonet", "provider-drift"), work: null, json: false, exitCode: false, report: null, dryRun: false };
  const args = [...argv];
  if (args[0] && COMMANDS.has(args[0])) opts.command = args.shift();
  while (args.length) {
    const a = args.shift();
    const value = () => {
      const v = args.shift();
      if (v == null) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === "--providers") opts.providers = value().split(",").map((s) => s.trim()).filter(Boolean);
    else if (a === "--version") {
      const [id, version] = value().split("=");
      if (!id || !version) throw new Error("--version takes <provider>=<version>");
      opts.versions[id] = version;
    } else if (a === "--latest-only") opts.latestOnly = true;
    else if (a === "--no-inspect") opts.inspect = false;
    else if (a === "--out") opts.out = path.resolve(value());
    else if (a === "--work") opts.work = path.resolve(value());
    else if (a === "--json") opts.json = true;
    else if (a === "--exit-code") opts.exitCode = true;
    else if (a === "--report") opts.report = path.resolve(value());
    else if (a === "--dry-run") opts.dryRun = true;
    else if (a === "--help" || a === "-h") opts.command = "help";
    else throw new Error(`unknown option ${a} (see the top of scripts/provider-drift.mjs)`);
  }
  return opts;
}

const log = (msg) => process.stderr.write(`${msg}\n`);

// Lines for $GITHUB_OUTPUT, and the job summary, when Actions gives them.
function actionsOutput(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(values).map(([k, v]) => `${k}=${v}\n`).join(""));
}
const runUrl = () => (process.env.GITHUB_RUN_ID ? `${process.env.GITHUB_SERVER_URL ?? "https://github.com"}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}` : null);

function selected(opts) {
  if (!opts.providers) return PROVIDERS;
  return opts.providers.map((id) => {
    const p = providerById(id);
    if (!p) throw new Error(`no provider ${id}; there are ${PROVIDERS.map((x) => x.id).join(", ")}`);
    return p;
  });
}

function xenonetFacts() {
  const version = JSON.parse(fs.readFileSync(path.join(REPO, "package.json"), "utf8")).version;
  let commit = process.env.GITHUB_SHA ?? null;
  if (!commit)
    try {
      commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: REPO, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
    } catch {}
  return { version, commit };
}

const rangeOf = (source) => xenonetSchema(source)?.["x-xenonet"]?.vendorVersions ?? null;

// ---------- inspect ----------

async function inspect(opts) {
  const work = opts.work ?? fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-provider-drift-work-"));
  fs.mkdirSync(work, { recursive: true });
  const entries = [];
  try {
    for (const p of selected(opts)) {
      log(`${p.name}`);
      let released = null;
      let releaseUrl = null;
      let releaseError = null;
      if (p.release)
        try {
          ({ versions: released, url: releaseUrl } = await latestVersions(p.release));
        } catch (err) {
          releaseError = err.message;
          log(`  couldn't look up its versions: ${err.message}`);
        }
      const pins = pinsOf(p);
      let targets = opts.versions[p.id] ? [{ version: opts.versions[p.id], labels: ["asked for"] }] : targetsOf(p, released, opts.latestOnly ? [] : pins);
      if (opts.latestOnly && !opts.versions[p.id]) targets = targets.slice(0, 1);
      const results = [];
      for (const t of targets) {
        const range = rangeFindings(p.rangeSources ?? [], t.version, rangeOf);
        if (!p.inspect || !opts.inspect) {
          results.push({ ...t, inspected: false, steps: [], findings: range, expected: null, read: {}, info: {} });
          continue;
        }
        log(`  ${t.version} (${t.labels.join(", ")})`);
        let res;
        try {
          res = await p.inspect({ provider: p, version: t.version, work });
        } catch (err) {
          res = { steps: [{ step: "inspect", ok: false, detail: String(err?.stack ?? err).slice(0, 400) }], findings: [], read: {}, info: {} };
        }
        for (const s of res.steps) if (s.ok === false) log(`    couldn't ${s.step}: ${s.detail}`);
        const expectations = res.findings.filter((f) => f.origin === "expectation");
        results.push({
          ...t,
          inspected: true,
          steps: res.steps,
          findings: [...range, ...res.findings.filter((f) => f.severity !== "ok")],
          expected: { met: expectations.filter((f) => f.ok).length, total: expectations.length },
          read: res.read,
          info: res.info,
        });
      }
      entries.push({
        id: p.id,
        name: p.name,
        sources: p.sources,
        release: released ? { url: releaseUrl, versions: released } : null,
        releaseError,
        pins,
        inspectable: !!p.inspect,
        rangeChecked: !!p.rangeSources?.length,
        notInspected: p.notInspected ?? null,
        notInspectedSources: p.notInspectedSources ?? null,
        targets: results,
      });
    }
  } finally {
    if (!opts.work) fs.rmSync(work, { recursive: true, force: true });
  }
  const report = buildReport({ at: new Date().toISOString(), xenonet: xenonetFacts(), providers: entries });
  report.fixes = fixPlan(report, PROVIDERS);
  const md = renderMarkdown(report, { runUrl: runUrl(), fixes: report.fixes });
  fs.mkdirSync(opts.out, { recursive: true });
  fs.writeFileSync(path.join(opts.out, "report.json"), JSON.stringify(report, null, 2) + "\n");
  fs.writeFileSync(path.join(opts.out, "report.md"), md);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, md);
  const { summary } = report;
  actionsOutput({ drift: hasDrift(report), attention: needsAttention(report), fixable: report.fixes.length > 0, changes: summary.changes, additions: summary.additions, errors: summary.errors });
  if (opts.json) process.stdout.write(JSON.stringify(report, null, 2) + "\n");
  else process.stdout.write(md);
  log(`\nwrote ${path.relative(process.cwd(), path.join(opts.out, "report.md"))} and report.json`);
  return opts.exitCode && needsAttention(report) ? 1 : 0;
}

// ---------- versions ----------

async function versions(opts) {
  const rows = [];
  for (const p of selected(opts)) {
    let released = null;
    let error = null;
    if (p.release)
      try {
        released = (await latestVersions(p.release)).versions;
      } catch (err) {
        error = err.message;
      }
    rows.push({ id: p.id, name: p.name, released, error, pins: pinsOf(p) });
  }
  if (opts.json) process.stdout.write(JSON.stringify(rows, null, 2) + "\n");
  else
    for (const r of rows) {
      const tags = r.released ? Object.entries(r.released).map(([k, v]) => `${k} ${v}`).join(", ") : r.error ? `couldn't look up: ${r.error}` : "no published versions to look up";
      const pins = r.pins.filter((p) => p.version).map((p) => `${p.label} ${p.version}`);
      process.stdout.write(`${r.name.padEnd(32)} ${tags}${pins.length ? `; pinned: ${pins.join(", ")}` : ""}\n`);
    }
  return 0;
}

// ---------- fix ----------

const readReport = (opts) => {
  if (!opts.report) throw new Error("--report <report.json> is needed");
  return JSON.parse(fs.readFileSync(opts.report, "utf8"));
};

// The definition file of a source: anthropic/agent-sdk/session-listing is
// in core/schema/vendor/anthropic/agent-sdk.js.
const definitionFile = (source) => {
  const [vendor, product] = source.split("/");
  return path.join(REPO, "core", "schema", "vendor", vendor, `${product}.js`);
};

async function fix(opts) {
  const report = readReport(opts);
  // The plan is worked out again from the findings, by the registry's
  // rules, rather than taken from the report as written.
  const plan = fixPlan(report, PROVIDERS);
  const docsFile = path.join(REPO, "docs", "design", "schemas.md");
  let docs = fs.readFileSync(docsFile, "utf8");
  const done = [];
  for (const f of plan) {
    const file = definitionFile(f.source);
    let text;
    try {
      text = fs.readFileSync(file, "utf8");
    } catch {
      log(`skipped ${f.source}: no ${path.relative(REPO, file)}`);
      continue;
    }
    const next = setVersionsInSource(text, f.source, f.from, f.to);
    if (!next) {
      log(`skipped ${f.source}: its versions aren't ${JSON.stringify(f.from)} in ${path.relative(REPO, file)} any more`);
      continue;
    }
    if (!opts.dryRun) fs.writeFileSync(file, next);
    docs = setVersionsInDocs(docs, f.source, f.from, f.to);
    done.push(f);
    log(`${opts.dryRun ? "would widen" : "widened"} ${f.source}: ${f.from} → ${f.to}`);
  }
  if (done.length && !opts.dryRun) fs.writeFileSync(docsFile, docs);
  const body = [
    "The scheduled provider drift check read these versions without finding a change to what XenoNet reads, but they're outside the versions the schemas were made for. This widens each range to take them; every version the range had stays in it.",
    "",
    ...done.map((f) => `- \`${f.source}\`: \`${f.from}\` → \`${f.to}\` (for ${f.versions.join(", ")})`),
    "",
    "The schema files are regenerated with `npm run schemas`. The tracking issue (label `provider-drift`) lists any additions to decide on.",
    ...(runUrl() ? ["", `From [this run](${runUrl()}).`] : []),
  ].join("\n");
  fs.mkdirSync(opts.out, { recursive: true });
  fs.writeFileSync(path.join(opts.out, "fix.md"), body + "\n");
  actionsOutput({ changed: done.length > 0 && !opts.dryRun, branch: BRANCH });
  if (!done.length) log("nothing to widen");
  return 0;
}

// ---------- the tracking issue ----------

const gh = (args, input) => execFileSync("gh", args, { cwd: REPO, encoding: "utf8", input, stdio: [input == null ? "ignore" : "pipe", "pipe", "inherit"] });

async function issue(opts) {
  const report = readReport(opts);
  let existing = null;
  if (!opts.dryRun) {
    try {
      gh(["label", "create", LABEL, "--color", "D93F0B", "--description", "Reported by the scheduled provider drift check", "--force"]);
    } catch {}
    const open = JSON.parse(gh(["issue", "list", "--label", LABEL, "--state", "open", "--json", "number,body", "--limit", "5"]) || "[]");
    existing = open.sort((a, b) => a.number - b.number)[0] ?? null;
  }
  const action = issueAction(report, existing, { runUrl: runUrl(), fixes: report.fixes ?? [] });
  log(`tracking issue: ${action.action}${existing ? ` #${existing.number}` : ""}`);
  if (opts.dryRun) {
    if (action.body) process.stdout.write(action.body);
    return 0;
  }
  let number = existing?.number ?? null;
  if (action.action === "create") {
    const url = gh(["issue", "create", "--title", action.title, "--label", LABEL, "--body-file", "-"], action.body).trim();
    number = Number(/\/(\d+)\s*$/.exec(url)?.[1]) || null;
  } else if (action.action === "update" || action.action === "refresh") {
    gh(["issue", "edit", String(number), "--title", action.title, "--body-file", "-"], action.body);
    if (action.comment) gh(["issue", "comment", String(number), "--body-file", "-"], action.comment);
  } else if (action.action === "close") {
    gh(["issue", "comment", String(number), "--body-file", "-"], action.comment);
    gh(["issue", "close", String(number)]);
  }
  actionsOutput({ issue: number ?? "", action: action.action });
  return 0;
}

// ---------- main ----------

let opts;
try {
  opts = parseArgs(process.argv.slice(2));
} catch (err) {
  log(err.message);
  process.exit(2);
}
if (opts.command === "help") {
  const head = fs.readFileSync(new URL(import.meta.url), "utf8").split("\n");
  process.stdout.write(head.slice(0, head.findIndex((l) => l.startsWith("import"))).map((l) => l.replace(/^\/\/ ?/, "")).join("\n") + "\n");
  process.exit(0);
}
const run = { inspect, versions, fix, issue }[opts.command];
try {
  process.exitCode = await run(opts);
} catch (err) {
  log(err.stack ?? err.message);
  process.exitCode = 2;
}
