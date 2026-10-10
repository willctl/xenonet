// OSC 52, the escape sequence that lets a program in the terminal drawer
// (src/terminal.js) reach the clipboard. Its data is "<where>;<base64>" to
// set it, or "<where>;?" to ask what's on it.
//
// Setting is taken, as most terminals take it: copying in vim or tmux over
// ssh relies on it. Asking is never answered. xterm's answer is typed into
// the shell as if you had typed it, so any output (a file you cat, a host
// you ssh to) could read your clipboard and send it on. Only the clipboard
// itself ("c") is set, as xterm's own clipboard addon did.

// The text a sequence's data sets the clipboard to, or null when it sets
// nothing: a question, another selection, or base64 that isn't UTF-8 text.
// Empty base64 clears it.
export function clipboardWrite(data) {
  const s = String(data ?? "");
  const cut = s.indexOf(";");
  if (cut < 0 || s.slice(0, cut) !== "c") return null;
  const payload = s.slice(cut + 1);
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(payload)) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)));
  } catch {
    return null;
  }
}

// A handler for xterm's parser.registerOscHandler(52, …): sets the
// clipboard through `write`, and never answers.
export function osc52Handler(write) {
  return (data) => {
    const text = clipboardWrite(data);
    if (text != null) write(text);
    return true;
  };
}
