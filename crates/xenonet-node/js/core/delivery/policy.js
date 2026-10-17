// core/delivery/policy.js in Rust (crates/xenonet-engine/src/delivery/policy.rs):
// what delivery may do and the guards on it, scopes, the pause switch,
// limits, loop and storm checks, consent, the wake budget, redaction and
// the audit log. What's kept and every check are Rust's; the policy's
// clock, limits and files stay the twin's own properties, which a program
// may change, and node:fs.
import fs from "node:fs";
import { native } from "../../index.js";

export const LIMITS = native.policyLimits();

export const isYou = (id) => native.policyIsYou(id);

export const redactSecrets = (text) => native.policyRedact(String(text));

export const sanitize = (text, { oneLine = false, max = 4000 } = {}) => native.policySanitize(String(text ?? ""), !!oneLine, Number(max));

export const framed = ({ id, from, verified, text, user = false }) => native.policyFramed(id, from, verified, text, !!user);

export const hashText = (text) => native.policyHashText(String(text));

export class DeliveryPolicy {
  #core = new native.PolicyCore();
  #deps = { policy: this, fs };

  // `file`: where grants, consents and the pause switch persist (JSON).
  // `auditFile`: the append-only log. Both optional (in memory), as in tests.
  constructor({ file = null, auditFile = null, now = () => Date.now(), limits = {} } = {}) {
    this.file = file;
    this.auditFile = auditFile;
    this.now = now;
    this.limits = { ...LIMITS, ...limits };
    this.#core.load(this.#deps);
  }

  // What's kept, as it stands.
  get data() {
    return this.#core.data();
  }
  get memoryAudit() {
    return this.#core.memoryAudit();
  }

  get paused() {
    return this.#core.paused();
  }
  setPaused(on, by = "you") {
    return this.#core.setPaused(this.#deps, !!on, by);
  }

  grant(actor, selector, { ttlMs, by = "you" } = {}) {
    return this.#core.grant(this.#deps, actor, selector, ttlMs, by);
  }
  revoke(actor, by = "you") {
    return this.#core.revoke(this.#deps, actor, by);
  }
  grants() {
    return this.#core.grants(this.#deps);
  }
  scopeOf(id) {
    return this.#core.scopeOf(this.#deps, id);
  }
  visible(caller, facts) {
    return this.#core.visible(this.#deps, caller, facts);
  }
  checkSend({ sender, recipients, text, depth = 0 }) {
    this.#core.checkSend(this.#deps, sender, [...recipients], text, depth);
  }
  textFor({ text, sender, senderFacts, recipientFacts }) {
    return native.policyTextFor(text, sender, senderFacts, recipientFacts);
  }
  allow(session, { remote = false, ttlMs, by = "you" } = {}) {
    return this.#core.allow(this.#deps, session, !!remote, ttlMs, by);
  }
  disallow(session, by = "you") {
    this.#core.disallow(this.#deps, session, by);
  }
  consented(session, { remote = false } = {}) {
    return this.#core.consented(this.#deps, session, !!remote);
  }
  wakeAllowed(sender, target, { usage = null } = {}) {
    return this.#core.wakeAllowed(this.#deps, sender, target, usage);
  }
  spendWake(sender, target) {
    this.#core.spendWake(this.#deps, sender, target);
  }
  audit(entry) {
    return this.#core.audit(this.#deps, entry);
  }
  readAudit(limit = 100) {
    return this.#core.readAudit(this.#deps, limit);
  }
}
