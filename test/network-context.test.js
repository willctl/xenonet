// The shared context in the Network panel (src/network-context.js): what
// GET /api/net/forwards says, read leniently; each ask with a box per
// network, ticked; a grant's line; markup escaped; and Allow's body from
// the boxes ticked under its ask alone.
import { test } from "node:test";
import assert from "node:assert/strict";
import { contextOf, contextHtml, grantLine, approveBody } from "../src/network-context.js";

const KEY = "xns1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const N1 = "net_00000000000000000000000000000001";
const N2 = "net_00000000000000000000000000000002";
const networks = [
  { id: N1, name: "home" },
  { id: N2, name: "work" },
];

test("asks and grants are read leniently, and a grant attached now says so", () => {
  const c = contextOf({
    asks: [{ alpn: "xenolith/sync/2", key: KEY, actor: "xenolith", at: 5 }, { alpn: "x" }, null],
    grants: [{ actor: "xenolith", alpn: "xenolith/sync/2", key: KEY, networks: [N1, 7], until: 9, state: "granted" }, { actor: "x" }],
    forwards: [{ alpn: "xenolith/sync/2", actor: "xenolith", connections: [{}, {}] }],
  });
  assert.deepEqual(c.asks, [{ alpn: "xenolith/sync/2", key: KEY, actor: "xenolith", at: 5 }]);
  assert.deepEqual(c.grants.map((g) => [g.networks, g.attached, g.connections]), [[[N1], true, 2]]);
  assert.deepEqual(contextOf(null), { asks: [], grants: [] });
  assert.equal(contextHtml(contextOf(null), networks), "", "nothing to show, no section");
});

test("a grant's line names its networks and how it stands", () => {
  const g = { networks: [N1, N2], state: "granted", attached: true, connections: 1, until: null };
  assert.equal(grantLine(g, networks), "reaches home, work · syncing with 1 machine");
  assert.match(grantLine({ ...g, state: "lapsed" }, networks), /ended while it was away: it asks you again/);
  assert.match(grantLine({ ...g, state: "approved" }, networks), /collects its grant when it next starts/);
});

test("an ask shows a ticked box per network, escaped, and Allow sends the ones ticked under it", () => {
  const c = contextOf({ asks: [{ alpn: "xenolith/sync/2", key: KEY, actor: "<b>" }] });
  const html = contextHtml(c, networks);
  assert.ok(!html.includes("<b>"), "an actor's name is escaped");
  assert.equal((html.match(/data-net-ctx-net=/g) ?? []).length, 2);
  assert.match(html, /data-net-ctx-allow="xenolith\/sync\/2\|xns1_a+"/);
  const box = (net, checked) => ({ checked, dataset: { netCtxNet: `${KEY}|${net}` } });
  const other = { checked: true, dataset: { netCtxNet: `xns1_other|${N1}` } };
  const root = { querySelectorAll: () => [box(N1, true), box(N2, false), other] };
  assert.deepEqual(approveBody(root, `xenolith/sync/2|${KEY}`), { alpn: "xenolith/sync/2", key: KEY, networks: [N1] });
});
