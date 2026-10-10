// Component releases change independently; these wire contracts do not.
export const SERVICE_API = { min: 1, max: 1 };
export const CONSOLE_API = { min: 1, max: 1 };
// "assistant-1": remote actors' read-only endpoint, at assistant API 1
// (core/assistant/protocol.js).
export const CAPABILITIES = ["console", "terminals", "terminal-reattach", "assistant-1"];

export function negotiate(local, remote) {
  if (![local?.min, local?.max, remote?.min, remote?.max].every(Number.isInteger)) throw new Error("XenoNet did not report a supported interface version. Update the service first.");
  const version = Math.min(local.max, remote.max);
  if (local.min > local.max || remote.min > remote.max || version < Math.max(local.min, remote.min)) throw new Error("These XenoNet components use incompatible interfaces. Open Updates or run xenonet update.");
  return version;
}
