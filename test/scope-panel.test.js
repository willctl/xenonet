// What an actor's grant reaches, as the Actors panel says it and sets it
// (src/scope-panel.js): its sessions scope and its allow scope in words,
// the Scope view's fields filled from them, and the calls its Sets make,
// with an allow scope's end kept while its days aren't changed, so
// narrowing it asks nothing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { grantWords, allowsWords, scopeDraft, grantBody, allowsBody } from "../src/scope-panel.js";

const NOW = 1_800_000_000_000;
const DAY = 24 * 3600e3;
const grant = { actor: "actor:kite", selector: "project:xenonet", at: NOW - DAY, expiresAt: NOW + 29.5 * DAY, expired: false };
const allows = { sessions: "all", kinds: ["edit", "command"], until: NOW + 6.5 * DAY };

test("each scope in words, and none when it has ended", () => {
  assert.equal(grantWords(grant, NOW), "reaches project:xenonet · 30 days left");
  assert.equal(grantWords({ ...grant, expired: true }, NOW), "reaches no sessions");
  assert.equal(grantWords(null, NOW), "reaches no sessions");
  assert.equal(allowsWords(allows, NOW), "allows edits, commands in every session · 7 days left");
  assert.equal(allowsWords({ ...allows, sessions: ["claude:a"] }, NOW), "allows edits, commands in 1 session · 7 days left");
  assert.equal(allowsWords({ ...allows, until: NOW - 1 }, NOW), "allows no prompts, only denies");
});

test("the Scope view's fields start from the scopes there are, else the defaults", () => {
  assert.deepEqual(scopeDraft({ grant, allows: { ...allows, sessions: ["claude:a", "codex:b"] } }, NOW), { selector: "project:xenonet", sessionDays: 30, allowAll: false, allowList: "claude:a codex:b", kinds: ["edit", "command"], allowDays: 7 });
  assert.deepEqual(scopeDraft({ grant: null, allows: null }, NOW), { selector: "", sessionDays: 30, allowAll: true, allowList: "", kinds: [], allowDays: 7 });
});

test("a sessions scope needs a selector and 1 to 90 days, and goes as hours", () => {
  const d = scopeDraft({ grant, allows: null }, NOW);
  assert.deepEqual(grantBody("actor:kite", d), { body: { actor: "actor:kite", scope: "project:xenonet", hours: 720 } });
  assert.match(grantBody("actor:kite", { ...d, selector: "" }).error, /which sessions/);
  assert.match(grantBody("actor:kite", { ...d, sessionDays: 91 }).error, /1 to 90 days/);
});

test("an allow scope keeps its end while its days stay, so taking a kind away narrows it", () => {
  const d = scopeDraft({ allows }, NOW);
  assert.deepEqual(allowsBody({ ...d, kinds: ["edit"] }, { allows }, NOW), { body: { sessions: "all", kinds: ["edit"], until: allows.until } });
  assert.deepEqual(allowsBody({ ...d, allowDays: 3 }, { allows }, NOW).body.until, NOW + 3 * DAY);
  assert.deepEqual(allowsBody({ ...d, allowAll: false, allowList: "claude:a  codex:b " }, {}, NOW).body, { sessions: ["claude:a", "codex:b"], kinds: ["edit", "command"], until: NOW + 7 * DAY });
  assert.match(allowsBody({ ...d, kinds: [] }, {}, NOW).error, /kinds of prompt/);
  assert.match(allowsBody({ ...d, allowAll: false, allowList: "" }, {}, NOW).error, /which sessions/);
});
