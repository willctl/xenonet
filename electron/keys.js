// Key combos like "Mod+K", "Escape" or "Control+`" as Chrome DevTools
// Protocol key events, for the control channel's keyboard input (see
// control.js). "Mod" is ⌘ on a Mac and Ctrl elsewhere, as in XenoNet's own
// shortcut hints. Kept apart from control.js so tests can load it without
// Electron.

// CDP's modifier bits.
const BITS = { Alt: 1, Option: 1, Control: 2, Ctrl: 2, Meta: 4, Cmd: 4, Command: 4, Shift: 8 };

// Named keys: [key, code, Windows virtual key code, text].
const NAMED = {
  Enter: ["Enter", "Enter", 13, "\r"],
  Escape: ["Escape", "Escape", 27],
  Esc: ["Escape", "Escape", 27],
  Tab: ["Tab", "Tab", 9],
  Backspace: ["Backspace", "Backspace", 8],
  Delete: ["Delete", "Delete", 46],
  Space: [" ", "Space", 32, " "],
  ArrowUp: ["ArrowUp", "ArrowUp", 38],
  ArrowDown: ["ArrowDown", "ArrowDown", 40],
  ArrowLeft: ["ArrowLeft", "ArrowLeft", 37],
  ArrowRight: ["ArrowRight", "ArrowRight", 39],
  Up: ["ArrowUp", "ArrowUp", 38],
  Down: ["ArrowDown", "ArrowDown", 40],
  Left: ["ArrowLeft", "ArrowLeft", 37],
  Right: ["ArrowRight", "ArrowRight", 39],
  Home: ["Home", "Home", 36],
  End: ["End", "End", 35],
  PageUp: ["PageUp", "PageUp", 33],
  PageDown: ["PageDown", "PageDown", 34],
  Backquote: ["`", "Backquote", 192, "`"],
};
const PUNCTUATION = { "`": "Backquote", "-": "Minus", "=": "Equal", "+": "Equal", "[": "BracketLeft", "]": "BracketRight", "\\": "Backslash", ";": "Semicolon", "'": "Quote", ",": "Comma", ".": "Period", "/": "Slash" };
const PUNCTUATION_VK = { Backquote: 192, Minus: 189, Equal: 187, BracketLeft: 219, BracketRight: 221, Backslash: 220, Semicolon: 186, Quote: 222, Comma: 188, Period: 190, Slash: 191 };

// "Mod+Shift+K" → { key, code, keyCode, modifiers, text }. `text` is set
// only when the key types something (no Ctrl, ⌘ or Alt held).
export function parseKey(combo, platform = process.platform) {
  const parts = String(combo).split("+");
  // "Control++" means Control and the plus key.
  let name = parts.pop();
  if (name === "" && parts.at(-1) === "") (parts.pop(), (name = "+"));
  let modifiers = 0;
  for (const p of parts) {
    const bit = p === "Mod" ? (platform === "darwin" ? 4 : 2) : BITS[p];
    if (!bit) throw new Error(`unknown modifier "${p}" in "${combo}"`);
    modifiers |= bit;
  }
  const shift = !!(modifiers & 8);
  const typing = !(modifiers & 7);
  let key, code, keyCode, text;
  if (NAMED[name]) [key, code, keyCode, text] = NAMED[name];
  else if (/^[a-z]$/i.test(name)) {
    key = shift ? name.toUpperCase() : name.toLowerCase();
    code = `Key${name.toUpperCase()}`;
    keyCode = name.toUpperCase().charCodeAt(0);
    text = key;
  } else if (/^[0-9]$/.test(name)) {
    [key, code, keyCode, text] = [name, `Digit${name}`, name.charCodeAt(0), name];
  } else if (/^F([1-9]|1[0-2])$/.test(name)) {
    [key, code, keyCode] = [name, name, 111 + Number(name.slice(1))];
  } else if (PUNCTUATION[name]) {
    code = PUNCTUATION[name];
    [key, keyCode, text] = [name, PUNCTUATION_VK[code], name];
  } else throw new Error(`unknown key "${name}" in "${combo}"`);
  return { key, code, keyCode, modifiers, text: typing ? text : undefined };
}

// ["Alt", "Shift"] → CDP's modifier bits, for mouse events.
export function modifierBits(names = [], platform = process.platform) {
  return [].concat(names).reduce((bits, m) => bits | parseKey(`${m}+A`, platform).modifiers, 0);
}

// "1600x900" or "1600x900@2" → { width, height, deviceScaleFactor }.
export function parseSize(size) {
  const m = /^(\d{2,5})x(\d{2,5})(?:@(\d+(?:\.\d+)?))?$/.exec(String(size));
  if (!m) throw new Error(`a size looks like 1600x900, or 1600x900@2 for a pixel ratio of 2 (got "${size}")`);
  return { width: Number(m[1]), height: Number(m[2]), deviceScaleFactor: m[3] ? Number(m[3]) : 0 };
}
