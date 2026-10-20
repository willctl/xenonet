// core/demo-root.js in Rust (crates/xenonet-app/src/demo_root.rs): a
// Windows terminal or watcher can still hold the old made-up workspace
// during a reload; then it's left alone and another is built beside it.
// The folders are taken out and made through the functions given.
import fs from "node:fs";
import { native } from "../index.js";

export function freshDemoRoot(root, { platform = process.platform, remove = fs.rmSync, allocate = fs.mkdtempSync } = {}) {
  const plan = native.appDemoRootPlan(String(root));
  try { remove(root, { recursive: true, force: true, maxRetries: plan.retries, retryDelay: plan.retryDelay }); return root; }
  catch (err) {
    if (!native.appDemoRootUseSibling(String(platform), typeof err?.code === "string" ? err.code : null)) throw err;
    return allocate(plan.sibling);
  }
}
