// core/open.js in Rust (crates/xenonet-app/src/open.rs): actions handed
// back to the desktop apps through their URL schemes, built from nodes the
// server already knows. Starting the program stays here.
import { spawn } from "node:child_process";
import { native } from "../index.js";

const C = native.appOpenConstants();

// The fields of a node the actions read.
const pick = (n) => n && { kind: n.kind, observed: n.observed, reader: n.reader, cwd: n.cwd, worktree: n.worktree, app: n.app, rawId: n.rawId, desktopId: n.desktopId, path: n.path };

export const actionsFor = (node) => native.appActionsFor(node ? pick(node) : null);

export const urlFor = (node, action) => native.appUrlFor(pick(node), String(action));

export const vscodeUrl = (folder) => native.appVscodeUrl(String(folder));

export const MAX_BRIEF = C.maxBrief;

export const handoffUrl = (target, folder, brief) => native.appHandoffUrl(String(target), folder ? String(folder) : null, String(brief ?? "")) ?? null;

export const NEEDS_INPUT_URL = C.needsInputUrl;

function start([cmd, args]) {
  // explorer.exe exits 1 even when it worked, so the exit code is ignored.
  const child = spawn(cmd, args, { detached: true, stdio: "ignore", windowsHide: true });
  child.on("error", () => {});
  child.unref();
}

export const launch = (target) => start(native.appLaunchCommand(String(target), process.platform));

export const revealCommand = (file, platform = process.platform) => native.appRevealCommand(String(file), String(platform));

export const reveal = (file) => start(revealCommand(file));
