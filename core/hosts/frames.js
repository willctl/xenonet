// The relay's frames (docs/design/hosts.md): one JSON object per line, as
// MCP's stdio transport has them. A relay writes its frames on its standard
// output and reads a console's on its standard input; a console does the
// opposite, over ssh. Each frame says what it is in `t`:
//
//   relay → console   hello     who's talking (host/hello/v1), always first
//                     snapshot  the map, the live state and the engine's
//                     event     one of the host's events, as its page gets it
//                     res       the answer to a get
//                     time      the host's clock, for a time frame
//                     ping      nothing else to say for 15 seconds
//                     bye       why the relay is stopping
//   console → relay   get       a read from the relay's allowlist
//                     time      what time is it there?
//
// Neither side acts on a frame it doesn't know: it's counted and dropped.
export const PROTOCOL = 1;
// From any host: bigger frames are dropped as they arrive, unread.
export const MAX_FRAME = 16 << 20;
// From a console, which only ever asks for small things.
export const MAX_REQUEST = 64 << 10;

export const FROM_RELAY = new Set(["hello", "snapshot", "event", "res", "time", "ping", "bye"]);
export const FROM_CONSOLE = new Set(["get", "time"]);

// Splits a stream of bytes into lines, however its chunks fall: a line
// across chunks, or a character split between two. A line over `max` bytes
// is dropped as it comes, without being held, and onDrop says so once per
// line. A carriage return before the newline goes too.
export function lineSplitter({ max = MAX_FRAME, onLine, onDrop = () => {} }) {
  let parts = [];
  let size = 0;
  let dropping = false;
  return function push(chunk) {
    const buf = typeof chunk === "string" ? Buffer.from(chunk) : chunk;
    let start = 0;
    for (;;) {
      const nl = buf.indexOf(10, start);
      const piece = buf.subarray(start, nl < 0 ? buf.length : nl);
      if (!dropping) {
        if (size + piece.length > max) {
          dropping = true;
          parts = [];
          size = 0;
          onDrop();
        } else if (piece.length) {
          parts.push(piece);
          size += piece.length;
        }
      }
      if (nl < 0) return;
      if (dropping) dropping = false;
      else if (size) {
        let text = (parts.length === 1 ? parts[0] : Buffer.concat(parts, size)).toString("utf8");
        if (text.endsWith("\r")) text = text.slice(0, -1);
        parts = [];
        size = 0;
        if (text) onLine(text);
      }
      start = nl + 1;
    }
  };
}

// A line as a frame: { frame } or { error }. A frame is a JSON object with
// a string `t`; `known` is the set of kinds the reader acts on.
export function parseFrame(line, known = null) {
  let frame;
  try {
    frame = JSON.parse(line);
  } catch {
    return { error: "not JSON" };
  }
  if (!frame || typeof frame !== "object" || Array.isArray(frame) || typeof frame.t !== "string") return { error: "not a frame" };
  if (known && !known.has(frame.t)) return { error: `a kind of frame this XenoNet doesn't know (${frame.t.slice(0, 32)})` };
  return { frame };
}

export const encode = (frame) => `${JSON.stringify(frame)}\n`;

// Whether a value nests no deeper than `max` arrays and objects, checked
// without recursion. JSON.parse takes any depth, but JSON.stringify and
// structuredClone overflow the stack on a deep enough value, so a host's
// frame nested too deep is dropped before anything else touches it; what
// XenoNet sends nests a few levels, and a transcript's tool calls a few more.
export const MAX_DEPTH = 64;
export function shallow(value, max = MAX_DEPTH) {
  const stack = [[value, 0]];
  while (stack.length) {
    const [v, depth] = stack.pop();
    if (depth > max) return false;
    for (const x of Array.isArray(v) ? v : Object.values(v)) if (x !== null && typeof x === "object") stack.push([x, depth + 1]);
  }
  return true;
}
