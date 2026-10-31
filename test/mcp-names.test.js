// Names for shared systems whose ids say nothing (core/mcp-names.js): read
// from the instructions a server hands a session and from its own results,
// never from a table of services. Made-up services and uuids throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { isOpaque, shortName, labelOf, leadOf, detect, ServerNames } from "../core/mcp-names.js";
import { mcpServerOf } from "../core/live.js";

const ID = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";
const OTHER = "a9d3f0b2-6e41-4c57-8b12-77c0e5a49d36";

const toolUse = (id, server, tool = "search") => ({ type: "assistant", message: { content: [{ type: "tool_use", id, name: `mcp__${server}__${tool}`, input: {} }] } });
const toolResult = (id, text) => ({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: id, content: text }] } });
const instructions = (server, body) => ({ type: "attachment", attachment: { type: "mcp_instructions_delta", addedNames: [server], addedBlocks: [`## ${server}\n${body}`], removedNames: [] } });
const names = (...lines) => {
  const n = new ServerNames();
  for (const l of lines) n.observe(l, mcpServerOf);
  return n;
};
// Results of a made-up music service: its own links and name, and many more
// links to the CDN that serves its covers.
const tracks = (n = 6) => Array.from({ length: n }, (_, i) => `{"name":"Song ${i}","uri":"tunebox:track:${i}","url":"https://open.tunebox.com/track/${i}","cover":"https://i.tbcdn.co/image/${i}a","thumb":"https://i.tbcdn.co/image/${i}b","note":"Tunebox track ${i}"}`).join("\n");

test("an id that is a uuid or a long hex token is opaque; a name someone gave is not", () => {
  assert.equal(isOpaque(ID), true);
  assert.equal(isOpaque("0123456789abcdef0123456789abcdef"), true);
  for (const named of ["xenonet", "Claude_Browser", "plugin:data:amplitude", "firefox", "a1b2c3d4", "", null, undefined, 7]) assert.equal(isOpaque(named), false, String(named));
  assert.equal(shortName(ID), "Connector 5c1e4a90");
});

test("a host's label is the part that names its owner", () => {
  assert.equal(labelOf("open.tunebox.com"), "tunebox");
  assert.equal(labelOf("news.bbc.co.uk"), "bbc");
  assert.equal(labelOf("TuneBox.COM"), "tunebox");
  for (const not of ["localhost", "127.0.0.1", "10.0.0.8", "co.uk", "a.b", "bucket.s3.amazonaws.com", "d111.cloudfront.net", "my-site.com", ""]) assert.equal(labelOf(not), null, not);
});

test("instructions name a server by their heading or the words before a colon, and not by a sentence", () => {
  assert.equal(leadOf("# Brightdesk `exec` covers\n\naction|alert"), "Brightdesk");
  assert.equal(leadOf("Docs Studio: living docs you create and edit here."), "Docs Studio");
  assert.equal(leadOf("# Notebook tools"), "Notebook");
  assert.equal(leadOf("\n\n# Xenolith protocol"), "Xenolith");
  assert.equal(leadOf("Docs Studio \u2014 living docs"), "Docs Studio");
  assert.equal(leadOf("Foo Bar Baz Qux: four words"), null);
  for (const sentence of ["You have a computer-use MCP available.", "Use this server to fetch documentation.", "Note: prefer the dedicated tool.", "# Tools", "lowercase heading", "", "A: b", "Docs:"]) assert.equal(leadOf(sentence), null, sentence);
});

test("a service is named by the domain its own results also say in words, not by its CDN", () => {
  const n = names(toolUse("t1", ID), toolResult("t1", tracks()));
  assert.deepEqual(n.name(ID), { name: "Tunebox", source: "results" });
});

test("the spelling the service is written with wins over the host's lowercase", () => {
  const body = Array.from({ length: 4 }, (_, i) => `BrightDesk ticket ${i}: https://app.brightdesk.com/t/${i}`).join("\n");
  assert.deepEqual(names(toolUse("t1", ID), toolResult("t1", body)).name(ID), { name: "BrightDesk", source: "results" });
});

test("evidence adds up over calls, and a name needs the domain in prose, not just in links", () => {
  const n = names(toolUse("t1", ID), toolResult("t1", "see https://open.tunebox.com/track/1"), toolUse("t2", ID), toolResult("t2", "Tunebox says so, https://open.tunebox.com/track/2"));
  assert.equal(n.name(ID), null, "one mention in words is not enough");
  n.observe(toolUse("t3", ID), mcpServerOf);
  n.observe(toolResult("t3", "Tunebox, Tunebox again: https://open.tunebox.com/track/3"), mcpServerOf);
  assert.equal(n.name(ID)?.name, "Tunebox");
});

test("only the results of calls to an opaque server count, matched by the call's id", () => {
  const n = names(toolUse("t1", "xenonet", "send"), toolResult("t1", tracks()), toolResult("never-called", tracks()), toolUse("t2", ID), toolResult("t2", "no links, no name"));
  assert.equal(n.name("xenonet"), null);
  assert.equal(n.name(ID), null);
  assert.equal(names(toolUse("t1", ID), toolResult("t1", [{ type: "text", text: tracks() }])).name(ID)?.name, "Tunebox", "a result's text blocks are read too");
});

test("instructions that open with a name outrank what the results say", () => {
  const n = names(instructions(ID, "Docs Studio: living docs you create and edit here."), toolUse("t1", ID), toolResult("t1", tracks()));
  assert.deepEqual(n.name(ID), { name: "Docs Studio", source: "instructions" });
  const sentence = names(instructions(ID, "Use this server to fetch documentation."), toolUse("t1", ID), toolResult("t1", tracks()));
  assert.equal(sentence.name(ID)?.source, "results", "a sentence names nobody, so the results decide");
});

test("a server's name stays its own: another's evidence and readable ids are not mixed in", () => {
  const n = names(instructions("firefox", "Browser: drives Firefox."), instructions(OTHER, "# Mailroom"), toolUse("t1", ID), toolResult("t1", tracks()));
  assert.equal(n.name("firefox"), null);
  assert.equal(n.name(OTHER)?.name, "Mailroom");
  assert.equal(n.name(ID)?.name, "Tunebox");
});

test("the evidence kept is bounded, and odd lines are skipped without throwing", () => {
  const n = new ServerNames();
  for (const bad of [null, undefined, 7, "x", {}, { message: {} }, { message: { content: "text" } }, { type: "attachment", attachment: { type: "mcp_instructions_delta", addedBlocks: [1, null, "no header"] } }, { message: { content: [null, 3, { type: "tool_use" }, { type: "tool_result", tool_use_id: {} }] } }]) n.observe(bad, mcpServerOf);
  assert.equal(n.name(ID), null);
  const many = Array.from({ length: 40 }, (_, i) => `https://h${i}.site${"abcdefghijklmnopqrstuvwxyz"[i % 26]}${i}.com/x Site${i}`).join("\n");
  n.observe(toolUse("t1", ID), mcpServerOf);
  n.observe(toolResult("t1", many), mcpServerOf);
  assert.equal(n.name(ID), null);
});

test("detect: no evidence is no name, and ties break by name so the answer doesn't flip", () => {
  assert.equal(detect(null), null);
  assert.equal(detect({ lead: new Map(), labels: new Map() }), null);
  const c = () => ({ urls: 2, prose: 4, forms: new Map() });
  assert.equal(detect({ labels: new Map([["zeta", c()], ["alpha", c()]]) }).name, "Alpha");
  assert.equal(detect({ lead: new Map([["Zed", 1], ["Abe", 1]]) }).name, "Abe");
});
