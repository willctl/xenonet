import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createOpenShell, runCommand } from "../../core/integrations/openshell.js";
import { createRecords } from "../../core/integrations/common.js";
import { createReviews } from "../../core/integrations/reviews.js";
import { patchExport } from "../../electron/patch-export.js";

export async function verifyResults({ output = null } = {}) {
  if (process.platform !== "linux") throw new Error("Run this free runtime verification in Linux/WSL2.");
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-result-check-"));
  const source = path.join(temp, "source"); fs.mkdirSync(source);
  fs.writeFileSync(path.join(source, "hello.txt"), "Initial tracked bytes\n");
  fs.writeFileSync(path.join(source, "removed.txt"), "Remove this text\n");
  fs.writeFileSync(path.join(source, ".gitignore"), "ignored.txt\n");
  execFileSync("git", ["init", "-q", source]); execFileSync("git", ["-C", source, "add", "."]);
  fs.writeFileSync(path.join(source, "hello.txt"), "Existing dirty edit\r\n");
  fs.writeFileSync(path.join(source, ".env"), "SYNTHETIC_PRIVATE_VALUE\n");
  const records = createRecords(), reviews = createReviews({ dir: path.join(temp, "reviews") });
  const shell = createOpenShell({ records, reviews, run: async (file, args, options) => {
    try { return await runCommand(file, args, options); }
    catch (error) { if (args.includes("-e")) console.error("Synthetic retrieval diagnostic:", String(error.stderr ?? error.message).slice(0, 2000)); throw error; }
  } }); let job;
  try {
    const preview = await shell.preview({ source, upload: true, review: true, image: "xenonet-review-fixture:1", agent: "pi", request: "Perform the deterministic synthetic file changes." });
    assert.ok(preview.baseline.excluded.some((f) => f.path === ".env"));
    job = await shell.launch(preview.ticket); assert.equal(job.status, "created");
    let review;
    for (const until = Date.now() + 30000; Date.now() < until;) {
      review = await shell.retrieve(job.id);
      if (review.result.changes.some((f) => f.path === "added.txt")) break;
      await new Promise((r) => setTimeout(r, 250));
    }
    assert.equal(review.result.complete, true); assert.equal(review.result.execution, "unknown");
    assert.deepEqual(review.result.changes.map((f) => f.path), ["added.txt", "empty.txt", "hello.txt", "removed.txt"]);
    const exported = shell.exportReview(job.id, review.result.revision), file = patchExport(exported);
    assert.ok(file); assert.equal(file.content.includes("SYNTHETIC_PRIVATE_VALUE"), false); assert.equal(review.result.changes.some((f) => f.path === "ignored.txt"), false);
    const patch = path.join(temp, "review.patch"); fs.writeFileSync(patch, file.content);
    const disposable = path.join(temp, "apply"); fs.cpSync(source, disposable, { recursive: true });
    execFileSync("git", ["-C", disposable, "apply", "--check", patch]); execFileSync("git", ["-C", disposable, "apply", patch]);
    assert.equal(fs.readFileSync(path.join(disposable, "hello.txt"), "utf8"), "Synthetic sandbox result\r\n");
    assert.equal(fs.readFileSync(path.join(disposable, "added.txt"), "utf8"), "Added without a final newline");
    assert.equal(fs.statSync(path.join(disposable, "empty.txt")).size, 0); assert.equal(fs.existsSync(path.join(disposable, "removed.txt")), false);
    assert.equal(fs.readFileSync(path.join(source, "hello.txt"), "utf8"), "Existing dirty edit\r\n");
    if (output) fs.writeFileSync(output, file.content, { mode: 0o600 });
    return { exactPatch: true, sourcePreserved: true, excludedSensitiveFile: true, frozenIgnoreRules: true, execution: "unknown", changedFiles: review.result.changes.length, patchHash: exported.receipt.patchHash };
  } catch (error) {
    if (job) console.error("Synthetic result check:", JSON.stringify({ status: records.list().find((r) => r.id === job.id)?.status, steps: records.list().find((r) => r.id === job.id)?.completedSteps, result: shell.review(job.id).result?.changes.map((f) => ({ path: f.path, status: f.status })) }));
    throw error;
  } finally {
    if (job) {
      let removed;
      for (const until = Date.now() + 30000; Date.now() < until;) {
        removed = await shell.remove(job.id); if (removed.status === "deleted") break;
        await new Promise((r) => setTimeout(r, 500));
      }
      assert.equal(removed?.status, "deleted", "Only this check's sandbox must be confirmed deleted.");
    }
    fs.rmSync(temp, { recursive: true, force: true });
  }
}
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) console.log(JSON.stringify(await verifyResults({ output: process.argv[2] })));
