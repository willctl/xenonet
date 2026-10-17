// The drift check's report, from what each provider's inspection found:
// its JSON (report.json, which the workflow's later jobs read), its text
// (report.md: the job summary, and the tracking issue's body), the fixes
// it allows, and what to do about the tracking issue. All of it pure: the
// script (scripts/provider-drift.mjs) does the reading and writing.
import crypto from "node:crypto";
import { satisfies } from "../../core/vendor.js";
import { widenRange } from "./ranges.mjs";

export const FORMAT = 1;
export const MARKER = "xenonet-provider-drift";

// ---------- findings ----------

// A version outside the range of a schema whose data carries it.
export function rangeFindings(sources, version, rangeOf) {
  const out = [];
  for (const source of sources) {
    const range = rangeOf(source);
    if (!range || satisfies(version, range)) continue;
    out.push({ source, kind: "new-version", severity: "change", path: null, variant: null, vendorVersion: version, range, detail: `version ${version} is outside the versions the schema was made for (${range})`, origin: "range" });
  }
  return out;
}

// One of each finding: a range check and a reader both say a version is
// new, and two pairs can find the same thing.
export function dedupe(findings) {
  const seen = new Map();
  for (const f of findings) {
    const key = f.kind === "new-version" ? `${f.source}|new-version|${f.vendorVersion}` : `${f.source}|${f.kind}|${f.path}|${JSON.stringify(f.variant ?? null)}|${f.origin}|${f.detail}`;
    const had = seen.get(key);
    if (!had) seen.set(key, f);
    else if (f.origin === "range") seen.set(key, { ...had, ...f, count: had.count });
  }
  return [...seen.values()];
}

const bySeverity = (findings, severity) => findings.filter((f) => f.severity === severity);

// An older version's findings beside the latest's. A field or method the
// older version's types or output don't have yet, and the latest's do, is
// one the provider added since: XenoNet reads it when it's there and does
// without it when not, so it's a note about that version, not drift. A
// version outside a range, an expectation that failed, or a field of
// another type in an older version is still what it was: XenoNet should read
// what older hosts send. What an older version shares with the latest is
// marked (`sameAsLatest`), so it's counted and listed once.
const OLDER_NOTES = (f) => f.origin === "types" || f.origin === "api" || (f.origin === "data" && f.kind === "missing-field");
const findingKey = (f) => `${f.source}|${f.kind}|${f.path}|${JSON.stringify(f.variant ?? null)}|${f.value ?? ""}|${f.kind === "new-version" ? f.vendorVersion : ""}`;
export function relativeToLatest(targets) {
  const latest = targets.find((t) => t.labels?.includes("latest") && t.inspected !== false && !(t.steps ?? []).some((s) => s.ok === false && /install/.test(s.step)));
  if (!latest) return targets;
  const there = new Set((latest.findings ?? []).map(findingKey));
  const relative = (f) => {
    if (there.has(findingKey(f))) return { ...f, sameAsLatest: true };
    if ((f.severity === "change" || f.severity === "addition") && OLDER_NOTES(f))
      return { ...f, severity: "info", detail: f.kind === "removed" ? "this older version's types don't declare it yet; XenoNet reads it where it's there" : `only in this older version: ${f.detail}`, older: true };
    return f;
  };
  return targets.map((t) => (t === latest ? t : { ...t, findings: (t.findings ?? []).map(relative) }));
}

// How a version's inspection went: error (a step failed), changes,
// additions or ok.
export function targetStatus(target) {
  if ((target.steps ?? []).some((s) => s.ok === false)) return "error";
  if (bySeverity(target.findings, "change").length) return "changes";
  if (bySeverity(target.findings, "addition").length) return "additions";
  return target.inspected === false ? "not-inspected" : "ok";
}

const RANK = { error: 4, changes: 3, additions: 2, "not-inspected": 1, ok: 0 };
const worst = (statuses) => statuses.reduce((a, b) => (RANK[b] > RANK[a] ? b : a), "ok");

