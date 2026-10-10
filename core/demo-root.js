import fs from "node:fs";

// A Windows terminal or watcher can still hold the old synthetic workspace
// during a reload. Leave that folder alone and build another made-up world.
export function freshDemoRoot(root, { platform = process.platform, remove = fs.rmSync, allocate = fs.mkdtempSync } = {}) {
  try { remove(root, { recursive: true, force: true, maxRetries: 4, retryDelay: 50 }); return root; }
  catch (err) {
    if (platform !== "win32" || !["EPERM", "EBUSY", "ENOTEMPTY"].includes(err.code)) throw err;
    return allocate(`${root}-`);
  }
}
