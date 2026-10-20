// An apply_patch text (Codex's patch format) read for the files it touches.
// Live, the engine and the native readers each keep their own record of
// what they find; this is only the reading.
//
// Only the headers are read: `*** Add File: `, `*** Update File: ` and
// `*** Delete File: `, and the `*** Move to: ` line under an update that
// renames its file. Each is a whole line, and a header with nothing after it
// isn't one. The path is trimmed, so it can be empty when only spaces follow.

const HEADER = /^\*\*\* (Add|Update|Delete) File: (.+)$|^\*\*\* Move to: (.+)$/gm;

// The headers of an apply_patch text, in the order they come:
// [{ kind: "add" | "update" | "delete" | "move", path }].
export function patchHeaders(text) {
  return [...String(text).matchAll(HEADER)].map((m) => (m[3] === undefined ? { kind: m[1].toLowerCase(), path: m[2].trim() } : { kind: "move", path: m[3].trim() }));
}

// The paths of its headers, in the order they come: the files it adds,
// updates and deletes, and with `moves` the names it moves files to.
export const patchPaths = (text, { moves = true } = {}) => patchHeaders(text).filter((h) => moves || h.kind !== "move").map((h) => h.path);
