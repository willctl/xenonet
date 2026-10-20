// The app registry (core/apps.js): each entry fits app/v1, it is what the
// session ids and the Rust's copy say, and its values are the ones the code
// it feeds reads.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APPS, APP } from "../core/apps.js";
import { SESSION_APPS, appOfSession } from "../core/schema/ids.js";
import { check, explain } from "../core/schema/index.js";
import { LEFT_OUT, readRecord } from "../core/numbat.js";
import { event } from "./fixtures/numbat.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("every entry fits app/v1, strictly", () => {
  for (const app of APPS) {
    const r = check("app/v1", JSON.parse(JSON.stringify(app)), { strict: true });
    assert.ok(r.ok, `${app.id} doesn't fit app/v1:\n${explain(r)}`);
  }
  for (const bad of [{ id: "x", name: "X" }, { id: "X", name: "X", numbat: [] }, { id: "x", name: "X", numbat: [], has: ["teleport"] }, { id: "x", name: "X", numbat: [], connect: { format: "claude-settings", events: [["A", "b", "c"]] } }]) {
    assert.equal(check("app/v1", bad).ok, false, JSON.stringify(bad));
  }
});

test("the registry is what session ids say, in their order", () => {
  assert.deepEqual(Object.entries(SESSION_APPS), APPS.map((a) => [a.id, a.name]));
  assert.equal(new Set(APPS.map((a) => a.id)).size, APPS.length, "each id once");
  for (const a of APPS) assert.equal(appOfSession(`${a.id}:0123456789`), a.id);
  assert.deepEqual(Object.keys(APP), APPS.map((a) => a.id));
});

test("the Rust's copy of the registry is current: node scripts/apps.mjs writes it", () => {
  execFileSync(process.execPath, ["scripts/apps.mjs", "--check"], { cwd: root, stdio: "pipe" });
});

test("numbat leaves out the apps XenoNet tails, and reads every other app under its own id", () => {
  for (const a of APPS) {
    if (a.has?.includes("lines")) for (const n of a.numbat) assert.ok(LEFT_OUT.has(n), `numbat leaves out ${n}`);
    else {
      assert.deepEqual(a.numbat, [a.id]);
      const r = readRecord(JSON.stringify(event({ source_agent: a.id, session_id: "s-0000001" })));
      assert.equal(r.event?.app, a.id, `numbat reads ${a.id}'s runs`);
    }
  }
});
