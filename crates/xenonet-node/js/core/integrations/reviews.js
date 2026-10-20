// core/integrations/reviews.js in Rust (crates/xenonet-integrations/src/
// reviews.rs): text change review for OpenShell jobs. The frozen
// snapshot, the checks of what a sandbox hands back, the patch and the
// local storage are the Rust's; git runs here. The program that collects
// text inside the sandbox runs in the sandbox's Node, so COLLECT_TEXT is
// the JS's own, from core/integrations/reviews.js.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { native } from "../../index.js";
import { drive } from "../../task.js";
import { settle, driven } from "./common.js";
import { COLLECT_TEXT as COLLECT } from "../../../../../core/integrations/reviews.js";

const execute = promisify(execFile);
const C = native.intgConstants();

export const REVIEW_LIMITS = C.reviewLimits;
export const REVIEW_STORAGE = C.reviewStorage;
export const COLLECT_TEXT = COLLECT;

export const excludedPath = (name) => native.intgExcludedPath(String(name));
export const reviewPath = (name) => native.intgReviewPath(name);
export const textPatch = (before, after) => native.intgTextPatch(before, after);

const env = () => ({ ...process.env });

// git as the Rust asks for it; `allowExit` is an exit code that still
// answers (check-ignore's 1, for nothing ignored).
const asGit = (git) => async (q) => {
  const pending = git("git", q.args, q.options);
  if (q.input != null) pending.child?.stdin?.end(q.input);
  try {
    const { stdout } = await pending;
    return { stdout: String(stdout) };
  } catch (err) {
    if (q.allowExit != null && err.code === q.allowExit) return { stdout: String(err.stdout ?? "") };
    throw err;
  }
};

export function sourceSnapshot(source, { git = execute } = {}) {
  const run = asGit(git);
  return driven(drive(native.intgSourceSnapshot(String(source), env()), (q) => (q.op === "git" ? run(q) : Promise.reject(new Error(`no such request: ${q.op}`)))));
}

export function createReviews({ dir = null, now = Date.now, snapshot = sourceSnapshot, demo = false } = {}) {
  const core = new native.IntgReviews(dir ? String(dir) : null, !!demo);
  const git = asGit(execute);
  const perform = (q) => {
    if (q.op === "now") return now();
    if (q.op === "snapshot") return snapshot(q.source);
    if (q.op === "git") return git(q);
    throw new Error(`no such request: ${q.op}`);
  };
  const run = (task) => driven(drive(task, perform));
  return {
    seed: (options) => settle(core.seed(options, now())),
    prepare: (source) => run(core.prepare(source, env())),
    launch: (id) => run(core.launch(id)).then(() => undefined),
    view: (id) => settle(core.view(id)),
    verifyUploaded: (id, input) => void settle(core.verifyUploaded(id, input)),
    accept: (id, input) => run(core.accept(id, input, env())),
    export: (id, revision) => settle(core.export(id, revision)),
    forget: (id) => settle(core.forget(id)),
  };
}
