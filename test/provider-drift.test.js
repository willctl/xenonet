// The provider drift check (scripts/provider-drift/): its registry covering
// every vendor source, the comparison of a provider's types with XenoNet's
// schemas, the ranges it widens, the stand-in API, the made-up home, and
// the report and tracking issue it makes. Made-up data only; nothing here
// installs a package or runs a provider's CLI.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { vendorSources } from "../core/schema/lenient.js";
import { satisfies } from "../core/vendor.js";
import { PROVIDERS, targetsOf } from "../scripts/provider-drift/registry.mjs";
import { REPO, xenonetSchema } from "../scripts/provider-drift/common.mjs";
import { compare, compareAll, nodeAt, schemaFromDts, tsToSchema, declarationsOf, functionsOf, methodsOf, codexMethodsIn, importsFrom } from "../scripts/provider-drift/types.mjs";
import { widenRange, setVersionsInSource, setVersionsInDocs } from "../scripts/provider-drift/ranges.mjs";
import { newestInPackages, newestInAppcast, versionInJson, releaseVersion, npmDistTags } from "../scripts/provider-drift/releases.mjs";
import { rangeFindings, dedupe, buildReport, fixPlan, renderMarkdown, fingerprint, issueAction, issueBody, hasDrift } from "../scripts/provider-drift/report.mjs";
import { readers, checkLines, CLAUDE_LINES, CODEX_LINES } from "../scripts/provider-drift/checks.mjs";
import { transcriptExpectations } from "../scripts/provider-drift/inspect-claude.mjs";
import { rolloutExpectations, codexConstants, codexConfig } from "../scripts/provider-drift/inspect-codex.mjs";
import { anthropicReply, openaiReply, startFakeApi } from "../scripts/provider-drift/fake-api.mjs";
import { makeSandbox } from "../scripts/provider-drift/sandbox.mjs";

const read = (rel) => fs.readFileSync(path.join(REPO, rel), "utf8");

// ---------- the registry ----------

test("every vendor source belongs to one provider, and every mapping points at a place in XenoNet's schema", () => {
  const owners = new Map();
  for (const p of PROVIDERS) for (const s of p.sources) owners.set(s, [...(owners.get(s) ?? []), p.id]);
  for (const source of vendorSources()) assert.deepEqual(owners.get(source)?.length, 1, `${source} belongs to one provider (${owners.get(source) ?? "none"})`);
  assert.deepEqual([...owners.keys()].sort(), vendorSources(), "and no provider claims a source with no schema");
  for (const p of PROVIDERS) {
    assert.ok(p.inspect || p.notInspected, `${p.id} is inspected, or says why not`);
    for (const s of p.rangeSources ?? []) assert.ok(p.sources.includes(s), `${p.id}'s range source ${s} is its own`);
    for (const pair of p.types ?? []) {
      assert.ok(p.sources.includes(pair.source), `${p.id}: ${pair.provider} maps to its own source`);
      const schema = xenonetSchema(pair.source);
      assert.ok(nodeAt(schema, pair.xenonet, schema), `${pair.source} has ${pair.xenonet.join(" ") || "a root"}`);
    }
  }
});

test("the versions to inspect: release tags in order, then pins, each version once with all its labels", () => {
  const p = { release: { tags: ["latest", "stable"] } };
  assert.deepEqual(targetsOf(p, { latest: "2.1.292", stable: "2.1.285", next: "2.1.292" }, [{ label: "the VM lab", version: "2.1.285" }, { label: "unknown", version: null }]), [
    { version: "2.1.292", labels: ["latest"] },
    { version: "2.1.285", labels: ["stable", "the VM lab"] },
  ]);
  assert.deepEqual(targetsOf({}, null, []), []);
});

// ---------- types ----------

const XENONET = {
  type: "object",
  properties: {
    id: { type: "string" },
    when: { type: ["number", "null"] },
    git: { anyOf: [{ type: "object", properties: { branch: { type: ["string", "null"] }, origin_url: { type: "string" } }, "x-xenonet": { closed: true, known: ["sha"] } }, { type: "null" }] },
    item: {
      oneOf: [
        { type: "object", properties: { type: { type: "string", const: "text" }, text: { type: "string" } }, required: ["type", "text"] },
        { type: "object", properties: { type: { type: "string", const: "old" }, body: { type: "string" } }, required: ["type"] },
        { type: "object", properties: { type: { type: "string", const: "skipped" } }, required: ["type"] },
      ],
      "x-xenonet": { discriminator: "type" },
    },
  },
  required: ["id"],
};
const THEIRS = {
  definitions: {
    Thing: { type: "object", properties: { id: { type: "integer" }, when: { $ref: "#/definitions/Seconds" }, extra: { type: "string" } }, required: [] },
    Seconds: { type: "integer", format: "int64" },
    Git: { type: "object", properties: { branch: { type: ["string", "null"] }, sha: { type: "string" }, originUrl: { type: "string" }, dirty: { type: "boolean" } } },
    Item: {
      oneOf: [
        { type: "object", properties: { type: { type: "string", enum: ["text"] }, text: { allOf: [{ $ref: "#/definitions/Text" }] } }, required: ["type", "text"] },
        { type: "object", properties: { type: { type: "string", enum: ["skipped"] } } },
        { type: "object", properties: { type: { type: "string", enum: ["image"] }, url: { type: "string" } } },
        { type: "object", properties: { type: { type: "string", enum: ["other"] } } },
      ],
    },
    Text: { type: "string" },
  },
};
const kinds = (findings) => findings.map((f) => [f.severity, f.kind, f.path, f.variant]);

