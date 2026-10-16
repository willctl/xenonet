// A free runtime check: no provider, agent or real project is involved.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { randomUUID } from "node:crypto";
const execute = promisify(execFile);
const name = `xeno-${randomUUID().replaceAll("-", "").slice(0, 14)}`;
const call = (args) => {
  const result = execute("openshell", args, { timeout: 120000, maxBuffer: 1024 * 1024 });
  result.child.stdin.end();
  return result;
};
let made = false;
try {
  // Mark it ours before create: even an interrupted create can leave a job.
  made = true;
  // A supervisor keeps the canonical process available after the work exits.
  await call(["sandbox", "create", "--name", name, "--from", "xenonet-review-fixture:1", "--label", `xenonet-retrieval-check=${name}`, "--detach", "--", "/bin/sh", "-c", "(printf 'synthetic\\r\\n' > /workspace/result.txt); exec sleep 86400"]);
  const { stdout: state } = await call(["sandbox", "get", name, "-o", "json"]);
  const { stdout } = await call(["sandbox", "exec", name, "--no-login-shell", "--timeout", "15", "--", "/usr/local/bin/node", "-e", "process.stdout.write(require('fs').readFileSync('/workspace/result.txt').toString('base64'))"]);
  if (stdout.trim() !== Buffer.from("synthetic\r\n").toString("base64")) throw new Error("Byte-preserving retrieval failed.");
  const parsed = JSON.parse(state);
  console.log(JSON.stringify({ exactBytes: true, phase: parsed.phase ?? parsed.sandbox?.phase ?? parsed.status?.phase ?? "unknown", processEvidence: "No exit status asserted by this check." }));
} finally {
  if (made) {
    try { await call(["sandbox", "delete", name]); } catch {}
    const { stdout } = await call(["sandbox", "list", "--selector", `xenonet-retrieval-check=${name}`, "-o", "json"]);
    const listing = JSON.parse(stdout);
    if (listing.sandboxes?.length !== 0 || listing.next_page_token) throw new Error("Synthetic sandbox cleanup is unconfirmed.");
  }
}
