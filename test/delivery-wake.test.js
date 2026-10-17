// The plan readings that hold wakes at 90% (core/delivery/usage.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import { planUsage } from "../core/delivery/usage.js";

test("plan usage for the 90% rule: the fuller window that hasn't reset, from a fresh reading", () => {
  const now = Date.parse("2026-10-16T12:00:00Z");
  const codex = { primary: { pct: 40, resetsAt: now + 3600e3 }, secondary: { pct: 92, resetsAt: null }, observedAt: now - 60e3 };
  assert.equal(planUsage({ codex }, "codex", now), 0.92);
  assert.equal(planUsage({ codex: { ...codex, secondary: { pct: 99, resetsAt: now - 1 } } }, "codex", now), 0.4, "a window that reset says nothing");
  assert.equal(planUsage({ codex: { ...codex, observedAt: now - 7 * 3600e3 } }, "codex", now), null, "too old");
  assert.equal(planUsage({ claude: { fiveHour: 30, sevenDay: 91, resets: {} } }, "claude", now), 0.91);
  assert.equal(planUsage({ claude: { fiveHour: null, sevenDay: null, stale: true } }, "claude", now), null);
  assert.equal(planUsage({ claude: { fiveHour: 95 } }, "opencode", now), null, "no readings for it");
});
