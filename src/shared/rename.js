// A session's name as you give it in XenoNet, before it's saved in the
// session's own app (state.renameSession): Claude Code and Claude desktop
// (claude.js), or Codex (codex.js).

// Long enough for any title the apps make, short enough to show whole.
export const MAX_TITLE = 120;

// A name on one line, without control characters, at most MAX_TITLE long;
// "" when there's nothing left of it. \s takes in the Unicode line breaks.
export function cleanTitle(title) {
  if (typeof title !== "string") return "";
  return title
    .replace(/[\u0000-\u001f\u007f-\u009f]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, MAX_TITLE)
    .trim();
}

// Whether a node is a session XenoNet can rename: one of this machine's.
// Another machine's are read-only (see hosts/hub.js).
export const renamable = (n) => n?.kind === "session" && !n.host && (n.app === "claude" || n.app === "codex");
