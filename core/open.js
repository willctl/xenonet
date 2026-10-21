// Hands actions back to the desktop apps through their URL schemes.
// URLs are built here from nodes the server already knows, never from client input.
import { spawn } from "node:child_process";
import { APPS } from "./apps.js";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DESKTOP_ID = /^local_[A-Za-z0-9-]{1,64}$/;

// What opens a session or project in each app, for the apps the registry
// gives `open`, by the app's id:
//
//   action(node)          the label of the action that opens a session in
//                         the app, when it offers one
//   url(node)             the URL that opens a session in the app, or null
//   newSession            the action that starts a session in a project's
//                         folder: its id, its label and url(folder)
//   handoff(folder, text) a new session, filled in with a handoff brief
const OPENERS = {
  // Claude desktop opens its own Code sessions by their id, and imports
  // Claude Code's by theirs.
  claude: {
    action: (node) => (node.desktopId && DESKTOP_ID.test(node.desktopId) ? "Open in Claude desktop" : !node.desktopId && UUID.test(node.rawId) ? "Import into Claude desktop" : null),
    url: (node) => (node.desktopId && DESKTOP_ID.test(node.desktopId) ? `claude://code/continue?session=${encodeURIComponent(node.desktopId)}` : UUID.test(node.rawId) ? `claude://resume?session=${node.rawId}` : null),
    newSession: { id: "new-claude", label: "New Claude Code session here", url: (folder) => `claude://code/new?folder=${encodeURIComponent(folder)}` },
    handoff: (folder, text) => `claude://code/new?${new URLSearchParams({ ...(folder ? { folder } : {}), q: text })}`,
  },
  // The ChatGPT app opens Codex's threads.
  codex: {
    action: (node) => (UUID.test(node.rawId) ? "Open in ChatGPT · Codex" : null),
    url: (node) => (UUID.test(node.rawId) ? `codex://threads/${node.rawId}` : null),
    handoff: (folder, text) => `codex://threads/new?${new URLSearchParams({ prompt: text, ...(folder ? { path: folder } : {}) })}`,
  },
};
const OPEN = Object.fromEntries(APPS.filter((a) => a.has?.includes("open") && Object.hasOwn(OPENERS, a.id)).map((a) => [a.id, OPENERS[a.id]]));
// An app's opener, by its id: none for anything but an id it has.
const opener = (app) => (typeof app === "string" && Object.hasOwn(OPEN, app) ? OPEN[app] : null);
// The actions that start a session in a project's folder, one for each app
// that has one.
const NEW_SESSION = Object.values(OPEN).flatMap((o) => (o.newSession ? [o.newSession] : []));

// Which actions a node offers. The UI renders these as buttons.
export function actionsFor(node) {
  if (!node) return [];
  const acts = [];
  // A run XenoNet only observes has no transcript here to preview, and
  // nothing to open it in.
  // Nor one an experimental reader reads, for now.
  if (node.kind === "session" && (node.observed || node.reader)) {
    if (node.cwd) acts.push({ id: "open-folder", label: "Open folder" });
    return acts;
  }
  if (node.kind === "session") {
    const label = opener(node.app)?.action(node);
    if (label) acts.push({ id: "open-app", label });
    acts.push({ id: "preview", label: "Preview transcript", client: true });
    acts.push({ id: "stats", label: "Stats", client: true });
    acts.push({ id: "handoff", label: "Hand off…", client: true });
    if (node.cwd) acts.push({ id: "open-folder", label: "Open folder" });
  }
  if (node.kind === "page" && node.path) acts.push({ id: "open-file", label: "Open page" });
  if (node.kind === "wiki" && node.path) acts.push({ id: "open-folder", label: "Open wiki folder" });
  if (node.kind === "project") {
    acts.push({ id: "open-folder", label: "Open folder" });
    acts.push({ id: "open-vscode", label: "Open in VS Code" });
    for (const n of NEW_SESSION) acts.push({ id: n.id, label: n.label });
  }
  if (node.kind === "zone" && node.path) acts.push({ id: "open-folder", label: "Open folder" });
  return acts;
}

export function urlFor(node, action) {
  const folder = node.kind === "session" ? node.worktree || node.cwd : node.path;
  switch (action) {
    case "open-app":
      return opener(node.app)?.url(node) ?? null;
    case "open-folder":
      return folder ?? null;
    case "open-file":
      return node.kind === "page" ? node.path : null;
    case "open-vscode":
      return folder ? vscodeUrl(folder) : null;
    default: {
      // An app's new session in the folder.
      const n = NEW_SESSION.find((x) => x.id === action);
      return n && folder ? n.url(folder) : null;
    }
  }
}

// vscode://file/C:/x on Windows, vscode://file/Users/x on macOS and Linux.
// POSIX paths are encoded per segment, because macOS `open` rejects a URL with
// a space in it; Explorer takes Windows paths as they are.
export function vscodeUrl(folder) {
  const p = String(folder).replace(/\\/g, "/");
  if (/^[A-Za-z]:\//.test(p)) return `vscode://file/${p}`;
  return `vscode://file/${p.replace(/^\/+/, "").split("/").map(encodeURIComponent).join("/")}`;
}

export const MAX_BRIEF = 6000;

// A new session in the other (or same) app, pre-filled with a handoff brief.
export function handoffUrl(target, folder, brief) {
  const text = String(brief ?? "").slice(0, MAX_BRIEF);
  return opener(target)?.handoff(folder, text) ?? null;
}

// Claude desktop's list of Code sessions that are waiting for you.
export const NEEDS_INPUT_URL = "claude://code/needs-input";

export function launch(target) {
  const [cmd, args] =
    process.platform === "win32" ? ["explorer.exe", [target]] : process.platform === "darwin" ? ["open", [target]] : ["xdg-open", [target]];
  // explorer.exe exits 1 even on success, so the exit code is ignored.
  const child = spawn(cmd, args, { detached: true, stdio: "ignore", windowsHide: true });
  child.on("error", () => {});
  child.unref();
}

// A file shown in the file manager: selected in Finder or Explorer, or its
// folder opened elsewhere. The command for `platform`, as [program, args].
export function revealCommand(file, platform = process.platform) {
  if (platform === "darwin") return ["open", ["-R", file]];
  if (platform === "win32") return ["explorer.exe", [`/select,${file}`]];
  return ["xdg-open", [file.replace(/[\\/][^\\/]*$/, "") || "/"]];
}

export function reveal(file) {
  const [cmd, args] = revealCommand(file);
  const child = spawn(cmd, args, { detached: true, stdio: "ignore", windowsHide: true });
  child.on("error", () => {});
  child.unref();
}
