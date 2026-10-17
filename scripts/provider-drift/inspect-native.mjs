// Inspecting the experimental providers (OpenCode, Gemini CLI, Copilot CLI)
// against XenoNet's schemas for what their readers read (core/native/*.js).
// None of them runs here: each writes its data only with a model behind it,
// or signed in. What's checked instead is what each project publishes
// itself:
//
//   - Gemini CLI: the types of its chat records (chatRecordingTypes.d.ts in
//     @google/gemini-cli-core, published with each Gemini CLI version);
//   - Copilot CLI: the session event types the official Copilot SDK
//     publishes (generated/session-events.d.ts in @github/copilot-sdk). The
//     SDK is versioned on its own, so its latest is read, whatever the CLI's
//     version;
//   - OpenCode: its database tables, from its source at the release's tag
//     (github.com/anomalyco/opencode, packages/core/src/session/sql.ts).
//
// Cursor publishes no package or source for its hook input; its schema
// follows Cursor's hooks reference and isn't inspected (registry.mjs).
import fs from "node:fs";
import path from "node:path";
import { npmInstall, npmDistTags, installedPackage } from "./releases.mjs";
import { schemaFromDts, splitTop, stripComments } from "./types.mjs";
import { typeFindings, steps, xenonetSchema } from "./common.mjs";

// The types a package declares, from a .d.ts in it, with `extra`
// declarations of XenoNet's naming the provider's own types together (a
// line of a file can be any of several of them).
async function packageTypes(s, { pkg, version, work, file, extra = "" }) {
  const dir = await s.run(`npm install ${pkg}@${version}`, () => npmInstall(pkg, version, work));
  if (!dir) return null;
  const root = path.join(dir, "node_modules", ...pkg.split("/"));
  const dts = await s.run(`read ${file}`, () => fs.readFileSync(path.join(root, file), "utf8"));
  return dts == null ? null : { dts: `${dts}\n${extra}\n`, version: installedPackage(dir, pkg).version };
}

function typesInspection(s, provider, found) {
  const findings = [];
  if (found) {
    const doc = schemaFromDts(found.dts, provider.types.map((t) => t.provider));
    for (const t of provider.types) if (!doc.definitions[t.provider]) findings.push({ source: t.source, kind: "removed", severity: "change", path: t.provider, variant: null, detail: "the types XenoNet compares with aren't declared any more", origin: "types" });
    findings.push(...typeFindings(provider.types.filter((t) => doc.definitions[t.provider]), () => doc));
  }
  return findings;
}

// ---------- Gemini CLI ----------

const GEMINI_CORE = "@google/gemini-cli-core";
// A line of a chat record is the session's metadata, a message, a patch to
// one, a metadata update or a rewind (chatRecordingService.ts writes each).
const GEMINI_LINE = "export type XenoNetChatRecordLine = PartialMetadataRecord | MessageRecord | MessagePatchRecord | MetadataUpdateRecord | RewindRecord;";

export async function inspectGeminiCli({ provider, version, work }) {
  const s = steps();
  const found = await packageTypes(s, { pkg: GEMINI_CORE, version, work, file: "dist/src/services/chatRecordingTypes.d.ts", extra: GEMINI_LINE });
  return { steps: s.list, findings: typesInspection(s, provider, found), read: {}, info: found ? { typesFrom: `${GEMINI_CORE} ${found.version}` } : {} };
}

// ---------- Copilot CLI ----------

const COPILOT_SDK = "@github/copilot-sdk";
// The data of the events XenoNet reads: session.start, session.resume,
// session.context_changed, session.title_changed, session.shutdown and
// tool.execution_start.
const COPILOT_DATA = "export type XenoNetEventData = StartData | ResumeData | WorkingDirectoryContext | TitleChangedData | ShutdownData | ToolExecutionStartData;";

export async function inspectCopilotCli({ provider, work }) {
  const s = steps();
  const latest = await s.run(`look up ${COPILOT_SDK}'s latest version`, async () => (await npmDistTags(COPILOT_SDK)).versions.latest);
  const found = latest ? await packageTypes(s, { pkg: COPILOT_SDK, version: latest, work, file: "dist/generated/session-events.d.ts", extra: COPILOT_DATA }) : null;
  return { steps: s.list, findings: typesInspection(s, provider, found), read: {}, info: found ? { typesFrom: `${COPILOT_SDK} ${found.version}` } : {} };
}

// ---------- OpenCode ----------

