// Hands actions back to the desktop apps through their URL schemes.
// URLs are built here from nodes the server already knows, never from client input.
import { spawn } from "node:child_process";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DESKTOP_ID = /^local_[A-Za-z0-9-]{1,64}$/;

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
    if (node.app === "codex" && UUID.test(node.rawId)) acts.push({ id: "open-app", label: "Open in ChatGPT · Codex" });
    if (node.app === "claude" && node.desktopId && DESKTOP_ID.test(node.desktopId)) acts.push({ id: "open-app", label: "Open in Claude desktop" });
    if (node.app === "claude" && !node.desktopId && UUID.test(node.rawId)) acts.push({ id: "open-app", label: "Import into Claude desktop" });
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
    acts.push({ id: "new-claude", label: "New Claude Code session here" });
  }
  if (node.kind === "zone" && node.path) acts.push({ id: "open-folder", label: "Open folder" });
  return acts;
}

export function urlFor(node, action) {
  const folder = node.kind === "session" ? node.worktree || node.cwd : node.path;
  switch (action) {
    case "open-app":
      if (node.app === "codex" && UUID.test(node.rawId)) return `codex://threads/${node.rawId}`;
      if (node.app === "claude" && node.desktopId && DESKTOP_ID.test(node.desktopId))
        return `claude://code/continue?session=${encodeURIComponent(node.desktopId)}`;
      if (node.app === "claude" && UUID.test(node.rawId)) return `claude://resume?session=${node.rawId}`;
      return null;
    case "open-folder":
      return folder ?? null;
    case "open-file":
      return node.kind === "page" ? node.path : null;
    case "open-vscode":
      return folder ? vscodeUrl(folder) : null;
    case "new-claude":
      return folder ? `claude://code/new?folder=${encodeURIComponent(folder)}` : null;
    default:
      return null;
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
  if (target === "claude") return `claude://code/new?${new URLSearchParams({ ...(folder ? { folder } : {}), q: text })}`;
  if (target === "codex") return `codex://threads/new?${new URLSearchParams({ prompt: text, ...(folder ? { path: folder } : {}) })}`;
  return null;
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