test("a provider's types against XenoNet's: a field gone, retyped or now optional is a change; one a closed object doesn't list is an addition", () => {
  const thing = compare(XENONET, XENONET, THEIRS.definitions.Thing, THEIRS, {});
  assert.deepEqual(kinds(thing), [
    ["change", "retyped", "id", null],
    ["change", "now-optional", "id", null],
    ["change", "removed", "git", null],
    ["change", "removed", "item", null],
  ]);
  assert.match(thing[0].detail, /as string; the types say number/);
  // An open object takes new fields without a word; a closed one lists them.
  assert.deepEqual(kinds(compare(XENONET.properties.git, XENONET, THEIRS.definitions.Git, THEIRS, { path: "git" })), [
    ["change", "removed", "git.origin_url", null],
    ["addition", "added", "git.originUrl", null],
    ["addition", "added", "git.dirty", null],
  ]);
  assert.deepEqual(kinds(compare(XENONET.properties.git, XENONET, THEIRS.definitions.Git, THEIRS, { path: "git", legacy: ["origin_url"], untyped: ["dirty"] })), [["addition", "added", "git.originUrl", null]], "legacy and untyped fields aren't drift");
});

test("kinds of record: a new one is an addition, one gone is a note, a catch-all can be ignored, and each kind's fields are compared", () => {
  const found = compare(XENONET.properties.item, XENONET, THEIRS.definitions.Item, THEIRS, { path: "item", ignoreVariants: ["other"] });
  assert.deepEqual(kinds(found), [
    ["addition", "new-variant", "item.type", null],
    ["info", "variant-gone", "item.type", null],
  ]);
  assert.match(found[0].detail, /"image"/);
  assert.match(found[1].detail, /"old".*older versions/);
  // A field of one kind of record.
  const narrower = structuredClone(THEIRS);
  narrower.definitions.Text = { type: "number" };
  assert.deepEqual(kinds(compare(XENONET.properties.item, XENONET, narrower.definitions.Item, narrower, { path: "item", ignoreVariants: ["other", "image"] })), [
    ["change", "retyped", "item.text", ["text"]],
    ["info", "variant-gone", "item.type", null],
  ]);
});

test("fields XenoNet reads from every kind of a union are gone only when no kind has them", () => {
  const mine = {
    oneOf: ["call", "output"].map((t) => ({ type: "object", properties: { type: { const: t }, name: { type: "string" }, output: {} }, "x-xenonet": { closed: true } })),
    "x-xenonet": { discriminator: "type" },
  };
  const theirs = { oneOf: [{ type: "object", properties: { type: { enum: ["call"] }, name: { type: "string" } } }, { type: "object", properties: { type: { enum: ["output"] }, output: {} } }] };
  assert.deepEqual(kinds(compare(mine, mine, theirs, theirs, { shared: ["name", "output"] })), []);
  const gone = { oneOf: [{ type: "object", properties: { type: { enum: ["call"] } } }, { type: "object", properties: { type: { enum: ["output"] }, output: {} } }] };
  assert.deepEqual(kinds(compare(mine, mine, gone, gone, { shared: ["name", "output"] })), [
    ["change", "removed", "name", ["call"]],
    ["change", "removed", "name", ["output"]],
  ]);
});

