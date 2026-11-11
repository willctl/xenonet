// Your devices and this machine's services, as the Network panel reads,
// shows and sends them (src/network-person.js): the person read leniently
// from network/state/v1, Revoke only on another active device, the root
// key's warning when it's in a file, and Publish a service's body from
// its fields.
import { test } from "node:test";
import assert from "node:assert/strict";
import { personOf, servicesOf, deviceLine, devicesHtml, servicesHtml, publishBody } from "../src/network-person.js";

const device = (id, label, extra = {}) => ({ id, label, kind: "weaker", active: true, here: false, ...extra });
const state = {
  person: {
    id: "prs_simulatedpersonofthehomenetwork2",
    owner: true,
    device: "xns1_here",
    kind: "biometric",
    acts: true,
    rootSealed: false,
    devices: [device("xns1_here", "studio", { kind: "biometric", here: true }), device("xns1_lost", "laptop"), device("xns1_gone", "rack", { active: false }), { label: "no id" }],
  },
};

test("the person is read leniently, and none before an identity is set up", () => {
  const p = personOf(state);
  assert.deepEqual(p.devices.map((d) => d.id), ["xns1_here", "xns1_lost", "xns1_gone"]);
  assert.equal(p.rootSealed, false);
  assert.equal(personOf({}), null);
  assert.equal(personOf({ person: { id: null } }), null);
  assert.deepEqual(servicesOf({ services: [{ name: "files", target: "http://127.0.0.1:1", paths: ["/a/"], hosts: [] }, { target: "x" }] }), [{ name: "files", target: "http://127.0.0.1:1", paths: ["/a/"], hosts: [] }]);
  assert.deepEqual(servicesOf(null), []);
});

test("Revoke shows on another active device only, and asks before it goes", () => {
  const p = personOf(state);
  const html = devicesHtml(p);
  assert.ok(html.includes('data-net-revoke="xns1_lost"'));
  assert.ok(!html.includes('data-net-revoke="xns1_here"'), "never this machine's own");
  assert.ok(!html.includes('data-net-revoke="xns1_gone"'), "nor one revoked already");
  assert.match(html, /data-net-root/, "a root in a file is said");
  const asking = devicesHtml(p, "xns1_lost");
  assert.match(asking, /Revoke laptop's device\?/);
  assert.match(asking, /data-net-revoke-yes="xns1_lost" data-yours/);
  assert.equal(deviceLine(p.devices[2]), "revoked: nothing it signs counts");
  // A machine that can't act as you offers no Revoke at all.
  assert.ok(!devicesHtml({ ...p, acts: false }).includes("data-net-revoke="));
  assert.equal(devicesHtml(null), "");
});

test("services list with Take off, and Publish's body from its fields", () => {
  assert.match(servicesHtml([]), /publishes no services/);
  assert.match(servicesHtml([{ name: "files", target: "http://127.0.0.1:1", paths: ["/a/"], hosts: ["laptop"] }]), /data-net-unpublish="files"[\s\S]*Publish a service/);
  assert.deepEqual(publishBody({ name: " files ", target: "http://127.0.0.1:8080 ", paths: "/sync/v1/, /api/", hosts: "" }), { name: "files", target: "http://127.0.0.1:8080", paths: ["/sync/v1/", "/api/"], hosts: [] });
});
