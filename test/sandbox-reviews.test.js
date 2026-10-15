import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReviews, sourceSnapshot, textPatch, reviewPath, COLLECT_TEXT, REVIEW_LIMITS, REVIEW_STORAGE } from "../core/integrations/reviews.js";
import { createOpenShell } from "../core/integrations/openshell.js";
import { createRecords } from "../core/integrations/common.js";
import { patchExport } from "../electron/patch-export.js";

const encoded = (entries) => Object.entries(entries).map(([path, value]) => ({ path, content: Buffer.from(value).toString("base64") }));
const result = (entries, extra = {}) => ({ files: encoded(entries), problems: [], omissions: [], ...extra });
function tree(t, files = {}) {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-review-test-")));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  for (const [name, text] of Object.entries(files)) { const file = path.join(dir, name); fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); }
  return dir;
}
function repo(t, files = { "file.txt": "original\n" }) {
  const dir = tree(t, files); execFileSync("git", ["init", "-q", dir]); execFileSync("git", ["-C", dir, "add", "."]); return dir;
}

test("frozen upload includes dirty working bytes and excludes ignored, sensitive and binary files", async (t) => {
  const source = repo(t, { "a.txt": "original\n", ".env": "PRIVATE_VALUE", ".gitignore": "ignored.txt\n", "binary": Buffer.from([0, 1]) });
  fs.writeFileSync(path.join(source, "a.txt"), "dirty\r\n"); fs.writeFileSync(path.join(source, "ignored.txt"), "ignored"); fs.writeFileSync(path.join(source, "new.txt"), "new");
  const store = createReviews({ dir: tree(t) }); const frozen = await store.prepare(source);
  assert.equal(fs.readFileSync(path.join(frozen.upload, "a.txt"), "utf8"), "dirty\r\n");
  assert.equal(fs.existsSync(path.join(frozen.upload, ".env")), false); assert.equal(fs.existsSync(path.join(frozen.upload, "ignored.txt")), false);
  assert.ok(frozen.excluded.some((e) => e.path === ".env")); assert.ok(frozen.excluded.some((e) => e.path === "binary"));
  assert.ok(frozen.files.some((e) => e.path === "new.txt")); assert.equal(JSON.stringify(frozen).includes("PRIVATE_VALUE"), false);
  await store.launch(frozen.id); const read = await store.accept(frozen.id, result({ ".gitignore": "ignored.txt\n", "a.txt": "dirty\r\n", "new.txt": "new" }));
  assert.equal(read.result.changes.length, 0); assert.equal(read.result.execution, "unknown");
});

test("preview content drift and staged-byte tampering invalidate launch", async (t) => {
  const source = repo(t), store = createReviews({ dir: tree(t) });
  const a = await store.prepare(source); fs.writeFileSync(path.join(source, "file.txt"), "changed");
  await assert.rejects(store.launch(a.id), /project changed/);
  const b = await store.prepare(source); fs.writeFileSync(path.join(b.upload, "file.txt"), "tampered");
  await assert.rejects(store.launch(b.id), /frozen upload changed/);
});

test("reviews persist separately, export the exact cached revision, and survive sandbox deletion", async (t) => {
  const source = repo(t), dir = tree(t), records = createRecords(tree(t)); const store = createReviews({ dir });
  let remote = result({ "file.txt": "edited\r\n", "new file.txt": "without newline" });
  let collections = 0;
  const shell = createOpenShell({ records, reviews: store, platform: "linux", run: async (_, args) => ({ stdout: args.includes(COLLECT_TEXT) ? JSON.stringify(++collections === 1 ? result({ "file.txt": "original\n" }) : remote) : args.includes("list") ? '{"sandboxes":[]}' : "{}" }) });
  const preview = await shell.preview({ source, upload: true, review: true, image: "pi:test", agent: "pi", request: "PRIVATE_TASK" });
  assert.ok(preview.baseline); assert.equal(preview.commands.length, 6); assert.match(preview.command.args[3], /^xeno-[a-f0-9]{14}$/);
  const job = await shell.launch(preview.ticket); assert.equal(job.completedSteps, 6);
  const review = await shell.retrieve(job.id); const revision = review.result.revision;
  const a = shell.exportReview(job.id, revision); assert.equal(a.receipt.execution, "unknown");
  assert.equal(createHash("sha256").update(a.patch).digest("hex"), a.receipt.patchHash);
  remote = result({ "file.txt": "later\n" });
  assert.equal(shell.exportReview(job.id, revision).patch, a.patch, "export never rereads the sandbox");
  assert.equal(JSON.stringify(records.list()).includes("PRIVATE_TASK"), false); assert.equal(JSON.stringify(records.list()).includes("edited"), false);
  assert.equal(createReviews({ dir }).view(preview.baseline.id).result.patch, a.patch);
  await shell.remove(job.id); assert.equal(shell.review(job.id).result.patch, a.patch);
  shell.forgetReview(job.id); assert.equal(shell.review(job.id).available, false);
  assert.throws(() => createReviews({ dir }).view(preview.baseline.id), /No local baseline/);
  assert.equal(fs.readFileSync(path.join(source, "file.txt"), "utf8"), "original\n");
});