test("a TypeScript declaration as JSON Schema: fields, optional ones, literals, unions and the types it names", () => {
  const dts = `
    /** A session. */
    export declare type Info = {
      /** Its id. */
      sessionId: string;
      when?: number;
      kind: 'user' | 'assistant';
      parent: string | null;
      tags: string[];
      id: UUID;
      nested: { a: string; b?: number };
      run: (x: number) => Promise<void>;
    };
    declare type UUID = \`\${string}-\${string}\`;
    export interface Options extends Base { dir?: string }
    export declare function listSessions(_options?: Options): Promise<Info[]>;
    export function renameSession(id: string, title: string): Promise<void>;
  `;
  const decls = declarationsOf(dts);
  assert.deepEqual([...decls.keys()].sort(), ["Info", "Options", "UUID"]);
  const { definitions } = schemaFromDts(dts, ["Info", "Options", "Missing"]);
  assert.deepEqual(Object.keys(definitions), ["Info", "Options"]);
  assert.deepEqual(definitions.Info.required, ["sessionId", "kind", "parent", "tags", "id", "nested", "run"]);
  assert.deepEqual(definitions.Info.properties.kind, { enum: ["user", "assistant"] });
  assert.deepEqual(definitions.Info.properties.parent, { anyOf: [{ type: "string" }, { type: "null" }] });
  assert.deepEqual(definitions.Info.properties.tags, { type: "array", items: {} });
  assert.deepEqual(definitions.Info.properties.id, { type: "string" });
  assert.equal(definitions.Info.properties.nested.type, "object");
  assert.deepEqual(definitions.Options.properties, { dir: { type: "string" } });
  assert.deepEqual(tsToSchema("Promise<void>", decls), {});
  assert.deepEqual(functionsOf(dts), ["listSessions", "renameSession"]);
});

test("a provider object told apart by a union of literals compares with XenoNet's kinds of record", () => {
  const mine = {
    oneOf: ["user", "assistant"].map((t) => ({ type: "object", properties: { type: { type: "string", const: t }, uuid: { type: "string" }, timestamp: { type: "string" } }, required: ["type", "uuid"], "x-xenonet": { closed: true, known: ["session_id"] } })),
    "x-xenonet": { discriminator: "type" },
  };
  const { definitions } = schemaFromDts("export declare type Message = { type: 'user' | 'assistant' | 'system'; uuid: string; session_id: string; parent_agent_id: string | null; };", ["Message"]);
  const doc = { definitions };
  assert.deepEqual(
    kinds(compareAll([{ source: "s", xenonet: [], provider: "Message", untyped: ["timestamp"] }], { xenonet: () => mine, provider: () => doc, source: "s" })),
    [
      ["addition", "new-variant", "type", null],
      ["addition", "added", "parent_agent_id", ["user"]],
      ["addition", "added", "parent_agent_id", ["assistant"]],
    ],
  );
  const missing = compareAll([{ source: "s", xenonet: [], provider: "Gone" }], { xenonet: () => mine, provider: () => doc, source: "s" });
  assert.deepEqual(kinds(missing), [["change", "removed", null, null]]);
});

test("the methods XenoNet calls, the ones the protocol declares, and what XenoNet imports from a package", () => {
  const clientRequest = {
    oneOf: [
      { type: "object", properties: { method: { type: "string", enum: ["thread/list"] }, params: { $ref: "#/definitions/ThreadListParams" } } },
      { type: "object", properties: { method: { const: "initialize" }, params: { $ref: "#/definitions/InitializeParams" } } },
    ],
  };
  assert.deepEqual([...methodsOf(clientRequest)], [
    ["thread/list", "ThreadListParams"],
    ["initialize", "InitializeParams"],
  ]);
  const code = `this.ready = this.#call("initialize", {}); await client.request("thread/list", {}); request("config/read"); client.request("console", body);`;
  assert.deepEqual(codexMethodsIn([code]), ["config/read", "initialize", "thread/list"]);
  assert.deepEqual(importsFrom([`import { listSessions, getSessionMessages as read } from "@anthropic-ai/claude-agent-sdk";`, `import { x } from "other";`], "@anthropic-ai/claude-agent-sdk"), ["getSessionMessages", "listSessions"]);
  // XenoNet's own, as they are today.
  assert.ok(codexMethodsIn([read("core/codex.js")]).includes("thread/list"));
  assert.ok(importsFrom([read("core/claude.js")], "@anthropic-ai/claude-agent-sdk").includes("listSessions"));
  const constants = codexConstants();
  assert.ok(Array.isArray(constants.sourceKinds) && constants.sourceKinds.includes("cli"));
  assert.equal(constants.stateDb, 5);
});

// ---------- ranges ----------

