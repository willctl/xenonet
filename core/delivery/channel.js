// Claude Code channels on XenoNet's side: the xenonet MCP server in a
// Claude Code session started with XenoNet as a channel
// (--dangerously-load-development-channels server:xenonet) holds a long
// poll here; a message for that session answers the poll, and the server
// pushes it into the live session as notifications/claude/channel
// (https://code.claude.com/docs/en/channels-reference).
const LIVE_MS = 5000; // a poll that ended this recently still counts: the next one is on its way

export class Channels {
  constructor({ now = () => Date.now() } = {}) {
    this.now = now;
    this.waiting = new Map(); // agent → resolve
    this.lastPoll = new Map();
    this.queue = new Map(); // agent → items not yet taken
  }

  live(agent) {
    return this.waiting.has(agent) || this.now() - (this.lastPoll.get(agent) ?? -Infinity) < LIVE_MS;
  }

  // Waits up to waitMs for items for `agent`; [] on time out.
  poll(agent, waitMs = 25e3) {
    this.lastPoll.set(agent, this.now());
    const queued = this.queue.get(agent);
    if (queued?.length) {
      this.queue.delete(agent);
      return Promise.resolve(queued);
    }
    this.waiting.get(agent)?.([]);
    return new Promise((resolve) => {
      const done = (items) => {
        clearTimeout(timer);
        if (this.waiting.get(agent) === done) this.waiting.delete(agent);
        this.lastPoll.set(agent, this.now());
        resolve(items);
      };
      const timer = setTimeout(done, Math.min(Math.max(waitMs, 0), 60e3), []);
      timer.unref?.();
      this.waiting.set(agent, done);
    });
  }

  // Hands an item to the session's poll; false when no channel is live.
  push(agent, item) {
    if (!this.live(agent)) return false;
    const w = this.waiting.get(agent);
    if (w) w([item]);
    else this.queue.set(agent, [...(this.queue.get(agent) ?? []), item]);
    return true;
  }
}