test("partial retrieval cannot export, and source drift stays separate from the reviewed baseline", async (t) => {
  const source = repo(t), store = createReviews({ dir: tree(t) }); const frozen = await store.prepare(source); await store.launch(frozen.id);
  fs.writeFileSync(path.join(source, "file.txt"), "unrelated user edit");
  const a = await store.accept(frozen.id, result({ "file.txt": "sandbox edit" }, { problems: [{ path: "image.png", reason: "Binary file" }] }));
  assert.equal(a.result.sourceState, "changed"); assert.equal(a.result.complete, false);
  assert.throws(() => store.export(frozen.id, a.result.revision), /incomplete/);
  const b = await store.accept(frozen.id, result({ "file.txt": "sandbox edit" }));
  assert.throws(() => store.export(frozen.id, a.result.revision), /changed/);
  assert.match(store.export(frozen.id, b.result.revision).patch, /-original/);
  assert.equal(fs.readFileSync(path.join(source, "file.txt"), "utf8"), "unrelated user edit");
});

for (const name of ["../escape", "/absolute", "C:/drive", "a\\b", "file:stream", "NUL.txt", "dir/COM1", "a/../b", "file.", "file ", "a\nfile", "//server/share"]) {
  test(`untrusted path rejected: ${JSON.stringify(name)}`, () => assert.equal(reviewPath(name), false));
}
test("malformed, colliding, binary and oversized results cannot become a review", async (t) => {
  const source = repo(t), store = createReviews({ dir: tree(t) }); const frozen = await store.prepare(source);
  const values = [result({ "../escape": "x" }), result({ ".env": "secret" }), result({ "a": "1", "A": "2" }), result({ "Foo/a.txt": "1", "foo/b.txt": "2" }), result({ "a": "1", "a/b": "2" }), result({ "x": Buffer.from([0]) }), result({ "x": Buffer.from([0xff]) }), result({ "x": "x".repeat(REVIEW_LIMITS.fileBytes + 1) }), { files: [{ path: "x", content: "not base64" }], problems: [], omissions: [] }];
  for (const v of values) await assert.rejects(store.accept(frozen.id, v), /gateway returned/);
  assert.equal(store.view(frozen.id).result, null);
});

test("unexpected staged files and links invalidate the exact upload manifest", async (t) => {
  const source = repo(t), store = createReviews({ dir: tree(t) });
  const a = await store.prepare(source); fs.writeFileSync(path.join(a.upload, ".env"), "not reviewed");
  await assert.rejects(store.launch(a.id), /manifest changed/);
  if (process.platform !== "win32") { const b = await store.prepare(source); fs.symlinkSync(source, path.join(b.upload, "outside")); await assert.rejects(store.launch(b.id), /unexpected file or link/); }
});

test("forget during suspended retrieval cannot recreate local source data", async (t) => {
  const source = repo(t), dir = tree(t); let hold = false, release;
  const store = createReviews({ dir, snapshot: async (src) => { if (hold) await new Promise((r) => { release = r; }); return sourceSnapshot(src); } });
  const frozen = await store.prepare(source); hold = true;
  const pending = store.accept(frozen.id, result({ "file.txt": "changed" }));
  while (!release) await new Promise((r) => setImmediate(r));
  store.forget(frozen.id); release(); await assert.rejects(pending, /No local baseline|forgotten/);
  assert.equal(fs.existsSync(path.join(dir, frozen.id)), false);
});