test("a range widened to take a version keeps every version it had", () => {
  assert.equal(widenRange(">=0.3.0 <0.4.0", "0.4.2"), ">=0.3.0 <0.5.0");
  assert.equal(widenRange(">=0.3.0 <0.4.0", "0.3.9"), ">=0.3.0 <0.4.0", "one it has already");
  assert.equal(widenRange("<1.0.0", "1.0.3"), "<2.0.0");
  assert.equal(widenRange(">=1.0.0 <3.0.0", "3.1.0"), ">=1.0.0 <4.0.0");
  assert.equal(widenRange(">=1.0.0 <3.0.0", "0.9.5"), ">=0.9.0 <3.0.0", "an older host's version too");
  assert.equal(widenRange("<0.3.7", "0.3.9"), "<0.3.10");
  assert.equal(widenRange("<1.0.0", "1.0.0-alpha.3"), "<1.0.0", "a prerelease counts as the version it leads to");
  for (const [range, version] of [["5", "6"], ["2 || 3", "4"], ["<=31", "32"], ["^0.3.0", "0.4.0"], ["*", "not a version"]]) assert.equal(widenRange(range, version), null, `${range} is a person's to widen`);
  for (const [range, version] of [[">=0.3.0 <0.4.0", "0.4.2"], ["<1.0.0", "1.2.0"]]) {
    const wider = widenRange(range, version);
    assert.ok(satisfies(version, wider) && satisfies("0.3.0", wider));
  }
});

test("the range is changed in the schema's definition and in the docs' table, and only there", () => {
  const sdk = read("core/schema/vendor/anthropic/agent-sdk.js");
  const listing = setVersionsInSource(sdk, "anthropic/agent-sdk/session-listing", ">=0.3.0 <0.4.0", ">=0.3.0 <0.5.0");
  assert.ok(listing);
  const ranges = (text) => [...text.matchAll(/versions:\s*"([^"]*)"/g)].map((m) => m[1]);
  assert.deepEqual(ranges(listing), [">=0.3.0 <0.5.0", ">=0.3.0 <0.4.0"], "the listing's, not the messages'");
  assert.deepEqual(ranges(setVersionsInSource(sdk, "anthropic/agent-sdk/session-message", ">=0.3.0 <0.4.0", ">=0.3.0 <0.5.0")), [">=0.3.0 <0.4.0", ">=0.3.0 <0.5.0"]);
  assert.equal(setVersionsInSource(sdk, "anthropic/agent-sdk/session-listing", "<9.0.0", "<10.0.0"), null, "not when it says another range");
  assert.equal(setVersionsInSource(sdk, "anthropic/agent-sdk/nothing", ">=0.3.0 <0.4.0", "x"), null);
  const docs = read("docs/design/schemas.md");
  const changed = setVersionsInDocs(docs, "anthropic/agent-sdk/session-listing", ">=0.3.0 <0.4.0", ">=0.3.0 <0.5.0");
  const row = (text, source) => text.split("\n").find((l) => l.startsWith(`| \`${source}\` |`));
  assert.ok(row(changed, "anthropic/agent-sdk/session-listing").endsWith("| `>=0.3.0 <0.5.0` |"));
  assert.equal(row(changed, "anthropic/agent-sdk/session-message"), row(docs, "anthropic/agent-sdk/session-message"));
  assert.equal(setVersionsInDocs(docs, "anthropic/agent-sdk/session-listing", "<9", "<10"), docs);
});

// ---------- releases ----------

test("versions from npm's dist-tags, a GitHub release, an APT index, a Sparkle feed and an update server's JSON", async () => {
  const fetchImpl = async (url) => ({ ok: true, json: async () => ({ latest: "0.160.1", beta: "0.1.2505172116", "linux-x64": "0.160.1-linux-x64", junk: "not a version", alpha: "0.162.0-alpha.18" }), url });
  const tags = await npmDistTags("@openai/codex", { fetchImpl });
  assert.equal(tags.url, "https://registry.npmjs.org/-/package/@openai%2fcodex/dist-tags");
  assert.equal(tags.versions.latest, "0.160.1");
  assert.equal(tags.versions.junk, undefined);
  assert.equal(releaseVersion({ tag_name: "v2.102.0", name: "GitHub CLI 2.102.0" }), "2.102.0");
  assert.equal(releaseVersion({ tag_name: "v2026.9.24", name: "Hermes Agent v0.21.5 (v2026.9.24)" }, "v(\\d+\\.\\d+\\.\\d+)"), "0.21.5");
  assert.equal(releaseVersion({ tag_name: "nightly-ada" }), null);
  const packages = ["Package: claude-desktop\nVersion: 1.52386.6\n", "Package: claude-desktop\nVersion: 2.110.1\n", "Package: other\nVersion: 9.0.0\n", "Package: claude-desktop\nVersion: 2.26454.0\nFilename: pool/x.deb\n"].join("\n");
  assert.equal(newestInPackages(packages, "claude-desktop"), "2.26454.0");
  assert.equal(newestInPackages(packages, "missing"), null);
  // Muse's feed has one item; an older feed says the version on its enclosure.
  const item = (v) => `<item><title>Version ${v}</title><sparkle:version>1086317620</sparkle:version><sparkle:shortVersionString>${v}</sparkle:shortVersionString><enclosure url="https://example.com/x.dmg" length="1" type="application/octet-stream"/></item>`;
  const feed = (items) => `<?xml version="1.0" encoding="utf-8"?><rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><title>Muse</title>${items}</channel></rss>`;
  assert.equal(newestInAppcast(feed(item("7.0"))), "7.0");
  assert.equal(newestInAppcast(feed(item("6.2") + item("7.1") + item("not a version") + `<item><enclosure sparkle:shortVersionString="7.0.1" url="https://example.com/y.dmg"/></item>`)), "7.1");
  assert.equal(newestInAppcast(feed("")), null);
  // Cursor's update server, for Grok Bot's app.
  assert.equal(versionInJson({ downloadUrl: "https://example.com/Grok_Bot_0.68.1.dmg", version: "0.68.1", commitSha: "abc" }), "0.68.1");
  assert.equal(versionInJson({ release: { version: "1.2.3" } }, "release.version"), "1.2.3");
  assert.equal(versionInJson({ version: "not a version" }), null);
  assert.equal(versionInJson({ version: 7 }), null);
  assert.equal(versionInJson(null), null);
});