// The report: every provider with its targets, statuses and counts.
export function buildReport({ at, xenonet, providers }) {
  const list = providers.map((p) => {
    const targets = relativeToLatest(p.targets.map((t) => ({ ...t, findings: dedupe(t.findings ?? []), status: null })));
    for (const t of targets) t.status = targetStatus(t);
    const status = p.releaseError && !targets.length ? "error" : targets.length ? worst(targets.map((t) => t.status)) : p.inspectable ? "ok" : "not-inspected";
    return { ...p, targets, status };
  });
  const all = list.flatMap((p) => p.targets.flatMap((t) => t.findings.filter((f) => !f.sameAsLatest)));
  const errors = list.reduce((n, p) => n + p.targets.filter((t) => t.status === "error").length + (p.releaseError ? 1 : 0), 0);
  return {
    format: FORMAT,
    at,
    xenonet,
    providers: list,
    summary: {
      providers: list.length,
      changes: bySeverity(all, "change").length,
      additions: bySeverity(all, "addition").length,
      notes: bySeverity(all, "info").length,
      errors,
    },
  };
}

export const hasDrift = (report) => report.summary.changes + report.summary.additions > 0;
export const needsAttention = (report) => hasDrift(report) || report.summary.errors > 0;

// ---------- fixes ----------

// The ranges a report says to widen: for each source whose data carries
// its provider's version, a version outside the schema's range that the
// check inspected without a failed step or a change to what XenoNet reads
// in that source. `providers` is the registry (which sources carry the
// provider's version, and which providers are inspected).
export function fixPlan(report, providers) {
  const plan = new Map();
  for (const p of report.providers) {
    const def = providers.find((x) => x.id === p.id);
    if (!def?.inspect) continue;
    for (const t of p.targets) {
      if (t.inspected === false || t.status === "error") continue;
      for (const f of t.findings) {
        if (f.kind !== "new-version" || !def.rangeSources?.includes(f.source)) continue;
        const blocked = t.findings.some((g) => g.source === f.source && g.severity === "change" && g.kind !== "new-version");
        if (blocked) continue;
        const had = plan.get(f.source);
        const from = had?.from ?? f.range;
        const to = widenRange(had?.to ?? from, f.vendorVersion);
        if (!from || !to || to === (had?.to ?? from)) continue;
        plan.set(f.source, { source: f.source, provider: p.id, from, to, versions: [...(had?.versions ?? []), f.vendorVersion] });
      }
    }
  }
  return [...plan.values()];
}

// ---------- text ----------

const code = (s) => `\`${String(s).replaceAll("`", "'")}\``;
const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
// Text kept to a line, for a code span.
const oneLine = (s, max = 300) => String(s ?? "").replace(/\s+/g, " ").trim().slice(0, max);
// What a provider says, kept to a line and to what a report can show.
const clean = (s, max = 300) =>
  String(s ?? "")
    .replace(/[\r\n]+/g, " ")
    .replace(/[<>]/g, (c) => (c === "<" ? "&lt;" : "&gt;"))
    .slice(0, max);

const STATUS_WORDS = { ok: "no drift", additions: "additions", changes: "changes", error: "couldn't check", "not-inspected": "not inspected" };

function countsOf(findings) {
  const n = (s) => bySeverity(findings, s).length;
  return [n("change") && plural(n("change"), "change"), n("addition") && plural(n("addition"), "addition")].filter(Boolean).join(", ");
}

const ORIGINS = { data: "in its output", types: "in its types", api: "in its API", expectation: "reading its output", range: "its version" };

// Findings as list items. The same finding in several kinds of record is
// one item that names them (a new field on every kind of line), and new
// kinds at one place are one item that lists them.
export function findingLines(findings) {
  const groups = new Map();
  for (const f of findings) {
    const listed = f.kind === "new-variant" && f.value !== undefined;
    const key = JSON.stringify([f.source, f.kind, f.path, f.origin, listed ? (f.variant ?? null) : f.detail]);
    let g = groups.get(key);
    if (!g) groups.set(key, (g = { f, variants: [], values: [] }));
    if (listed) g.values.push(f.value);
    else if (f.variant?.length) g.variants.push(f.variant.map((v) => clean(v, 64)).join(" › "));
  }
  return [...groups.values()].map(({ f, variants, values }) => {
    const where = [f.source && code(f.source), f.path && code(clean(f.path, 200))].filter(Boolean).join(" ");
    const kindsOf = values.length ? f.variant : null;
    const inWhat = kindsOf?.length ? ` in ${kindsOf.map((v) => code(clean(v, 64))).join(" › ")}` : variants.length ? ` in ${[...new Set(variants)].map(code).join(", ")}` : "";
    const text =
      values.length > 1 ? `${values.length} kinds ${f.origin === "types" ? "the types declare" : "the data has"} that XenoNet doesn't list: ${values.map((v) => code(clean(v, 64))).join(", ")}` : clean(f.detail);
    const from = ORIGINS[f.origin];
    return `- ${where}${inWhat}${where || inWhat ? ": " : ""}${text}${from ? ` (${from})` : ""}`;
  });
}

