// Production exposes the supported driver, never arbitrary page execution.
export function consolePolicy({ op, method, args }, production) {
  if (!production) return;
  if (["eval", "cdp"].includes(op)) throw new Error("Arbitrary page execution is available only in development instances.");
  if (op === "driver" && (!/^[a-z][a-zA-Z0-9]*$/.test(method) || ["constructor", "prototype"].includes(method))) throw new Error("Unknown console action.");
  if (op === "driver" && method === "updates" && ![undefined, "status", "check", "automatic", "components", "component-update"].includes(args?.[0])) throw new Error("Install Desktop updates through XenoNet's Updates panel.");
}