// ---------- reading output ----------

test("lines read as XenoNet's tailer reads them: drift by source, with the version the lines carry", () => {
  const r = readers();
  const T = "2026-10-05T12:00:00.000Z";
  const text = [
    JSON.stringify({ type: "user", timestamp: T, version: "2.1.300", message: { role: "user", content: "Check the plan." } }),
    JSON.stringify({ type: "progress-report", timestamp: T, version: "2.1.300" }),
    "not json",
  ].join("\n");
  const { lines, version } = checkLines(r.reader("anthropic/claude-code/transcript-line"), text, CLAUDE_LINES);
  assert.equal(lines.length, 2);
  assert.equal(version, "2.1.300");
  assert.deepEqual(
    r.findings().map((f) => [f.severity, f.kind, f.path, f.vendorVersion]),
    [
      ["addition", "new-variant", "type", "2.1.300"],
      ["change", "unparsable", null, "2.1.300"],
    ],
  );
  assert.deepEqual(r.read(), { "anthropic/claude-code/transcript-line": 2 });
  // A rollout's version is in its first line.
  const c = readers();
  checkLines(c.reader("openai/codex/rollout-line"), [JSON.stringify({ timestamp: T, type: "session_meta", payload: { cli_version: "1.2.0" } }), JSON.stringify({ timestamp: T, type: "token_usage_record", payload: {} })].join("\n"), CODEX_LINES);
  assert.deepEqual(
    c.findings().map((f) => [f.kind, f.vendorVersion]),
    [
      ["new-version", "1.2.0"],
      ["new-variant", "1.2.0"],
    ],
  );
});

test("a turn's transcript and rollout meet the expectations when XenoNet finds what the stand-in put there", () => {
  const T = "2026-10-05T12:00:00.000Z";
  const v = "2.1.300";
  const usage = { input_tokens: 12, output_tokens: 8 };
  const claude = [
    { type: "user", timestamp: T, version: v, message: { role: "user", content: "Check the plan." } },
    { type: "assistant", timestamp: T, version: v, message: { id: "m1", model: "m", content: [{ type: "tool_use", id: "t1", name: "Read", input: {} }], usage, stop_reason: "tool_use" } },
    { type: "user", timestamp: T, version: v, message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "ok" }] } },
    { type: "assistant", timestamp: T, version: v, message: { id: "m2", model: "m", content: [{ type: "text", text: "Done." }], usage, stop_reason: "end_turn" } },
  ];
  assert.deepEqual(transcriptExpectations(claude, v).filter((e) => !e.ok), []);
  const moved = claude.map((l) => (l.type === "assistant" ? { ...l, message: { ...l.message, usage: undefined } } : l));
  assert.deepEqual(
    transcriptExpectations(moved, v).filter((e) => !e.ok).map((e) => [e.severity, e.detail]),
    [["change", "the reply's tokens are read from message.usage"]],
  );
  const codex = [
    { timestamp: T, type: "session_meta", payload: { cli_version: "0.160.1" } },
    { timestamp: T, type: "event_msg", payload: { type: "user_message", message: "Check the plan." } },
    { timestamp: T, type: "response_item", payload: { type: "function_call", name: "update_plan", call_id: "c1", arguments: "{}" } },
    { timestamp: T, type: "response_item", payload: { type: "function_call_output", call_id: "c1", output: "ok" } },
    { timestamp: T, type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { total_tokens: 20 } } } },
    { timestamp: T, type: "event_msg", payload: { type: "task_complete" } },
  ];
  assert.deepEqual(rolloutExpectations(codex, "0.160.1").filter((e) => !e.ok), []);
  assert.equal(rolloutExpectations(codex.slice(0, 4), "0.160.1").filter((e) => !e.ok).length, 2);
});

