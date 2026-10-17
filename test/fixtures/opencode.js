// A made-up opencode.db, in the layout OpenCode 1.19 writes (its session and
// part tables, the columns XenoNet reads and a few it doesn't).
import { DatabaseSync } from "node:sqlite";

export function opencodeDb(file, { sessions = [], parts = [], layout = "current" } = {}) {
  const db = new DatabaseSync(file);
  db.exec(
    layout === "current"
      ? `CREATE TABLE session (id text PRIMARY KEY, project_id text NOT NULL, parent_id text, slug text NOT NULL, directory text NOT NULL, title text NOT NULL, version text NOT NULL, share_url text, time_created integer NOT NULL, time_updated integer NOT NULL, time_archived integer);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);`
      : `CREATE TABLE session (id text PRIMARY KEY, title text);`,
  );
  if (layout === "current") {
    const s = db.prepare("INSERT INTO session VALUES (?, 'prj', ?, 'slug', ?, ?, ?, NULL, ?, ?, ?)");
    for (const x of sessions) s.run(x.id, x.parent ?? null, x.directory, x.title ?? "New session", x.version ?? "1.19.3", x.created, x.updated ?? x.created, x.archived ?? null);
    const p = db.prepare("INSERT INTO part VALUES (?, 'msg', ?, ?, ?, ?)");
    parts.forEach((x, i) => p.run(`prt_${i}`, x.session, x.at, x.at, typeof x.data === "string" ? x.data : JSON.stringify(x.data)));
  }
  db.close();
}

export const tool = (name, input, status = "completed") => ({ type: "tool", tool: name, callID: "call_1", state: { status, input, output: "SECRET OUTPUT", time: { start: 1, end: 2 } } });
