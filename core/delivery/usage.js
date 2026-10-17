// How much of its plan a provider has used, from 0 to 1, for the wake
// budget's rule that a provider at 90% takes no wakes (policy.js). Read from
// the live limit readings (core/live.js): the fuller of the provider's
// windows that hasn't reset yet. null when there's no fresh reading, and for
// a provider XenoNet reads no limits for.
const FRESH_MS = 6 * 3600e3;

export function planUsage(limits, app, now = Date.now()) {
  const pcts = [];
  if (app === "codex") {
    const c = limits?.codex;
    if (!c || now - (c.observedAt ?? 0) > FRESH_MS) return null;
    for (const w of [c.primary, c.secondary]) if (Number.isFinite(w?.pct) && !(w.resetsAt && w.resetsAt <= now)) pcts.push(w.pct);
  } else if (app === "claude") {
    const c = limits?.claude;
    if (!c || c.stale) return null;
    if (Number.isFinite(c.fiveHour) && !(c.resets?.fiveHour && c.resets.fiveHour <= now)) pcts.push(c.fiveHour);
    if (Number.isFinite(c.sevenDay) && !(c.resets?.sevenDay && c.resets.sevenDay <= now)) pcts.push(c.sevenDay);
  }
  return pcts.length ? Math.max(...pcts) / 100 : null;
}