// ---------- the stand-in API and the made-up home ----------

test("the stand-in API calls a harmless tool once, then answers in text", async () => {
  const tools = [{ name: "Bash" }, { name: "Read" }];
  assert.equal(anthropicReply({ tools }, 1).content[0].type, "text", "Read only when there's a made-up file to read");
  const call = anthropicReply({ tools, model: "claude-x" }, 1, { readFile: "/tmp/box/project/README.md" });
  assert.deepEqual([call.content[0].name, call.content[0].input, call.stop_reason, call.model], ["Read", { file_path: "/tmp/box/project/README.md" }, "tool_use", "claude-x"]);
  assert.equal(anthropicReply({ tools: [{ name: "TodoWrite" }, { name: "Read" }] }, 1, { readFile: "x" }).content[0].name, "TodoWrite");
  assert.equal(anthropicReply({ tools, messages: [{ role: "user", content: [{ type: "tool_result", tool_use_id: "t" }] }] }, 2, { readFile: "x" }).stop_reason, "end_turn");
  assert.equal(openaiReply({ tools: [{ type: "function", name: "exec_command" }, { type: "function", name: "get_goal" }] }, 1).item.name, "get_goal", "never a command");
  assert.equal(openaiReply({ tools: [{ name: "exec_command" }] }, 1).item.type, "message");
  assert.equal(openaiReply({ tools: [{ name: "update_plan" }], input: [{ type: "function_call_output" }] }, 2).item.type, "message");

  const api = await startFakeApi({ readFile: "/x/README.md" });
  try {
    assert.match(api.url, /^http:\/\/127\.0\.0\.1:\d+$/);
    const res = await fetch(`${api.url}/v1/messages`, { method: "POST", body: JSON.stringify({ stream: true, tools: [{ name: "Read" }], messages: [] }) });
    const text = await res.text();
    assert.match(text, /event: message_start/);
    assert.match(text, /"partial_json":"\{\\"file_path\\":\\"\/x\/README.md\\"\}"/);
    const responses = await (await fetch(`${api.url}/v1/responses`, { method: "POST", body: JSON.stringify({ stream: true, tools: [] }) })).text();
    assert.match(responses, /event: response.completed/);
    assert.equal((await fetch(`${api.url}/v1/elsewhere`)).status, 404);
    assert.deepEqual(api.requests.map((q) => [q.method, q.path, q.reply ?? null]), [
      ["POST", "/v1/messages", "tool_use"],
      ["POST", "/v1/responses", "message"],
      ["GET", "/v1/elsewhere", null],
    ]);
  } finally {
    await api.close();
  }
});

test("the made-up home: every home and config folder inside it, and none of this process's keys", () => {
  const saved = { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY, GH_TOKEN: process.env.GH_TOKEN };
  process.env.ANTHROPIC_API_KEY = "sk-ant-real-looking";
  process.env.GH_TOKEN = "gho_real_looking";
  const box = makeSandbox();
  try {
    for (const key of ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "CLAUDE_CONFIG_DIR", "CODEX_HOME"]) assert.ok(box.env[key].startsWith(box.root), `${key} is inside`);
    assert.equal(box.env.ANTHROPIC_API_KEY, undefined);
    assert.equal(box.env.GH_TOKEN, undefined);
    assert.ok(fs.existsSync(box.project) && fs.existsSync(box.codexHome));
    assert.match(codexConfig("http://127.0.0.1:9"), /base_url = "http:\/\/127\.0\.0\.1:9\/v1"/);
  } finally {
    box.remove();
    for (const [k, v] of Object.entries(saved)) if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
  assert.ok(!fs.existsSync(box.root));
});

// ---------- the report ----------

const range = (source) => ({ "anthropic/agent-sdk/session-listing": ">=0.3.0 <0.4.0", "anthropic/agent-sdk/session-message": ">=0.3.0 <0.4.0", "nous/hermes-agent/spawn-ledger": "<1.0.0" })[source] ?? null;
const finding = (over) => ({ source: "anthropic/agent-sdk/session-listing", kind: "new-field", severity: "addition", path: "summaryKind", variant: null, detail: "summaryKind is new (string)", origin: "data", ...over });
const step = (ok) => ({ step: "npm install @anthropic-ai/claude-agent-sdk@0.4.1", ok, detail: ok ? null : "npm install failed (1)" });

