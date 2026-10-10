// A SQLite database with a version (see store.js for the JSON kind):
// PRAGMA user_version counts the migration steps taken, and each step runs
// in a transaction of its own with the version it reaches. A database a
// newer XenoNet has moved further is left at its version. With `backup`, a
// database about to be migrated is copied first (VACUUM INTO
// <name>.v<n>.sqlite), the way state.json keeps state.v1.json, and the
// copy is deleted a week later: it holds what the database itself keeps
// only so long (agents' messages, actors' events, your own record), so it
// doesn't keep them for good.
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { Block } from "./lifecycle.js";
import { freeName } from "./store.js";

const WEEK_MS = 7 * 24 * 3600e3;
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// Takes `db` from its version up to the last of `migrations`, each step in
// a transaction of its own with the version it reaches. A database a newer
// XenoNet has moved further is left as it is. Says where it started and where
// it is.
export function migrateSqlite(db, migrations) {
  const from = db.prepare("PRAGMA user_version").get().user_version;
  if (from > migrations.length) return { from, version: from, newer: true };
  for (let v = from; v < migrations.length; v++) {
    db.exec("BEGIN");
    try {
      migrations[v](db);
      db.exec(`PRAGMA user_version = ${v + 1}`);
      db.exec("COMMIT");
    } catch (err) {
      db.exec("ROLLBACK");
      throw err;
    }
  }
  return { from, version: migrations.length, newer: false };
}

export class SqliteStore extends Block {
  // `migrations`: the steps, in order (each a function of the database).
  // `pragmas`: set on open, before migrating ("journal_mode = WAL").
  // `backup`: copy a database before migrating it, and keep each copy for
  // `keepBackupMs` after it was made.
  constructor({ name, file, migrations = [], pragmas = [], backup = false, keepBackupMs = WEEK_MS, clock } = {}) {
    super({ name, kind: "store", clock });
    this.file = file;
    this.migrations = migrations;
    this.pragmas = pragmas;
    this.backup = backup;
    this.keepBackupMs = keepBackupMs;
    this.db = null;
    this.version = null;
    this.backupFile = null;
  }

  // Opens the database and brings it up to date: the DatabaseSync.
  open() {
    if (this.db?.isOpen) return this.db;
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const db = new DatabaseSync(this.file);
      this.db = db;
      for (const p of this.pragmas) db.exec(`PRAGMA ${p}`);
      if (this.backup) {
        this.pruneBackups();
        this.#backup(db);
      }
      this.version = migrateSqlite(db, this.migrations);
    } catch (err) {
      // A failed migration still owns a file handle until it is closed.
      // Windows cannot remove or reopen that file while we hold it.
      if (this.db?.isOpen) this.db.close();
      this.fail(err);
      throw err;
    }
    this.count("opens");
    if (this.version.newer) this.problem(`${path.basename(this.file)} is from a newer XenoNet (version ${this.version.version}), so this one leaves its shape as it is`);
    else this.ok();
    return this.db;
  }

  // A copy of a database with data in it, before its first step up.
  #backup(db) {
    const from = db.prepare("PRAGMA user_version").get().user_version;
    const tables = db.prepare("SELECT count(*) AS n FROM sqlite_master WHERE type = 'table'").get().n;
    if (from >= this.migrations.length || (from === 0 && tables === 0)) return;
    const ext = path.extname(this.file);
    const target = freeName(this.file, `${path.basename(this.file, ext)}.v${from}`, this.clock.now(), ext);
    db.exec(`VACUUM INTO '${target.replace(/'/g, "''")}'`);
    this.backupFile = target;
  }

  // Deletes the copies made before migrations (<name>.v<n>.sqlite, or
  // <name>.v<n>-<time>.sqlite when that name was taken) that are
  // keepBackupMs old, by when they were written. open() does it, and an
  // owner that prunes its database as it runs can call it then too.
  // Returns the files deleted.
  pruneBackups(now = this.clock.now()) {
    const ext = path.extname(this.file);
    const copy = new RegExp(`^${escape(path.basename(this.file, ext))}\\.v\\d+(-[0-9TZ.-]+)?${escape(ext)}$`);
    const dir = path.dirname(this.file);
    let names;
    try {
      names = fs.readdirSync(dir);
    } catch {
      return [];
    }
    const gone = [];
    for (const name of names.filter((n) => copy.test(n))) {
      const file = path.join(dir, name);
      try {
        if (now - fs.statSync(file).mtimeMs < this.keepBackupMs) continue;
        fs.rmSync(file);
        gone.push(file);
      } catch {}
    }
    if (gone.includes(this.backupFile)) this.backupFile = null;
    return gone;
  }

  // Closes it as DatabaseSync does: closing one that's closed throws.
  close() {
    try {
      this.db.close();
    } finally {
      this.stop();
    }
  }
}
