// What Token use shows of each actor's spend today (src/actor-spend-view.js):
// the top few, the rest folded until it's opened, what no actor is tied to
// last, and on hover what each row covers, never a cost XenoNet can't see.
import { test } from "node:test";
import assert from "node:assert/strict";
import { actorSpendView, TOP } from "../src/actor-spend-view.js";

const actor = (n, cost, extra = {}) => ({ id: `actor:a${n}`, name: `Agent ${n}`, cost, tokens: 1000, sessions: 1, unpriced: [], elsewhere: 0, ...extra });
const answer = (actors, rest = 4) => ({ total: actors.reduce((a, r) => a + r.cost, 0) + rest, actors, unattributed: { cost: rest, tokens: 500, sessions: 6, unpriced: [] } });

test("nothing to show without an answer, or with no actor's work today", () => {
  assert.equal(actorSpendView(null), null);
  assert.equal(actorSpendView({ openrouter: {} }.actors), null, "a XenoNet from before it");
  assert.equal(actorSpendView(answer([])), null, "only unattributed work says nothing the estimate doesn't");
});

test("the top few, the rest under more until it's opened, and the unattributed rest last", () => {
  const a = answer([actor(1, 4), actor(2, 2, { sessions: 3 }), actor(3, 1), actor(4, 0.5), actor(5, 0.25)], 2.25);
  const v = actorSpendView(a);
  assert.equal(TOP, 3);
  assert.deepEqual(v.rows.map((r) => [r.name, r.value]), [["Agent 1", "$4.00"], ["Agent 2", "$2.00"], ["Agent 3", "$1.00"], ["unattributed", "$2.25"]]);
  assert.equal(v.more, 2);
  assert.equal(v.rows[0].share, 0.4);
  assert.equal(v.rows.at(-1).unattributed, true);
  const open = actorSpendView(a, { expanded: true });
  assert.equal(open.more, 0);
  assert.deepEqual(open.rows.map((r) => r.name), ["Agent 1", "Agent 2", "Agent 3", "Agent 4", "Agent 5", "unattributed"]);
  assert.match(v.rows[1].title, /^Agent 2: About \$2\.00 today over 3 sessions XenoNet ties to it\.$/);
  assert.match(v.rows.at(-1).title, /^About \$2\.25 today over 6 sessions not tied to an actor, such as the ones you start/);
});

test("a run reported from elsewhere is said, never priced, and models without a price are named", () => {
  const v = actorSpendView(answer([actor(1, 3, { unpriced: ["gpt-made-up"] }), actor(2, 0, { sessions: 0, elsewhere: 2 })]));
  assert.equal(v.rows[1].value, "—");
  assert.equal(v.rows[1].title, "Agent 2: 2 runs it reported from elsewhere today aren't counted: XenoNet can't see their calls.");
  assert.match(v.rows[0].title, /Not counted, with no price: gpt-made-up\.$/);
  const money = (x) => `${x} dollars`;
  assert.equal(actorSpendView(answer([actor(1, 3)]), { money }).rows[0].value, "3 dollars");
});