function sdkReport({ findings = [], steps = [step(true)], version = "0.4.1" } = {}) {
  const sources = ["anthropic/agent-sdk/session-listing", "anthropic/agent-sdk/session-message"];
  return buildReport({
    at: "2026-10-07T06:17:00.000Z",
    xenonet: { version: "0.6.3", commit: "c5891b2aaaaaaaa" },
    providers: [
      {
        id: "anthropic/agent-sdk",
        name: "Claude Agent SDK",
        sources,
        inspectable: true,
        targets: [{ version, labels: ["latest"], inspected: true, steps, findings: [...rangeFindings(sources, version, range), ...findings] }],
      },
      { id: "anthropic/oauth", name: "Anthropic's plan usage API", sources: ["anthropic/oauth/usage"], inspectable: false, notInspected: "It needs a sign-in.", targets: [] },
    ],
  });
}

test("a version outside a range is a change, found once though a reader finds it too", () => {
  const found = rangeFindings(["anthropic/agent-sdk/session-listing", "nous/hermes-agent/spawn-ledger"], "0.4.1", range);
  assert.deepEqual(found.map((f) => [f.source, f.kind, f.range]), [["anthropic/agent-sdk/session-listing", "new-version", ">=0.3.0 <0.4.0"]]);
  const reader = { ...found[0], origin: "data", range: undefined, count: 3 };
  const one = dedupe([reader, ...found]);
  assert.equal(one.length, 1);
  assert.deepEqual([one[0].origin, one[0].range, one[0].count], ["range", ">=0.3.0 <0.4.0", 3]);
});

test("the report's statuses and counts, and the ranges it allows widening", () => {
  const additions = sdkReport({ findings: [finding()] });
  assert.deepEqual(additions.summary, { providers: 2, changes: 2, additions: 1, notes: 0, errors: 0 });
  assert.deepEqual(additions.providers.map((p) => p.status), ["changes", "not-inspected"]);
  assert.ok(hasDrift(additions));
  const providers = [{ id: "anthropic/agent-sdk", inspect: () => {}, rangeSources: ["anthropic/agent-sdk/session-listing", "anthropic/agent-sdk/session-message"] }];
  assert.deepEqual(fixPlan(additions, providers), [
    { source: "anthropic/agent-sdk/session-listing", provider: "anthropic/agent-sdk", from: ">=0.3.0 <0.4.0", to: ">=0.3.0 <0.5.0", versions: ["0.4.1"] },
    { source: "anthropic/agent-sdk/session-message", provider: "anthropic/agent-sdk", from: ">=0.3.0 <0.4.0", to: ">=0.3.0 <0.5.0", versions: ["0.4.1"] },
  ]);
  // A change to what XenoNet reads in a source holds its range back; a
  // failed step holds back the version's.
  const changed = sdkReport({ findings: [finding({ kind: "type-change", severity: "change", path: "lastModified" })] });
  assert.deepEqual(fixPlan(changed, providers).map((f) => f.source), ["anthropic/agent-sdk/session-message"]);
  assert.deepEqual(fixPlan(sdkReport({ steps: [step(false)] }), providers), []);
  assert.equal(sdkReport({ steps: [step(false)] }).providers[0].status, "error");
  assert.deepEqual(fixPlan(additions, [{ ...providers[0], inspect: null }]), [], "not for a provider the check doesn't run");
  assert.deepEqual(fixPlan(sdkReport({ version: "0.3.9" }), providers), [], "nothing outside a range");
  assert.equal(sdkReport({ version: "0.3.9" }).summary.changes, 0);
});

