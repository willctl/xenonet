// Platform conventions for shortcuts and wording. The server only listens on
// 127.0.0.1, so the browser's platform is always the machine XenoNet reads.
const desktop = globalThis.window?.xenonetDesktop ?? null;
const browserPlatform = globalThis.navigator?.userAgentData?.platform ?? globalThis.navigator?.platform ?? "";
export const isMac = desktop ? desktop.platform === "darwin" : /Mac/.test(browserPlatform);

// The primary shortcut modifier: ⌘ on macOS, Ctrl elsewhere.
export const isMod = (e) => (isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey);

// Shortcut labels are written "Mod+Shift+T" (⌘ on macOS, Ctrl elsewhere);
// "Ctrl" means the Control key on every platform.
const MAC = { Mod: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Enter: "↩" };
const ORDER = ["Ctrl", "Alt", "Shift", "Mod"];
export function keyLabel(combo) {
  const parts = String(combo).split("+");
  const key = parts.pop() || "+";
  if (!isMac) return [...parts.map((p) => (p === "Mod" ? "Ctrl" : p)), key].join("+");
  return [...parts.sort((a, b) => ORDER.indexOf(a) - ORDER.indexOf(b)).map((p) => MAC[p] ?? p), MAC[key] ?? key.toUpperCase()].join("");
}

export const fileManager = isMac ? "Finder" : desktop?.platform === "win32" || /Win/.test(browserPlatform) ? "Explorer" : "the file manager";
