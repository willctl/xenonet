// The delivery router: for each message and each session it's for, the
// fastest way in that's allowed, then whether it arrived.
//
//   live       into the session now: its Stop hook listening after a turn,
//              a Claude Code channel, an OpenCode server's prompt_async,
//              typed into its tmux pane, or a headless resume of a session
//              whose process has gone
//   next turn  waits for the session's hooks, as before (when it next starts
//              a turn, or during the one under way)
//   notified   nothing picked it up within ackMs, so XenoNet told you
//
// Acks: an OpenCode server's 204 is one; a channel, a typed message or a
// resumed turn is acked by the session's next hook after it; a message a
// listening Stop hook or the hooks hand over, by the engine's "seen". A
// push with no ack goes back to waiting for the hooks. Actors aren't routed
// here: they read their inbox.
//
// Waking a session spends its plan, so every wake is checked first: against
// the policy (pause switch, wake budget), and with reach.js, which decides
// what this machine lets the sender do (your consent); a message that may
// not wake anything still waits for the next turn.
import { EventEmitter } from "node:events";
import { framed, hashText, isYou } from "./policy.js";
import { decide, senderOf } from "./reach.js";
import { waysOf, served } from "./apps.js";

export const ACK_MS = 60e3;
const KEEP = 500;

export class DeliveryRouter extends EventEmitter {
  // `reach()` is this machine's start and wake settings ({ start, wake }).
  constructor({ engine, policy, channels = null, opencode = null, tmux = null, resume = null, notify = () => {}, usage = () => null, reach = () => ({}), now = () => Date.now(), ackMs = ACK_MS }) {
    super();
    Object.assign(this, { engine, policy, channels, opencode, tmux, resume, notify, usage, reach, now, ackMs });
    this.records = new Map(); // `${message}|${agent}` → record
    this.timers = new Map();
    engine.on("message", (m) => void this.route(m).catch((err) => this.emit("error", err)));
    engine.on("seen", ({ agent, ids }) => this.#seen(agent, ids));
    engine.on("contact", ({ agent, via }) => via === "hook" && this.#contact(agent));
  }

  // Whether a session can be reached without hooks (an OpenCode run XenoNet
  // has a way into), so the engine counts it as reachable.
  reaches(id) {
    return served(id) && !!this.opencode && this.policy.consented(id);
  }

  // The OpenCode sessions you allowed XenoNet to wake. XenoNet has no
  // transcript of them, so the engine counts these among the agents, and
  // they can be found by their titles and selectors before they've been in
  // touch. Only these: XenoNet doesn't list every OpenCode session it finds.
  known() {
    if (!this.opencode) return [];
    return Object.keys(this.policy.data?.consent ?? {}).filter((id) => served(id) && this.policy.consented(id));
  }

  // What its own OpenCode says of an OpenCode session (its title, folder,
  // last change, and whether a turn is under way), for the engine's view of
  // a session the map hasn't read; null for any other.
  async describe(id) {
    if (!served(id) || !this.opencode?.info) return null;
    return this.opencode.info(id).catch(() => null);
  }

  // Whether a session that ended can still be messaged: you allowed it (or
  // wake is network), and a headless resume would start it for the message.
  resumable(view) {
    return !!this.resume && !!view?.ended && this.resume.can(view.id) && (this.#wakeNetwork() || this.policy.consented(view.id));
  }

  // This machine's wake setting: network lets any sender wake a session,
  // within the budgets.
  #wakeNetwork() {
    return this.reach?.()?.wake === "network";
  }

  // Whether a message to a Claude Code session that a process holds (its
  // app, its terminal, or a run XenoNet resumed) and that isn't at work goes
  // to a copy of it, forked with its whole context: under wake network
  // only, since the session itself can't take it now.
  forkable(view) {
    if (!view || view.ended || view.status === "working" || !String(view.id).startsWith("claude:")) return false;
    if (!this.resume?.fork || !this.resume.can(view.id)) return false;
    return this.#wakeNetwork();
  }

  // A wake for a session's listening Stop hook to hand a message over (the
  // engine asks): the same budget as any other wake, spent when allowed.
  wake(from, agent) {
    const w = this.policy.wakeAllowed(from, agent, { usage: this.usage(String(agent).split(":")[0]) });
    if (w.ok) this.policy.spendWake(from, agent);
    return w;
  }

  // Whether this machine lets `from` wake `agent` (reach.js): your
  // allowance for the session, here and for actors.
  #mayWake(from, agent) {
    const consent = { here: !!this.policy.consented(agent), remote: !!this.policy.consented(agent, { remote: true }) };
    return decide("wake", senderOf(from), agent, { consent, wake: this.reach?.()?.wake });
  }

  // How a message from `from` would reach `a`, an agent as the engine lists
  // it, if sent now: for the sender's answer. null when it waits for the hooks.
  way(from, a) {
    const idle = a.status === "idle" || a.status === "ended";
    if (idle && !this.policy.wakeAllowed(from, a.id, { usage: this.usage(a.app) }).ok) return null;
    if (this.engine.listening?.has(a.id)) return "right away: it's listening for messages";
    if (waysOf(a.app)?.channel && a.status === "idle" && this.channels?.live(a.id)) return "right away, through its channel";
    if (a.status === "ended" && this.resumable(a) && this.#mayWake(from, a.id).ok) return "right away, in a turn XenoNet resumes it for";
    if (this.forkable(a) && this.#mayWake(from, a.id).ok) return "right away, in a copy XenoNet forks from it, since it's open elsewhere";
    if (waysOf(a.app)?.server && this.opencode && this.#mayWake(from, a.id).ok) return "right away, through its OpenCode server";
    return null;
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
    const fromName = m.fromName ?? (isYou(m.from) ? "your user" : (await this.engine.view(m.from).catch(() => null))?.title) ?? m.from;
    const text = framed({ id: m.id, from: fromName, verified: m.verified, text: m.text, user: isYou(m.from) });
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
    // 0. Its Stop hook, listening after a turn: the hook hands the message
    // over itself, and checks the wake then, when it keeps the session going.
    if (this.engine.listening?.has(rec.agent)) {
      Object.assign(rec, { state: "live", how: "listen" });
      this.emit("delivery", { ...rec });
      return true;
    }
    if (!wake.ok) tried.push(`no wake: ${wake.why}`);
    else {
      // 1. A Claude Code channel: the session opted in by starting with it.
      // Only to wake an idle one: a working session gets the message from its
      // hooks during the turn under way, sooner than from a channel, which
      // Claude Code holds until that turn ends.
      if (waysOf(app)?.channel && idle && this.channels?.live(rec.agent)) {
        if (this.channels.push(rec.agent, { id: m.id, from: fromName, text })) return success("channel");
        tried.push("channel closed");
      }
      // 2. OpenCode's own server, with your consent for that session.
      if (waysOf(app)?.server && this.opencode) {
        const may = this.#mayWake(m.from, rec.agent);
        if (!may.ok) tried.push(`opencode: ${may.why}`);
        else {
          const r = await this.opencode.deliver(rec.agent, text).catch((err) => ({ ok: false, why: err.message }));
          if (r.ok) return success("opencode", { ackedAt: this.now(), where: r.where });
          tried.push(`opencode: ${r.why}`);
        }
      }
      // 3. Typing into its tmux pane: consent, a sender XenoNet verified with
      // a token or you, and an idle session that isn't waiting on its user.
      if (this.tmux && idle && !view?.ended && !view?.needsYou) {
        // Only you, or an actor this machine checked by its token.
        const who = senderOf(m.from);
        const sender = who.kind === "you" || (who.kind === "actor" && !who.host);
        const pid = this.engine.store.contact?.(rec.agent)?.pid ?? null;
        const may = this.#mayWake(m.from, rec.agent);
        if (!may.ok) tried.push(`tmux: ${may.why}`);
        else if (!sender) tried.push("tmux: only you or an actor with a token may type");
        else if (!pid) tried.push("tmux: no process known");
        else {
          const r = await this.tmux.deliver({ pid, text }).catch((err) => ({ ok: false, why: err.message }));
          this.policy.audit({ kind: "typed", message: m.id, from: m.from, to: rec.agent, pane: r.where ?? null, ok: r.ok, why: r.why ?? null, text: hashText(m.text) });
          if (r.ok) return success("tmux", { where: r.where });
          tried.push(`tmux: ${r.why}`);
        }
      }
      // 4. A headless resume, for a Claude Code or Codex session whose
      // process has gone, with your consent: never while it runs.
      if (this.resume && view?.ended && this.resume.can(rec.agent)) {
        const may = this.#mayWake(m.from, rec.agent);
        if (!may.ok) tried.push(`resume: ${may.why}`);
        else {
          const r = await this.resume.deliver({ agent: rec.agent, cwd: view.cwd, text }).catch((err) => ({ ok: false, why: err.message }));
          this.policy.audit({ kind: "resumed", message: m.id, from: m.from, to: rec.agent, ok: r.ok, why: r.why ?? null, text: hashText(m.text) });
          if (r.ok) return success("resume", { where: r.where });
          tried.push(`resume: ${r.why}`);
        }
      } else if (this.forkable(view)) {
        // 5. A copy of a Claude Code session a process holds, under wake
        // network: it answers with the whole context, now.
        const may = this.#mayWake(m.from, rec.agent);
        if (!may.ok) tried.push(`fork: ${may.why}`);
        else {
          const r = await this.resume.deliver({ agent: rec.agent, cwd: view.cwd, text, fork: true }).catch((err) => ({ ok: false, why: err.message }));
          this.policy.audit({ kind: "forked", message: m.id, from: m.from, to: rec.agent, ok: r.ok, why: r.why ?? null, text: hashText(m.text) });
          if (r.ok) return success("fork", { where: r.where });
          tried.push(`fork: ${r.why}`);
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
      // off for the organization, say) without a word, a typed line can be
      // lost, and a resumed turn can fail to start. With no hook since, the
      // message waits for the hooks again, so the session's next turn still
      // gets it.
      if (["channel", "tmux", "resume"].includes(rec.how)) this.engine.store.markUnseen(rec.agent, [m.id]);
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

  // A hook from the session after a push acks it; a message waiting for
  // its listening Stop hook is acked when the hook takes it (#seen).
  #contact(agent) {
    for (const rec of this.records.values()) if (rec.agent === agent && rec.state === "live" && rec.how !== "listen" && !rec.ackedAt) this.#ack(rec);
  }
}
