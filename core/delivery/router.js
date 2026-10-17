// The delivery router: for each message and each session it's for, the
// fastest way in that's allowed, then whether it arrived.
//
//   live       pushed into the running session: a Claude Code channel, an
//              OpenCode server's prompt_async, or typed into its tmux pane
//   next turn  waits for the session's hooks, as before (when it next starts
//              a turn, or during the one under way)
//   notified   nothing picked it up within ackMs, so XenoNet told you
//
// Acks: an OpenCode server's 204 is one; a channel or a typed message is
// acked by the session's next hook after the push; a next-turn message when
// the hooks hand it over (the engine's "seen"). A push with no ack goes back
// to waiting for the hooks. Actors aren't routed here: they read their inbox.
//
// Waking a session spends its plan, so every wake is checked against the
// policy (pause switch, consent, wake budget) first; a message that may not
// wake anything still waits for the next turn.
import { EventEmitter } from "node:events";
import { framed, hashText } from "./policy.js";

export const ACK_MS = 60e3;
const KEEP = 500;

export class DeliveryRouter extends EventEmitter {
  constructor({ engine, policy, channels = null, opencode = null, tmux = null, notify = () => {}, usage = () => null, now = () => Date.now(), ackMs = ACK_MS }) {
    super();
    Object.assign(this, { engine, policy, channels, opencode, tmux, notify, usage, now, ackMs });
    this.records = new Map(); // `${message}|${agent}` → record
    this.timers = new Map();
    engine.on("message", (m) => void this.route(m).catch((err) => this.emit("error", err)));
    engine.on("seen", ({ agent, ids }) => this.#seen(agent, ids));
    engine.on("contact", ({ agent, via }) => via === "hook" && this.#contact(agent));
  }

  // Whether a session can be reached without hooks (an OpenCode run XenoNet
  // has a way into), so the engine counts it as reachable.
  reaches(id) {
    return String(id).startsWith("opencode:") && !!this.opencode && this.policy.consented(id);
  }

  list(limit = 100) {
    return [...this.records.values()].slice(-limit);
  }

  async route(m) {
    const out = [];
    for (const agent of m.to) {
      if (String(agent).startsWith("actor:") || agent === "you") continue;
      const rec = { message: m.id, agent, from: m.from, state: "queued", how: null, at: this.now(), ackedAt: null, why: null };
      this.records.set(`${m.id}|${agent}`, rec);
      while (this.records.size > KEEP) this.records.delete(this.records.keys().next().value);
      await this.#attempt(rec, m);
      this.#arm(rec, m);
      out.push(rec);
    }
    return out;
  }

  async #attempt(rec, m) {
    const view = await this.engine.view(rec.agent).catch(() => null);
    const app = rec.agent.split(":")[0];
    const idle = !view || view.status === "idle" || view.status === "ended";
    const fromName = m.fromName ?? (m.from === "you" ? "your user" : (await this.engine.view(m.from).catch(() => null))?.title) ?? m.from;
    const text = framed({ id: m.id, from: fromName, verified: m.verified, text: m.text, user: m.from === "you" });
    rec.fromName = fromName;
    const tried = [];
    const wake = idle ? this.policy.wakeAllowed(m.from, rec.agent, { usage: this.usage(app) }) : { ok: true };
    const success = (how, extra = {}) => {
      Object.assign(rec, { state: "live", how, ...extra });
      if (idle) this.policy.spendWake(m.from, rec.agent);
      this.engine.store.markSeen(rec.agent, [m.id], this.now());
      this.policy.audit({ kind: "delivered", message: m.id, from: m.from, to: rec.agent, how, text: hashText(m.text) });
      this.emit("delivery", { ...rec });
      return true;
    };
    if (!wake.ok) tried.push(`no wake: ${wake.why}`);
    else {
      // 1. A Claude Code channel: the session opted in by starting with it.
      // Only to wake an idle one: a working session gets the message from its
      // hooks during the turn under way, sooner than from a channel, which
      // Claude Code holds until that turn ends.
      if (app === "claude" && idle && this.channels?.live(rec.agent)) {
        if (this.channels.push(rec.agent, { id: m.id, from: fromName, text })) return success("channel");
        tried.push("channel closed");
      }
      // 2. OpenCode's own server, with your consent for that session.
      if (app === "opencode" && this.opencode) {
        if (!this.policy.consented(rec.agent, { remote: m.from.startsWith("actor:") })) tried.push("opencode: no consent");
        else {
          const r = await this.opencode.deliver(rec.agent, text).catch((err) => ({ ok: false, why: err.message }));
          if (r.ok) return success("opencode", { ackedAt: this.now(), where: r.where });
          tried.push(`opencode: ${r.why}`);
        }
      }
      // 3. Typing into its tmux pane: consent, a sender XenoNet verified with
      // a token or you, and an idle session that isn't waiting on its user.
      if (this.tmux && idle && !view?.needsYou) {
        const sender = m.from === "you" || m.from.startsWith("actor:");
        const pid = this.engine.store.contact?.(rec.agent)?.pid ?? null;
        if (!this.policy.consented(rec.agent, { remote: m.from.startsWith("actor:") })) tried.push("tmux: no consent");
        else if (!sender) tried.push("tmux: only you or an actor with a token may type");
        else if (!pid) tried.push("tmux: no process known");
        else {
          const r = await this.tmux.deliver({ pid, text }).catch((err) => ({ ok: false, why: err.message }));
          this.policy.audit({ kind: "typed", message: m.id, from: m.from, to: rec.agent, pane: r.where ?? null, ok: r.ok, why: r.why ?? null, text: hashText(m.text) });
          if (r.ok) return success("tmux", { where: r.where });
          tried.push(`tmux: ${r.why}`);
        }
      }
    }
    Object.assign(rec, { state: "next-turn", how: "hooks", why: tried.join("; ") || null });
    this.emit("delivery", { ...rec });
    return false;
  }

  #arm(rec, m) {
    if (rec.ackedAt) return;
    const timer = setTimeout(async () => {
      if (rec.ackedAt) return;
      // Claude Code drops a push on a channel it didn't register (channels
      // off for the organization, say) without a word, and a typed line can
      // be lost. With no hook since, the message waits for the hooks again,
      // so the session's next turn still gets it.
      if (rec.how === "channel" || rec.how === "tmux") this.engine.store.markUnseen(rec.agent, [m.id]);
      rec.state = "notified";
      this.policy.audit({ kind: "notified", message: m.id, to: rec.agent });
      this.emit("delivery", { ...rec });
      const view = await this.engine.view(rec.agent).catch(() => null);
      this.notify({ message: m.id, agent: rec.agent, title: view?.title ?? rec.agent, from: rec.fromName ?? m.from });
    }, this.ackMs);
    timer.unref?.();
    this.timers.set(rec, timer);
  }

  #ack(rec) {
    if (rec.ackedAt) return;
    rec.ackedAt = this.now();
    clearTimeout(this.timers.get(rec));
    this.timers.delete(rec);
    this.emit("delivery", { ...rec });
  }

  #seen(agent, ids) {
    for (const id of ids) {
      const rec = this.records.get(`${id}|${agent}`);
      if (rec) this.#ack(rec);
    }
  }

  #contact(agent) {
    for (const rec of this.records.values()) if (rec.agent === agent && rec.state === "live" && !rec.ackedAt) this.#ack(rec);
  }
}
