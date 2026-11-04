// A made-up opencode.db, in the layout OpenCode 1.19 writes (its session and
// part tables, the columns XenoNet reads and a few it doesn't).
import { DatabaseSync } from "node:sqlite";

// `usage` adds the columns OpenCode 1.19 keeps a session's cost, tokens and
// model in; a session's own `usage` is { cost, tokens: [input, output,
// reasoning, cacheRead, cacheWrite], model } (model as the JSON text).
export function opencodeDb(file, { sessions = [], parts = [], layout = "current", usage = false } = {}) {
  const db = new DatabaseSync(file);
  db.exec(
    layout === "current"
      ? `CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, version text NOT NULL, share_url text, time_created integer NOT NULL, time_updated integer NOT NULL, time_archived integer);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`
      : `CREATE TABLE session (id text PRIMARY KEY, title text);`,
  );
  if (layout === "current" && usage) {
    db.exec("ALTER TABLE session ADD COLUMN cost real DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN tokens_input integer DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN tokens_output integer DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN tokens_reasoning integer DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN tokens_cache_read integer DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN tokens_cache_write integer DEFAULT 0 NOT NULL; ALTER TABLE session ADD COLUMN model text; ALTER TABLE session ADD COLUMN agent text");
  }
  if (layout === "current") {
    const s = db.prepare("INSERT INTO session (id, project_id, parent_id, slug, directory, title, version, share_url, time_created, time_updated, time_archived) VALUES (?, 'prj', ?, 'slug', ?, ?, ?, NULL, ?, ?, ?)");
    for (const x of sessions) s.run(x.id, x.parent ?? null, x.directory, x.title ?? "New session", x.version ?? "1.19.3", x.created, x.updated ?? x.created, x.archived ?? null);
    if (usage) {
      const u = db.prepare("UPDATE session SET cost = ?, tokens_input = ?, tokens_output = ?, tokens_reasoning = ?, tokens_cache_read = ?, tokens_cache_write = ?, model = ? WHERE id = ?");
      for (const x of sessions) if (x.usage) u.run(x.usage.cost, ...x.usage.tokens, x.usage.model ?? null, x.id);
    }
    const p = db.prepare("INSERT INTO part VALUES (?, 'msg', ?, ?, ?, ?)");
    parts.forEach((x, i) => p.run(`prt_${i}`, x.session, x.at, x.at, typeof x.data === "string" ? x.data : JSON.stringify(x.data)));
  }
  db.close();
}

export const tool = (name, input, status = "completed") => ({ type: "tool", tool: name, callID: "call_1", state: { status, input, output: "SECRET OUTPUT", time: { start: 1, end: 2 } } });

// A made-up opencode.db in OpenCode 2's layout: session_v2 and session_part
// (a tool part has data.name and state.input.path), with the 1.x session
// and part tables beside them when `legacy` is set (a database upgraded
// from 1.x). A session or part marked `legacy` goes in the old tables.
export function opencodeDb2(file, { sessions = [], parts = [], legacy = false } = {}) {
  const db = new DatabaseSync(file);
  const usage = "cost real DEFAULT 0 NOT NULL, tokens_input integer DEFAULT 0 NOT NULL, tokens_output integer DEFAULT 0 NOT NULL, tokens_reasoning integer DEFAULT 0 NOT NULL, tokens_cache_read integer DEFAULT 0 NOT NULL, tokens_cache_write integer DEFAULT 0 NOT NULL, model text";
  db.exec(
    `CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, workspace_id text, parent_id text, slug text NOT NULL, directory text NOT NULL, title text, version text NOT NULL, ${usage}, time_created integer NOT NULL, time_updated integer NOT NULL, time_archived integer);
     CREATE TABLE session_part (message_id text NOT NULL, session_id text NOT NULL, ordinal integer NOT NULL, type text NOT NULL, call_id text, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL, PRIMARY KEY (message_id, ordinal));`,
  );
  if (legacy) {
    db.exec(
      `CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, version text NOT NULL, ${usage}, time_created integer NOT NULL, time_updated integer NOT NULL, time_archived integer);
       CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`,
    );
  }
  for (const x of sessions) {
    const table = x.legacy ? "session" : "session_v2";
    const model = x.model ? JSON.stringify(x.model) : null;
    db.prepare(`INSERT INTO ${table} (id, project_id, parent_id, slug, directory, title, version, cost, tokens_input, model, time_created, time_updated) VALUES (?, 'prj', ?, 'slug', ?, ?, ?, ?, ?, ?, ?, ?)`).run(x.id, x.parent ?? null, x.directory, x.title ?? "New session", x.version ?? "2.2.5", x.cost ?? 0, x.tokens ?? 0, model, x.created, x.updated ?? x.created);
  }
  parts.forEach((x, i) => {
    const data = typeof x.data === "string" ? x.data : JSON.stringify(x.data);
    if (x.legacy) db.prepare("INSERT INTO part VALUES (?, 'msg', ?, ?, ?, ?)").run(`prt_${i}`, x.session, x.at, x.at, data);
    else db.prepare("INSERT INTO session_part VALUES (?, ?, ?, 'tool', NULL, ?, ?, ?)").run(`msg_${i}`, x.session, 0, x.at, x.at, data);
  });
  db.close();
}

// A 2.x tool part: the tool is data.name, and its file is input.path.
export const tool2 = (name, input, status = "completed") => ({ type: "tool", callID: "call_1", name, state: { status, input, output: "SECRET OUTPUT" } });
