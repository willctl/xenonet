// The shared context in the Network panel (src/network-context.js): what
// GET /api/net/forwards says, read leniently; each ask with a box per
// network, ticked; a grant's line; markup escaped; and Allow's body from
// the boxes ticked under its ask alone; each space with what can be signed
// for it, and Sign's body.
import { test } from "node:test";
import assert from "node:assert/strict";
import { contextOf, contextHtml, grantLine, approveBody, signBody } from "../src/network-context.js";

test("a space shows what you can sign for it, the project's networks first, and a narrowed one says so", () => {
  const c = contextOf({
    spaces: [
      {
        id: "spc_atlas",
        address: "project:atlas",
        owner: "person:prs_p",
        networks: [{ network: "net_00000000000000000000000000000002", level: "append" }],
        offers: [
          { op: "shareNetwork", network: "net_00000000000000000000000000000001", fromProject: true },
          { op: "unshareNetwork", network: "net_00000000000000000000000000000002", narrowed: true },
          { op: "take" },
        ],
      },
      { id: "spc_notes", address: "project:notes", weaker: true, offers: [{ op: "moveOwner" }] },
      { address: "no id" },
    ],
  });
  assert.deepEqual(c.spaces.map((s) => [s.id, s.offers.map((o) => o.op)]), [["spc_atlas", ["shareNetwork", "unshareNetwork"]], ["spc_notes", ["moveOwner"]]]);
  const html = contextHtml(c, [{ id: "net_00000000000000000000000000000001", name: "home" }, { id: "net_00000000000000000000000000000002", name: "work" }]);
  assert.match(html, /Share into home \(from the project\)/);
  assert.match(html, /Stop sharing into work/);
  assert.match(html, /isn&#39;t shared into work any more, so it&#39;s held back from there|isn't shared into work any more, so it's held back from there/);
  assert.match(html, /Make it yours/);
  assert.deepEqual(signBody("spc_atlas|shareNetwork|net_1"), { space: "spc_atlas", op: "shareNetwork", network: "net_1", level: "append" });
  assert.deepEqual(signBody("spc_notes|moveOwner|"), { space: "spc_notes", op: "moveOwner" });
});

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
  assert.deepEqual(contextOf(null), { asks: [], spaces: [], grants: [] });
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
