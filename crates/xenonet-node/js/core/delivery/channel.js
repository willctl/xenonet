// core/delivery/channel.js in Rust (crates/xenonet-engine/src/delivery/channel.rs):
// Claude Code channels on XenoNet's side. When each session last polled,
// what waits for a poll and every decision are Rust's; the polls
// themselves, promises with their timers, are the twin's.
import { native } from "../../index.js";

export class Channels {
  #core = new native.ChannelsCore();
  #deps = { now: () => this.now() };

  constructor({ now = () => Date.now() } = {}) {
    this.now = now;
    this.waiting = new Map(); // agent → resolve
  }

  live(agent) {
    return this.#core.live(this.#deps, String(agent), this.waiting.has(agent));
  }

  // Waits up to waitMs for items for `agent`; [] on time out.
  poll(agent, waitMs = 25e3) {
    const queued = this.#core.poll(String(agent), this.now());
    if (queued) return Promise.resolve(queued);
    this.waiting.get(agent)?.([]);
    return new Promise((resolve) => {
      const done = (items) => {
        clearTimeout(timer);
        if (this.waiting.get(agent) === done) this.waiting.delete(agent);
        this.#core.polled(String(agent), this.now());
        resolve(items);
      };
      const timer = setTimeout(done, native.channelWaitMs(Number(waitMs)), []);
      timer.unref?.();
      this.waiting.set(agent, done);
    });
  }

  // Hands an item to the session's poll; false when no channel is live.
  push(agent, item) {
    const went = this.#core.push(this.#deps, String(agent), item, this.waiting.has(agent));
    if (went === "waiter") this.waiting.get(agent)([item]);
    return went !== "not-live";
  }
}
