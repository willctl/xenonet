// A turn you stopped, as the page says it (core/live.js): a live session's
// interruptedAt, from when you stopped its last turn (Esc in Claude Code,
// a turn you interrupted in Codex) until its next prompt, and the turn-end
// that came with it, marked interrupted. It isn't waiting on you, since
// you're the one who stopped it (its needsYou is null on purpose), so it
// never reaches the alert strip, and it doesn't chime.
import { timeAgo } from "./format.js";

// "stopped 2m ago" while an idle session's last turn is one you stopped,
// else null. A session at work again says what it's doing instead.
export function stoppedWords(s, now = Date.now()) {
  const at = s?.interruptedAt;
  if (s?.status !== "idle" || !Number.isFinite(at) || at <= 0) return null;
  return `stopped ${timeAgo(at, now)}`;
}

// A turn-end's words in the activity feed: "turn finished in 42s", or
// "turn stopped after 12s" for one you stopped.
export function turnEndWords(ev) {
  const secs = Number.isFinite(ev?.duration) && ev.duration > 0 ? Math.round(ev.duration / 1000) : null;
  if (ev?.interrupted) return `turn stopped${secs != null ? ` after ${secs}s` : ""}`;
  return `turn finished${secs != null ? ` in ${secs}s` : ""}`;
}

// Whether a live event chimes: a turn that ends by itself, and a subagent
// that starts. A turn you stopped doesn't; you were there.
export const chimes = (ev) => (ev?.kind === "turn-end" && !ev.interrupted) || ev?.kind === "subagent-start";