const OPENCODE_REPO = "anomalyco/opencode";
const OPENCODE_DB = "anomaly/opencode/db";
// Where its tables are declared (drizzle's sqliteTable), and the columns
// they spread in from elsewhere.
export const OPENCODE_FILES = { tables: "packages/core/src/session/sql.ts", shared: "packages/core/src/database/schema.sql.ts" };

async function githubFile(repo, file, ref, { fetchImpl = fetch, token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN } = {}) {
  const url = `https://api.github.com/repos/${repo}/contents/${file}?ref=${encodeURIComponent(ref)}`;
  const res = await fetchImpl(url, {
    headers: { "user-agent": "xenonet-provider-drift (+https://github.com/willctl/xenonet)", accept: "application/vnd.github.raw", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    signal: AbortSignal.timeout(20_000),
  });
  if (!res.ok) throw new Error(`${repo}/${file} at ${ref} answered ${res.status}`);
  return res.text();
}

// The object literal after `start` in `text`, braces and all.
function objectAfter(text, start) {
  const open = text.indexOf("{", start);
  if (open < 0) return null;
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) return text.slice(open + 1, i);
  }
  return null;
}

// The columns of an object of drizzle column builders: name → { notNull }.
// `spreads` are the objects `...Name` brings in.
function columnsOf(body, spreads = {}) {
  const out = new Map();
  for (const member of splitTop(body, ",")) {
    const spread = /^\.\.\.([A-Za-z_$][\w$]*)$/.exec(member);
    if (spread) {
      for (const [k, v] of spreads[spread[1]] ?? []) out.set(k, v);
      continue;
    }
    const m = /^([A-Za-z_$][\w$]*)\s*:\s*([\s\S]+)$/.exec(member);
    if (m) out.set(m[1], { notNull: /\.notNull\(\)|\.primaryKey\(\)/.test(m[2]) });
  }
  return out;
}

// The tables a drizzle source declares: table name → columns.
export function drizzleTables(source, shared = "") {
  const spreads = {};
  const sharedText = stripComments(shared);
  for (const m of sharedText.matchAll(/export\s+const\s+([A-Za-z_$][\w$]*)\s*=\s*\{/g)) {
    const body = objectAfter(sharedText, m.index + m[0].length - 1);
    if (body != null) spreads[m[1]] = columnsOf(body);
  }
  const text = stripComments(source);
  const tables = new Map();
  for (const m of text.matchAll(/sqliteTable\(\s*["']([\w]+)["']\s*,/g)) {
    const body = objectAfter(text, m.index + m[0].length);
    if (body != null) tables.set(m[1], columnsOf(body, spreads));
  }
  return tables;
}

// XenoNet's columns against the tables: those it reads must be there, and
// those it needs must be NOT NULL. The edits' tool and path are in
// part.data's JSON, which the tables don't type, so only data is checked.
export function opencodeFindings(tables, schema = xenonetSchema(OPENCODE_DB)) {
  const findings = [];
  const add = (kind, severity, at, detail) => findings.push({ source: OPENCODE_DB, kind, severity, path: at, variant: null, detail, origin: "types" });
  const want = [
    ["session", schema?.properties?.session?.items, null],
    ["part", schema?.properties?.edits?.items, ["session_id", "time_created"]],
  ];
  for (const [table, node, only] of want) {
    const columns = tables.get(table);
    if (!columns) {
      add("removed", "change", table, "XenoNet reads this table, and OpenCode's source doesn't declare it any more");
      continue;
    }
    const required = new Set(node?.required ?? []);
    for (const name of only ?? Object.keys(node?.properties ?? {})) {
      const col = columns.get(name);
      if (!col) add("removed", "change", `${table}.${name}`, "XenoNet reads this column, and OpenCode's source doesn't declare it any more");
      else if (required.has(name) && !col.notNull) add("now-optional", "change", `${table}.${name}`, "XenoNet needs it; OpenCode's source says it may be NULL");
    }
    if (table === "part" && !columns.has("data")) add("removed", "change", "part.data", "XenoNet reads the edit tools' file paths from it, and OpenCode's source doesn't declare it any more");
  }
  return findings;
}

export async function inspectOpenCode({ version }) {
  const s = steps();
  const ref = `v${version}`;
  const tables = await s.run(`read its tables from ${OPENCODE_REPO} at ${ref}`, async () => drizzleTables(await githubFile(OPENCODE_REPO, OPENCODE_FILES.tables, ref), await githubFile(OPENCODE_REPO, OPENCODE_FILES.shared, ref)), (t) => `tables ${[...t.keys()].join(", ")}`);
  return { steps: s.list, findings: tables ? opencodeFindings(tables) : [], read: {}, info: tables ? { typesFrom: `${OPENCODE_REPO} ${ref}` } : {} };
}