const versionsLine = (t) => `${code(t.version)} (${t.labels.join(", ")})`;

// The report as Markdown: a table of providers, then each one's versions,
// with their changes, additions, notes and failed steps, then what to do.
// `runUrl` links the workflow run.
export function renderMarkdown(report, { runUrl = null, fixes = [] } = {}) {
  const { summary } = report;
  const out = [];
  out.push("## Provider drift");
  out.push("");
  const when = report.at ? new Date(report.at).toISOString().replace("T", " ").replace(/\.\d+Z$/, " UTC") : "now";
  const xenonet = [report.xenonet?.version && `XenoNet ${report.xenonet.version}`, report.xenonet?.commit && code(report.xenonet.commit.slice(0, 7))].filter(Boolean).join(" at ");
  out.push(`Checked ${when}${xenonet ? ` against ${xenonet}` : ""}${runUrl ? ` ([run](${runUrl}))` : ""}: ${plural(summary.changes, "change")} to what XenoNet reads, ${plural(summary.additions, "addition")}, ${plural(summary.errors, "check")} that couldn't run.`);
  out.push("");
  out.push("| Provider | Versions | Result |");
  out.push("| --- | --- | --- |");
  for (const p of report.providers) {
    const versions = p.targets.length ? p.targets.map(versionsLine).join(", ") : p.releaseError ? "couldn't look up" : "none";
    const counts = countsOf(p.targets.flatMap((t) => t.findings.filter((f) => !f.sameAsLatest)));
    out.push(`| ${p.name}${p.experimental ? " (experimental)" : ""} | ${versions} | ${STATUS_WORDS[p.status] ?? p.status}${counts && p.status !== "ok" ? ` (${counts})` : ""} |`);
  }
  for (const p of report.providers) {
    out.push("");
    out.push(`### ${p.name}${p.experimental ? " (experimental)" : ""}`);
    if (p.experimental) out.push("", "XenoNet's reader for it is experimental: its findings are for whoever works on that reader.");
    if (p.releaseError) out.push("", `Couldn't look up its versions: ${clean(p.releaseError)}`);
    if (p.notInspected) out.push("", `Not inspected: ${p.notInspected}`);
    for (const [source, why] of Object.entries(p.notInspectedSources ?? {})) out.push("", `Not inspected, ${code(source)}: ${why}`);
    for (const t of p.targets) {
      out.push("");
      const info = t.info ?? {};
      const facts = [
        info.claudeCodeVersion && `its Claude Code ${clean(info.claudeCodeVersion, 40)}`,
        info.transcriptLines != null && `${info.transcriptLines} transcript lines read`,
        info.rolloutLines != null && `${info.rolloutLines} rollout lines read`,
        info.typesFrom && `checked against ${clean(info.typesFrom, 80)}`,
        t.expected?.total && `${t.expected.met} of ${t.expected.total} expectations met`,
      ].filter(Boolean);
      const status = t.inspected === false ? (p.rangeChecked ? "checked against the schemas' ranges only" : "the version only") : (STATUS_WORDS[t.status] ?? t.status);
      out.push(`**${t.version}** (${t.labels.join(", ")}): ${t.inspected === false && t.status === "changes" ? "changes" : status}${facts.length ? `; ${facts.join(", ")}` : ""}`);
      const failed = (t.steps ?? []).filter((s) => s.ok === false);
      const skipped = (t.steps ?? []).filter((s) => s.ok === null);
      const own = t.findings.filter((f) => !f.sameAsLatest);
      const shared = t.findings.length - own.length;
      const lists = [
        // What a step says comes from a provider's own output, so it goes
        // in code, where nothing in it is a link or a mention.
        ["Couldn't run", failed.map((s) => `- ${clean(s.step)}: ${code(oneLine(s.detail))}`)],
        ["Skipped", skipped.map((s) => `- ${clean(s.step)}: ${code(oneLine(s.detail))}`)],
        ["Changes to what XenoNet reads", findingLines(bySeverity(own, "change"))],
        ["Additions", findingLines(bySeverity(own, "addition"))],
        ["Notes", findingLines(bySeverity(own, "info"))],
      ];
      if (shared) out.push("", `${plural(shared, "finding")} the same as the latest version's.`);
      for (const [title, lines] of lists) {
        if (!lines.length) continue;
        out.push("", `${title}:`, "", ...lines.slice(0, 60));
        if (lines.length > 60) out.push(`- and ${lines.length - 60} more (see report.json)`);
      }
    }
  }
  out.push("", "### What to do", "");
  if (!needsAttention(report)) out.push("Nothing: every inspected version reads the way XenoNet expects.");
  else {
    if (summary.changes) out.push("- **Changes** can break what XenoNet reads. Read the provider's release notes, then declare the field or kind XenoNet should read, optional so older data still fits, or make a new major of the schema beside the old one when a field is gone or retyped for good (docs/design/schemas.md, \"When the report shows drift\").");
    if (summary.additions) out.push("- **Additions** can't break a reader. Decide for each whether XenoNet should read it (declare it) or pass over it (`skipped`, or a closed object's `known`), then run `npm run schemas`.");
    if (summary.errors) out.push("- **Couldn't run** means the check itself failed for that version: a provider's CLI may have changed its flags or output. Run `npm run provider-drift -- --providers <id>` locally to see.");
    if (fixes.length) out.push(`- **Prepared**: ${fixes.map((f) => `${code(f.source)} ${code(f.from)} → ${code(f.to)}`).join("; ")}, in a pull request from the ${code("provider-drift/ranges")} branch.`);
  }
  return out.join("\n") + "\n";
}

