// The drift check for the experimental providers (scripts/provider-drift/
// inspect-native.mjs): what it reads from each project's own types and
// source, against XenoNet's schemas. The fixtures are trimmed copies of
// what those projects publish; nothing is installed or fetched.
import { test } from "node:test";
import assert from "node:assert/strict";
import { schemaFromDts, tsToSchema, declarationsOf } from "../scripts/provider-drift/types.mjs";
import { typeFindings } from "../scripts/provider-drift/common.mjs";
import { drizzleTables, opencodeFindings } from "../scripts/provider-drift/inspect-native.mjs";
import { PROVIDERS, providerById } from "../scripts/provider-drift/registry.mjs";

// From @google/gemini-cli-core's chatRecordingTypes.d.ts.
const GEMINI = `
export interface BaseMessageRecord { id: string; timestamp: string; content: PartListUnion; }
export interface ToolCallRecord { id: string; name: string; args: Record<string, unknown>; status: Status; timestamp: string; }
export type ConversationRecordExtra = { type: 'user' | 'info' | 'error' | 'warning'; } | { type: 'gemini'; toolCalls?: ToolCallRecord[]; model?: string; };
export type MessageRecord = BaseMessageRecord & ConversationRecordExtra;
export interface ConversationRecord { sessionId: string; projectHash: string; startTime: string; lastUpdated: string; messages: MessageRecord[]; summary?: string; kind?: 'main' | 'subagent'; }
export interface RewindRecord { $rewindTo: string; }
export interface MessagePatchRecord { $patch: { id?: string; }; }
export interface MetadataUpdateRecord { $set: Partial<Omit<ConversationRecord, 'messages'>>; }
export interface PartialMetadataRecord { sessionId: string; projectHash: string; startTime?: string; lastUpdated?: string; summary?: string; kind?: 'main' | 'subagent'; }
export type XenoNetChatRecordLine = PartialMetadataRecord | MessageRecord | MessagePatchRecord | MetadataUpdateRecord | RewindRecord;
`;

// From @github/copilot-sdk's generated/session-events.d.ts.
const COPILOT = `
export type SessionEvent = StartEvent | TitleChangedEvent | ShutdownEvent | ContextChangedEvent | ToolExecutionStartEvent;
export interface StartEvent { data: StartData; id: string; parentId: string | null; timestamp: string; type: "session.start"; }
export interface TitleChangedEvent { data: TitleChangedData; id: string; parentId: string | null; timestamp: string; type: "session.title_changed"; }
export interface ShutdownEvent { data: ShutdownData; id: string; parentId: string | null; timestamp: string; type: "session.shutdown"; }
export interface ContextChangedEvent { data: WorkingDirectoryContext; id: string; parentId: string | null; timestamp: string; type: "session.context_changed"; }
export interface ToolExecutionStartEvent { data: ToolExecutionStartData; id: string; parentId: string | null; timestamp: string; type: "tool.execution_start"; }
export interface StartData { context?: WorkingDirectoryContext; copilotVersion: string; sessionId: string; startTime: string; version: number; }
export interface ResumeData { context?: WorkingDirectoryContext; resumeTime: string; }
export interface WorkingDirectoryContext { branch?: string; cwd: string; }
export interface TitleChangedData { title: string; }
export interface ShutdownData { codeChanges: ShutdownCodeChanges; }
export interface ShutdownCodeChanges { filesModified: string[]; linesAdded: number; }
export interface ToolExecutionStartData { arguments?: JsonValue; toolCallId: string; toolName: string; }
export type XenoNetEventData = StartData | ResumeData | WorkingDirectoryContext | TitleChangedData | ShutdownData | ToolExecutionStartData;
`;

const findings = (id, dts) => {
  const p = providerById(id);
  const doc = schemaFromDts(dts, p.types.map((t) => t.provider));
  return typeFindings(p.types, () => doc).map((f) => [f.kind, f.path]);
};

