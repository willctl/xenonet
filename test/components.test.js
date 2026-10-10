import { test } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import { rpc } from "../core/components/rpc.js";
import { negotiate } from "../core/components/protocol.js";
import { consolePolicy } from "../core/components/console-policy.js";

test("components negotiate the highest shared interface, independent of releases", () => {
  assert.equal(negotiate({ min: 1, max: 3 }, { min: 1, max: 2 }), 2);
  assert.equal(negotiate({ min: 1, max: 1 }, { min: 1, max: 3 }), 1);
  assert.throws(() => negotiate({ min: 2, max: 3 }, { min: 1, max: 1 }), /incompatible/);
  assert.throws(() => negotiate({ min: 1, max: 1 }, undefined), /Update the service/);
});

test("production console refuses arbitrary execution and prototype methods", () => {
  for (const op of ["eval", "cdp"]) assert.throws(() => consolePolicy({ op }, true), /development/);
  assert.throws(() => consolePolicy({ op: "driver", method: "constructor" }, true), /Unknown/);
  assert.throws(() => consolePolicy({ op: "driver", method: "updates", args: ["install"] }, true), /Updates panel/);
  consolePolicy({ op: "driver", method: "terminal" }, true);
  consolePolicy({ op: "eval" }, false);
});

test("an interrupted component action fails without replaying it", async () => {
  let calls = 0;
  const server = net.createServer((socket) => rpc(socket, () => { calls++; socket.destroy(); }));
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const client = rpc(net.connect(server.address().port, "127.0.0.1"));
  try {
    await assert.rejects(client.request("write", { text: "once" }), /not retried/);
    assert.equal(calls, 1);
  } finally { client.close(); await new Promise((r) => server.close(r)); }
});

test("wire events cannot fake a disconnect or crash a component", async () => {
  const server = net.createServer((socket) => {
    const peer = rpc(socket, () => "alive");
    peer.notify("error", "untrusted");
    peer.notify("disconnect", null);
    peer.notify("notification", "expected");
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const client = rpc(net.connect(server.address().port, "127.0.0.1"), undefined, { incomingEvents: ["notification"] });
  let disconnected = false;
  client.on("disconnect", () => { disconnected = true; });
  const note = new Promise((r) => client.once("notification", r));
  try {
    assert.equal(await note, "expected");
    assert.equal(await client.request("status"), "alive");
    assert.equal(disconnected, false);
  } finally { client.close(); await new Promise((r) => server.close(r)); }
});