// ---------- the tracking issue ----------

// What identifies a report's drift: its changes and additions (without
// counts or times), the versions outside ranges, and the checks that
// failed. The same drift found again is the same fingerprint, so the
// tracking issue only says so when something's different.
export function fingerprint(report) {
  const items = new Set();
  for (const p of report.providers) {
    if (p.releaseError) items.add(`${p.id}|release-error`);
    for (const t of p.targets) {
      for (const s of t.steps ?? []) if (s.ok === false) items.add(`${p.id}|step|${s.step.replace(/@\S+/, "")}`);
      for (const f of t.findings)
        if (f.severity === "change" || f.severity === "addition") items.add(`${p.id}|${f.source}|${f.kind}|${f.path}|${JSON.stringify(f.variant ?? null)}|${f.kind === "new-version" ? f.vendorVersion : ""}`);
    }
  }
  return crypto.createHash("sha256").update([...items].sort().join("\n")).digest("hex").slice(0, 16);
}

const fingerprintIn = (body) => new RegExp(`<!-- ${MARKER} fingerprint:([0-9a-f]+) -->`).exec(body ?? "")?.[1] ?? null;

const LIMIT = 60_000;
export function issueBody(report, opts) {
  let text = renderMarkdown(report, opts);
  if (text.length > LIMIT) text = `${text.slice(0, LIMIT)}\n\n(The report is longer than an issue holds; the run's artifact has all of it.)\n`;
  return `${text}\n<!-- ${MARKER} fingerprint:${fingerprint(report)} -->\n`;
}

export const issueTitle = (report) =>
  `Provider drift: ${[report.summary.changes && plural(report.summary.changes, "change"), report.summary.additions && plural(report.summary.additions, "addition"), report.summary.errors && plural(report.summary.errors, "failed check")].filter(Boolean).join(", ") || "none"}`;

// What to do with the tracking issue, given the report and the open issue
// (`existing`: { number, body }, or null):
//   create   no issue is open, and there's something to say
//   update   the drift changed: new body and title, and a comment saying so
//   refresh  the same drift: the body is brought up to date, quietly
//   close    nothing to say any more: a comment, and the issue closed
//   none     nothing to say, and no issue open
export function issueAction(report, existing, opts = {}) {
  const when = report.at ? new Date(report.at).toISOString().slice(0, 10) : "today";
  const run = opts.runUrl ? ` ([run](${opts.runUrl}))` : "";
  if (!needsAttention(report)) return existing ? { action: "close", comment: `No drift as of ${when}${run}: every inspected version reads the way XenoNet expects. Closing.` } : { action: "none" };
  const body = issueBody(report, opts);
  const title = issueTitle(report);
  if (!existing) return { action: "create", title, body };
  if (fingerprintIn(existing.body) === fingerprint(report)) return { action: "refresh", title, body };
  return { action: "update", title, body, comment: `The report changed on ${when}${run}: ${plural(report.summary.changes, "change")}, ${plural(report.summary.additions, "addition")}, ${plural(report.summary.errors, "failed check")}. The description has it all.` };
}