test("the experimental providers are marked, and each is inspected or says why not", () => {
  const ids = ["anomaly/opencode", "google/gemini-cli", "github/copilot-cli", "anysphere/cursor"];
  for (const id of ids) {
    const p = providerById(id);
    assert.ok(p, id);
    assert.equal(p.experimental, true, `${id} is experimental`);
    assert.ok(p.inspect || p.notInspected, id);
  }
  assert.deepEqual(PROVIDERS.filter((p) => p.experimental).map((p) => p.id).sort(), ids.sort());
});

test("an intersection is one object with every part's fields, one per option of a union in it", () => {
  const decls = declarationsOf(GEMINI);
  const node = tsToSchema(decls.get("MessageRecord"), decls);
  assert.equal(node.anyOf.length, 2);
  assert.deepEqual(Object.keys(node.anyOf[1].properties).sort(), ["content", "id", "model", "timestamp", "toolCalls", "type"]);
  assert.ok(node.anyOf[0].required.includes("id"));
});

test("Gemini CLI: its chat record types have what XenoNet reads, and a field gone is a change", () => {
  assert.deepEqual(findings("google/gemini-cli", GEMINI), []);
  const gone = findings("google/gemini-cli", GEMINI.replace("toolCalls?: ToolCallRecord[]", "calls?: ToolCallRecord[]").replace("status: Status;", ""));
  assert.deepEqual(gone, [["removed", "toolCalls"], ["removed", "toolCalls[].status"]]);
});

test("Copilot CLI: the SDK's event types have what XenoNet reads, and a field gone is a change", () => {
  assert.deepEqual(findings("github/copilot-cli", COPILOT), []);
  const gone = findings("github/copilot-cli", COPILOT.replace("cwd: string;", "dir: string;").replace("filesModified: string[];", ""));
  assert.deepEqual(gone, [["removed", "data.cwd"], ["removed", "data.context.cwd"], ["removed", "data.codeChanges.filesModified"]]);
});

// From OpenCode's packages/core/src/session/sql.ts and database/schema.sql.ts.
const SHARED = `export const Timestamps = {
  time_created: integer().notNull().$default(() => Date.now()),
  time_updated: integer().notNull().$onUpdate(() => Date.now()),
}`;
const TABLES = `export const SessionTable = sqliteTable(
  "session",
  {
    id: text().$type<SessionSchema.ID>().primaryKey(),
    parent_id: text().$type<SessionSchema.ID>(),
    directory: DatabasePath.directoryColumn().notNull(),
    title: text().notNull(),
    version: text().notNull(),
    model: text({ mode: "json" }).$type<{ id: string, providerID: string }>(),
    ...Timestamps,
    time_archived: integer(),
  },
  (table) => [index("session_parent_idx").on(table.parent_id)],
)
export const PartTable = sqliteTable(
  "part",
  {
    id: text().$type<PartID>().primaryKey(),
    session_id: text().$type<SessionSchema.ID>().notNull(),
    ...Timestamps,
    data: text({ mode: "json" }).notNull().$type<V1PartData>(),
  },
)`;

test("OpenCode: its tables from its source, with the columns XenoNet reads", () => {
  const tables = drizzleTables(TABLES, SHARED);
  assert.deepEqual([...tables.keys()], ["session", "part"]);
  assert.deepEqual([...tables.get("session").keys()], ["id", "parent_id", "directory", "title", "version", "model", "time_created", "time_updated", "time_archived"]);
  assert.equal(tables.get("session").get("time_created").notNull, true);
  assert.equal(tables.get("session").get("parent_id").notNull, false);
  assert.deepEqual(opencodeFindings(tables), []);
  const changed = drizzleTables(TABLES.replace("    version: text().notNull(),\n", "").replace("directory: DatabasePath.directoryColumn().notNull()", "directory: DatabasePath.directoryColumn()"), SHARED);
  assert.deepEqual(opencodeFindings(changed).map((f) => [f.kind, f.path]).sort(), [["now-optional", "session.directory"], ["removed", "session.version"]]);
  assert.deepEqual(opencodeFindings(new Map()).map((f) => f.path), ["session", "part"]);
});
