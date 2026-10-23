// Anthropic's answer to GET /api/oauth/usage, which core/claude-usage.js
// asks the way Claude Code's /usage does, for Claude plan usage.
import { vendor, open, closed, maybe, num, str } from "../common.js";

// A window may also give a limit in dollars, what's used and left of it,
// and why it's locked (null on the plans seen so far). XenoNet shows the
// percent alone.
const window = closed(
  {
    utilization: maybe(num).describe("Used, in percent"),
    resets_at: maybe(str).describe("ISO 8601"),
  },
  { known: ["limit_dollars", "used_dollars", "remaining_dollars", "locked_reason"] },
);

export const Usage = vendor(
  "anthropic/oauth/usage",
  1,
  open({
    five_hour: maybe(window),
    seven_day: maybe(window),
  }),
  {
    title: "Anthropic's plan usage answer",
    description: "The body of GET https://api.anthropic.com/api/oauth/usage (asked with anthropic-beta: oauth-2025-04-20): the five-hour and seven-day windows XenoNet shows. The answer carries no version, so the newest schema reads it.",
    versions: "*",
  },
);