test("later CLI absence retains the already-created sandbox for reconciliation", async (t) => {
  const source = repo(t), records = createRecords(); let count = 0;
  const shell = createOpenShell({ records, platform: "linux", run: async () => { if (++count === 2) throw Object.assign(new Error("later CLI missing"), { code: "ENOENT" }); return { stdout: "{}" }; } });
  const job = await shell.launch((await shell.preview({ source, upload: true, image: "pi:test", agent: "pi", request: "task" })).ticket);
  assert.equal(job.status, "creation-unknown"); assert.equal(job.completedSteps, 1);
  assert.equal((await shell.remove(job.id)).status, "deletion-requested");
});

test("frozen repository ignore rules govern new files, including nested rules and negation", async (t) => {
  const source = repo(t, { "file.txt": "tracked\n", ".gitignore": "ignored.txt\ngenerated/\n*.tmp\n!keep.tmp\nfile.txt\n", "nested/.gitignore": "local.txt\n", "keep.tmp": "keep\n" });
  execFileSync("git", ["-C", source, "add", "-f", "file.txt"]);
  fs.mkdirSync(path.join(source, "generated")); fs.writeFileSync(path.join(source, "generated", "old.txt"), "local");
  const store = createReviews({ dir: tree(t) }), frozen = await store.prepare(source);
  const read = await store.accept(frozen.id, result({
    "file.txt": "tracked edit\n", ".gitignore": "", "nested/.gitignore": "", "keep.tmp": "keep edit\n",
    "ignored.txt": "excluded", "generated/new.txt": "excluded", "new.tmp": "excluded", "nested/local.txt": "excluded", "allowed.txt": "included",
  }));
  const paths = read.result.changes.map((f) => f.path);
  assert.ok(paths.includes("file.txt")); assert.ok(paths.includes("keep.tmp")); assert.ok(paths.includes("allowed.txt"));
  for (const name of ["ignored.txt", "generated/new.txt", "new.tmp", "nested/local.txt"]) { assert.equal(paths.includes(name), false); assert.ok(read.result.omissions.some((f) => f.path === name)); }
});

test("oversized serialized diffs preserve a smaller readable review after restart", async (t) => {
  const files = Object.fromEntries(Array.from({ length: 16 }, (_, i) => [`f${i}.txt`, "a\n".repeat(131072)]));
  const source = repo(t, files), dir = tree(t), store = createReviews({ dir }); const frozen = await store.prepare(source);
  const small = await store.accept(frozen.id, result({ ...files, "f0.txt": "small\n" }));
  const oversized = Object.fromEntries(Object.keys(files).map((name) => [name, "b\n".repeat(131072)]));
  await assert.rejects(store.accept(frozen.id, result(oversized)), /too large/);
  assert.equal(createReviews({ dir }).view(frozen.id).result.revision, small.result.revision);
  assert.ok(fs.statSync(path.join(dir, frozen.id, "review.json")).size <= REVIEW_STORAGE.recordBytes);
});

test("local review capacity stops new snapshots while retaining existing reviews", async (t) => {
  const source = repo(t), dir = tree(t), store = createReviews({ dir, demo: true });
  const ids = Array.from({ length: REVIEW_STORAGE.count }, () => store.seed({ before: { "a.txt": "a" }, after: { "a.txt": "b" } }));
  await assert.rejects(store.prepare(source), /storage is full/);
  for (const id of ids) assert.ok(createReviews({ dir }).view(id).result);
  store.forget(ids[0]); assert.ok(await store.prepare(source));
});
test("record retention keeps a deleted sandbox's local review reachable until it is forgotten", () => {
  let clock = 0;
  const records = createRecords(null, { limit: 1, now: () => ++clock });
  const saved = records.add({ kind: "sandbox", status: "deleted", reviewId: "local-baseline" });
  records.add({ kind: "decision" }); records.add({ kind: "decision" });
  assert.ok(records.list().some((r) => r.id === saved.id));
  records.update(saved.id, { reviewId: null }); records.add({ kind: "decision" });
  assert.equal(records.list().some((r) => r.id === saved.id), false);
});