test("an older version: what the provider added since is a note, what it shares with the latest is counted once, and what XenoNet must still read stays", () => {
  const types = (path, severity = "change") => finding({ source: "openai/codex/app-server", kind: severity === "change" ? "removed" : "new-variant", severity, path, origin: "types", detail: "XenoNet reads it, and the types no longer declare it" });
  const latest = { version: "0.160.1", labels: ["latest"], inspected: true, steps: [], findings: [types("item.type", "addition")] };
  const older = {
    version: "0.146.0",
    labels: ["the VM lab"],
    inspected: true,
    steps: [],
    findings: [
      types("item.type", "addition"),
      types("thread.projectId"),
      finding({ source: "openai/codex/state-db", kind: "missing-field", severity: "change", path: "projects", detail: "missing" }),
      finding({ source: "openai/codex/rollout-line", kind: "type-change", severity: "change", path: "payload.info", detail: "string now" }),
      { source: "openai/codex/rollout-line", kind: "expectation", severity: "change", path: null, variant: null, detail: "token_count events carry info.total_token_usage", origin: "expectation" },
    ],
  };
  const report = buildReport({ at: "2026-10-07T06:17:00.000Z", xenonet: {}, providers: [{ id: "openai/codex", name: "Codex CLI", sources: [], inspectable: true, targets: [latest, older] }] });
  const [, old] = report.providers[0].targets;
  assert.deepEqual(old.findings.map((f) => [f.path ?? f.kind, f.severity, !!f.sameAsLatest]), [
    ["item.type", "addition", true],
    ["thread.projectId", "info", false],
    ["projects", "info", false],
    ["payload.info", "change", false],
    ["expectation", "change", false],
  ]);
  assert.match(old.findings[1].detail, /^this older version's types don't declare it yet/);
  assert.equal(old.findings[2].detail, "only in this older version: missing");
  assert.deepEqual(report.summary, { providers: 1, changes: 2, additions: 1, notes: 2, errors: 0 });
  assert.match(renderMarkdown(report), /1 finding the same as the latest version's\./);
});

test("the report as text: a table, each version's findings, and what to do; nothing a provider sent is markup", () => {
  const report = sdkReport({ findings: [finding({ detail: "<script>alert(1)</script> is new" })] });
  const md = renderMarkdown(report, { runUrl: "https://github.com/willctl/xenonet/actions/runs/1", fixes: fixPlan(report, PROVIDERS) });
  assert.match(md, /^## Provider drift\n/);
  assert.match(md, /Checked 2026-10-07 06:17:00 UTC against XenoNet 0\.6\.3 at `c5891b2` \(\[run\]\(https:\/\/github\.com\/willctl\/xenonet\/actions\/runs\/1\)\): 2 changes to what XenoNet reads, 1 addition, 0 checks that couldn't run\./);
  assert.match(md, /\| Claude Agent SDK \| `0\.4\.1` \(latest\) \| changes \(2 changes, 1 addition\) \|/);
  assert.match(md, /Not inspected: It needs a sign-in\./);
  assert.match(md, /- `anthropic\/agent-sdk\/session-listing`: version 0\.4\.1 is outside the versions the schema was made for \(&gt;=0\.3\.0 &lt;0\.4\.0\) \(its version\)/);
  assert.ok(!md.includes("<script>"));
  assert.match(md, /&lt;script&gt;/);
  assert.match(md, /\*\*Prepared\*\*: `anthropic\/agent-sdk\/session-listing` `>=0\.3\.0 <0\.4\.0` → `>=0\.3\.0 <0\.5\.0`/);
  assert.match(renderMarkdown(sdkReport({ version: "0.3.9" })), /Nothing: every inspected version reads the way XenoNet expects\./);
  // A failed step's words are the provider's: in code, a line long.
  const failed = renderMarkdown(sdkReport({ version: "0.3.9", steps: [{ step: "run claude", ok: false, detail: "exited 1:\nsee @someone's [notes](https://example.com)" }] }));
  assert.match(failed, /Couldn't run:\n\n- run claude: `exited 1: see @someone's \[notes\]\(https:\/\/example\.com\)`\n/);
  assert.match(failed, /\*\*Couldn't run\*\* means the check itself failed/);
});

test("the tracking issue: opened for drift, quietly refreshed for the same drift, commented on when it changes, closed when it's gone", () => {
  const report = sdkReport({ findings: [finding()] });
  const create = issueAction(report, null, { runUrl: "https://example.com/run" });
  assert.equal(create.action, "create");
  assert.equal(create.title, "Provider drift: 2 changes, 1 addition");
  assert.match(create.body, /<!-- xenonet-provider-drift fingerprint:[0-9a-f]{16} -->\n$/);
  // The same drift, seen again later with other counts: no comment.
  const later = { ...report, at: "2026-10-08T06:17:00.000Z", providers: report.providers.map((p) => ({ ...p, targets: p.targets.map((t) => ({ ...t, findings: t.findings.map((f) => ({ ...f, count: 9 })).reverse() })) })) };
  assert.equal(fingerprint(later), fingerprint(report));
  const refresh = issueAction(later, { number: 7, body: create.body });
  assert.deepEqual([refresh.action, refresh.comment], ["refresh", undefined]);
  const update = issueAction(sdkReport({ findings: [finding(), finding({ path: "otherField" })] }), { number: 7, body: create.body }, { runUrl: "https://example.com/run" });
  assert.equal(update.action, "update");
  assert.match(update.comment, /^The report changed on 2026-10-07 \(\[run\]\(https:\/\/example\.com\/run\)\): 2 changes, 2 additions, 0 failed checks\./);
  const clean = sdkReport({ version: "0.3.9" });
  assert.equal(issueAction(clean, { number: 7, body: create.body }).action, "close");
  assert.deepEqual(issueAction(clean, null), { action: "none" });
  // A failed check is worth an issue too.
  assert.equal(issueAction(sdkReport({ version: "0.3.9", steps: [step(false)] }), null).action, "create");
  assert.ok(issueBody(report).length < 65_536);
});
