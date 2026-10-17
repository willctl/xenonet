// core/integrations/openshell.js in Rust (crates/xenonet-integrations/src/
// openshell.rs): agents in OpenShell sandboxes. The command lines, the
// preview's checks and steps, and what a launch, an inspection, a
// retrieval and a removal record are the Rust's, each a task this drives;
// the commands, the records and the reviews are asked here.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { native } from "../../index.js";
import { drive, driveSync } from "../../task.js";
import { settle, driven, drivenSync } from "./common.js";
import { createReviews, COLLECT_TEXT } from "./reviews.js";

const execute = promisify(execFile);
const systemRoot = () => process.env.SystemRoot ?? null;

export async function executionProfiles({ platform = process.platform, run = runCommand } = {}) {
  if (platform !== "win32") return [{ backend: "native", distro: "" }];
  try {
    const { stdout } = await run(native.intgWslFile(systemRoot()), ["--list", "--quiet"]);
    return native.intgWslProfiles(String(stdout));
  } catch {
    return [];
  }
}

export async function runCommand(file, args, { timeout = 15000, maxBuffer = 1024 * 1024 } = {}) {
  const result = execute(file, args, { timeout, maxBuffer, windowsHide: true, shell: false, encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
  // None of these CLI operations accepts host input. OpenShell exec also
  // needs EOF on its own input pipe before it can finish a short command.
  result.child?.stdin?.end();
  return result;
}

export const openShellCommand = (profile, args, platform = process.platform) => settle(native.intgCommand(profile, args, String(platform), systemRoot()));

export function createOpenShell({ records, run = runCommand, platform = process.platform, now = Date.now, reviews = createReviews({ now }) }) {
  const core = new native.IntgOpenShell(String(platform), systemRoot(), COLLECT_TEXT);
  // What a task asks: everything but a command is answered at once.
  const asked = (q) => {
    switch (q.op) {
      case "now":
        return now();
      case "records.list":
        return records.list();
      case "records.add":
        return records.add(q.value);
      case "records.update":
        return records.update(q.id, q.fields);
      case "reviews.prepare":
        return reviews.prepare(q.source);
      case "reviews.launch":
        return reviews.launch(q.id);
      case "reviews.verifyUploaded":
        return reviews.verifyUploaded(q.id, q.value);
      case "reviews.view":
        return reviews.view(q.id);
      case "reviews.accept":
        return reviews.accept(q.id, q.value);
      case "reviews.export":
        return reviews.export(q.id, q.revision);
      case "reviews.forget":
        return reviews.forget(q.id);
      default:
        throw new Error(`no such request: ${q.op}`);
    }
  };
  const perform = async (q) => {
    if (q.op !== "run") return asked(q);
    const r = await run(q.file, q.args, q.options ?? undefined);
    return { stdout: r?.stdout, stderr: r?.stderr };
  };
  const go = (task) => driven(drive(task, perform));
  const now_ = (task) => drivenSync(() => driveSync(task, asked));
  return {
    status: (profile) => go(core.status(profile)),
    preview: (body) => go(core.preview(body)),
    launch: (ticket) => go(core.launch(ticket)),
    inspect: (id) => go(core.inspect(id)),
    review: (id) => now_(core.review(id)),
    retrieve: (id) => go(core.retrieve(id)),
    exportReview: (id, revision) => now_(core.exportReview(id, revision)),
    forgetReview: (id) => now_(core.forgetReview(id)),
    remove: (id) => go(core.remove(id)),
  };
}