test("full-file patches apply exactly: additions, deletions, spaces, CRLF, empty files and missing final newlines", (t) => {
  const before = { "crlf.txt": "before\r\n", "delete.txt": "gone\n", "empty-delete.txt": "", "nonl.txt": "before", "space name.txt": "old\n" };
  const after = { "crlf.txt": "after\r\n", "empty-add.txt": "", "new.txt": "added\n", "nonl.txt": "after", "space name.txt": "new\n" };
  const dir = repo(t, before), patch = textPatch(encoded(before), encoded(after));
  const file = path.join(tree(t), "result.patch"); fs.writeFileSync(file, patch.patch);
  const native = patchExport({ filename: "xenonet-12345678.patch", patch: patch.patch, receipt: { baseline: "a".repeat(64), revision: "b".repeat(36), retrievedAt: 1, patchHash: createHash("sha256").update(patch.patch).digest("hex"), sourceStateAtRetrieval: "changed", excluded: [] } });
  assert.ok(native); fs.writeFileSync(file, native.content);
  execFileSync("git", ["-C", dir, "apply", "--check", file]); execFileSync("git", ["-C", dir, "apply", file]);
  for (const [name, content] of Object.entries(after)) assert.deepEqual(fs.readFileSync(path.join(dir, name)), Buffer.from(content));
  assert.equal(fs.existsSync(path.join(dir, "delete.txt")), false); assert.equal(fs.existsSync(path.join(dir, "empty-delete.txt")), false);
});
test("native patch export rejects malformed filenames, modified bytes and unbounded receipts", () => {
  assert.equal(patchExport({ filename: "../../file", patch: "text" }), null);
  assert.equal(patchExport({ filename: "xenonet-12345678.patch", patch: "text", receipt: { patchHash: "wrong" } }), null);
});

test("collector preserves exact text and refuses symlinks and binary files without following them", (t) => {
  const dir = tree(t, { "text.txt": "exact\r\n", "binary": Buffer.from([0]), ".env": "secret" });
  if (process.platform !== "win32") fs.symlinkSync("/etc/passwd", path.join(dir, "link"));
  const script = COLLECT_TEXT.replace('"/workspace/project"', JSON.stringify(dir));
  if (process.platform === "win32") return t.skip("The declared review collector runs in a Linux image.");
  const value = JSON.parse(execFileSync(process.execPath, ["-e", script], { encoding: "utf8" }));
  assert.equal(value.files.length, 1); assert.deepEqual(Buffer.from(value.files[0].content, "base64"), Buffer.from("exact\r\n"));
  assert.ok(value.problems.some((p) => p.path === "link")); assert.ok(value.problems.some((p) => p.path === "binary"));
  assert.ok(value.omissions.some((p) => p.path === ".env")); assert.equal(JSON.stringify(value).includes("secret"), false);
});

test("expired previews are removed while launched baselines are retained", async (t) => {
  let clock = 0; const source = repo(t), dir = tree(t), store = createReviews({ dir, now: () => clock });
  const abandoned = await store.prepare(source), launched = await store.prepare(source); await store.launch(launched.id);
  clock = 600001; await assert.rejects(store.launch(abandoned.id), /expired/); await store.prepare(source);
  assert.equal(fs.existsSync(path.join(dir, abandoned.id)), false); assert.ok(store.view(launched.id));
});

test("legacy jobs remain inspectable without a fabricated review, and transport loss keeps the cached revision", async (t) => {
  const source = repo(t), records = createRecords(), reviews = createReviews({ dir: tree(t) }); let lost = false;
  let collections = 0;
  const shell = createOpenShell({ records, reviews, platform: "linux", run: async (_, args) => { if (args.includes(COLLECT_TEXT)) { if (lost) throw new Error("transport"); return { stdout: JSON.stringify(result({ "file.txt": ++collections === 1 ? "original\n" : "new" })) }; } return { stdout: "{}" }; } });
  const legacy = records.add({ kind: "sandbox", status: "created", name: "legacy" }); assert.equal(shell.review(legacy.id).available, false);
  await assert.rejects(shell.retrieve(legacy.id), /no retrievable/);
  const job = await shell.launch((await shell.preview({ source, upload: true, review: true, image: "pi:test", agent: "pi", request: "task" })).ticket);
  const first = await shell.retrieve(job.id); lost = true; await assert.rejects(shell.retrieve(job.id), /could not be retrieved/);
  assert.equal(shell.review(job.id).result.revision, first.result.revision); assert.equal(records.list().find((r) => r.id === job.id).retrieval, "unavailable");
});
