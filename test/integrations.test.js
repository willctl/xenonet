import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRecords, createTickets, queryRecords, clean } from "../core/integrations/common.js";
import { createRouting, routingInventory } from "../core/integrations/routing.js";
import { createOpenShell, openShellCommand, runCommand, executionProfiles } from "../core/integrations/openshell.js";
import { piSessionMetadata, piExportMetadata, scanPi } from "../core/integrations/pi.js";
import { createIntegrations } from "../core/integrations/index.js";
import { createDecider, KEY_TARGET } from "../core/decide.js";
import { createMemorySecrets } from "../core/secret-store.js";

const temp = (t) => { const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-integrations-")); t.after(() => fs.rmSync(dir, { recursive: true, force: true })); return dir; };
const nodes = [{ kind: "session", app: "codex" }, { kind: "session", app: "claude" }];
const good = { tool: { type: "choice", choice: "codex", confidence: 0.9, probabilities: { codex: 0.8, claude: 0.1, review: 0.1 } } };
function router({ answers = good, inventory = () => routingInventory(nodes), decide, now, records = createRecords(null) } = {}) {
  const requests = [];
  const decider = { model: "test/jev", decide: decide ?? (async (request) => { requests.push(request); return { answers, model: "test/jev", cost: 0.00004, ms: 8 }; }) };
  return { routing: createRouting({ decider, inventory, records, now }), records, requests };
}

test("preview tickets expire, are single-use, and cannot grow without a bound", () => {
  let clock = 0; const tickets = createTickets({ now: () => clock });
  const first = tickets.issue("private"); assert.equal(tickets.take(first.ticket), "private");
  assert.throws(() => tickets.take(first.ticket), /expired/);
  const expired = tickets.issue("old"); clock = expired.expires;
  assert.throws(() => tickets.take(expired.ticket), /expired/);
  for (let i = 0; i < 32; i++) tickets.issue(i);
  assert.throws(() => tickets.issue(33), /old preview/);
  clock += 600001; assert.ok(tickets.issue("new").ticket);
});

test("metadata records survive restart, prune retention and query before paging", (t) => {
  let clock = 0; const dir = temp(t), records = createRecords(dir, { limit: 60, now: () => ++clock });
  for (let i = 0; i < 65; i++) records.add({ kind: "decision", status: i === 64 ? "rare" : "normal", model: `m${i}` });
  const restored = createRecords(dir).list(); assert.equal(restored.length, 60);
  const newest = queryRecords(restored, { status: "rare" }); assert.equal(newest.matched, 1);
  const sorted = queryRecords(restored, { sort: "oldest", page: 2 }); assert.equal(sorted.records.length, 10);
  assert.equal(queryRecords(restored, { q: "not present" }).pages, 0);
  assert.equal(queryRecords(restored, { kind: "sandbox" }).total, 0);
  const id = restored[0].id; records.update(id, { outcome: "useful" }); assert.equal(createRecords(dir).list().find((r) => r.id === id).outcome, "useful");
  fs.writeFileSync(path.join(dir, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa.json"), "broken");
  assert.equal(createRecords(dir).list().length, 60);
  assert.throws(() => records.update("../bad", {}), /No such record/);
});

test("routing preview sends nothing; JevRouter records probabilities and never task context", async (t) => {
  const dir = temp(t), { routing, records, requests } = router({ records: createRecords(dir) });
  const preview = await routing.preview({ request: "PRIVATE_TASK coding review" });
  assert.equal(requests.length, 0); assert.deepEqual(preview.outgoing.provider, { zdr: true });
  const result = await routing.route(preview.ticket);
  assert.equal(result.selected, "codex"); assert.equal(result.status, "needs_confirmation");
  assert.equal(requests[0].state.request, "PRIVATE_TASK coding review");
  assert.equal(result.candidates.find((r) => r.id === "codex").probability, 0.8);
  assert.equal(result.taskHash.length, 64); assert.equal(result.cost, 0.00004);
  assert.equal(JSON.stringify(records.list()).includes("PRIVATE_TASK"), false);
  assert.equal(fs.readFileSync(path.join(dir, `${result.id}.json`), "utf8").includes("PRIVATE_TASK"), false);
  assert.equal(routing.outcome(result.id, "useful").outcome, "useful");
  assert.throws(() => routing.outcome(result.id, "permission-granted"), /valid decision/);
  await assert.rejects(routing.route(preview.ticket), /already used/);
});

test("routing uses the existing credential store and actual zero-retention provider payload", async () => {
  const secrets = createMemorySecrets(); const key = "sk-or-v1-CANARY0123456789abcdefABCDEF"; await secrets.set(KEY_TARGET, key);
  let sent;
  const decider = createDecider({ secrets, env: {}, fetch: async (_, init) => { sent = init; return { ok: true, json: async () => ({ answers: good, model: "test", usage: { cost: 0.1 } }) }; } });
  const routing = createRouting({ decider, records: createRecords(null), inventory: () => routingInventory(nodes) });
  await routing.route((await routing.preview({ request: "Inspect repository" })).ticket);
  const body = JSON.parse(sent.body); assert.deepEqual(body.provider, { zdr: true });
  assert.equal(sent.headers.authorization, `Bearer ${key}`); assert.equal(sent.body.includes(key), false);
  assert.deepEqual(Object.keys(body.state), ["request"]);
});

test("low confidence and unavailable agents remain suggestions with explicit fallback", async () => {
  const low = router({ answers: { tool: { ...good.tool, confidence: 0.5 } } });
  const value = await low.routing.route((await low.routing.preview({ request: "Inspect repository" })).ticket);
  assert.equal(value.status, "no_decision"); assert.equal(value.selected, null); assert.equal(value.reason, "low_confidence");
  const unavailable = router({ inventory: () => routingInventory([]) });
  const filtered = await unavailable.routing.route((await unavailable.routing.preview({ request: "Inspect repository" })).ticket);
  assert.equal(filtered.selected, "review"); assert.equal(filtered.reason, "manual_review");
  assert.equal(filtered.candidates.find((r) => r.id === "codex").filtered, true);
});

for (const answers of [{}, { tool: { ...good.tool, choice: "injected" } }, { tool: { ...good.tool, confidence: NaN } }, { tool: { ...good.tool, probabilities: { codex: 9, claude: 0, review: 0 } } }, { tool: { ...good.tool, probabilities: { codex: 0.3 } } }]) {
  test(`routing refuses malformed provider evidence ${JSON.stringify(answers)}`, async () => {
    const { routing } = router({ answers }); const result = await routing.route((await routing.preview({ request: "Inspect" })).ticket);
    assert.equal(result.status, "no_decision"); assert.equal(result.selected, null); assert.equal(result.reason, "provider_error");
  });
}

test("provider outage and missing credentials never execute or persist error contents", async () => {
  const { routing } = router({ decide: async () => { throw new Error("API_KEY=PRIVATE_VALUE"); } });
  const result = await routing.route((await routing.preview({ request: "Inspect" })).ticket);
  assert.equal(result.reason, "provider_error"); assert.equal(JSON.stringify(result).includes("PRIVATE_VALUE"), false);
  await assert.rejects(routing.preview({ request: "token=PRIVATE_VALUE" }), /credentials/);
  await assert.rejects(routing.preview({ request: "x".repeat(2001) }), /2000/);
});

test("routing detects inventory changes and serializes paid requests", async () => {
  let list = nodes; const r = router({ inventory: () => routingInventory(list) });
  const preview = await r.routing.preview({ request: "Inspect" }); list = [];
  await assert.rejects(r.routing.route(preview.ticket), /agents changed/); assert.equal(r.requests.length, 0);
  let finish; const slow = router({ decide: () => new Promise((resolve) => { finish = resolve; }) });
  const a = await slow.routing.preview({ request: "First" }), b = await slow.routing.preview({ request: "Second" });
  const running = slow.routing.route(a.ticket); await assert.rejects(slow.routing.route(b.ticket), /already running/);
  finish({ answers: good }); await running;
});

test("OpenShell profiles use argv and reject Windows native execution and invalid WSL names", () => {
  assert.deepEqual(openShellCommand({ backend: "native" }, ["--version"], "linux"), { file: "openshell", args: ["--version"] });
  const wsl = openShellCommand({ backend: "wsl", distro: "Ubuntu-24.04" }, ["--version"], "win32");
  assert.deepEqual(wsl.args, ["--distribution", "Ubuntu-24.04", "--exec", "openshell", "--version"]);
  assert.throws(() => openShellCommand({ backend: "wsl", distro: "Ubuntu;whoami" }, [], "win32"), /distribution/);
  assert.throws(() => openShellCommand({ backend: "native" }, [], "win32"), /unavailable/);
  assert.throws(() => openShellCommand({ backend: "wsl", distro: "Ubuntu" }, [], "linux"), /Windows only/);
});

test("OpenShell reviewed upload, policy hash, provider and command stay bound to a single job", async (t) => {
  const dir = temp(t), policy = path.join(dir, "policy.yaml"); fs.writeFileSync(policy, "network_policies: {}\n");
  const records = createRecords(null), calls = [];
  const shell = createOpenShell({ records, platform: "linux", run: async (file, args, opts) => { calls.push({ file, args, opts }); return { stdout: "{}" }; } });
  const body = { backend: "native", agent: "codex", image: "agent:1.0", provider: "openrouter", source: dir, upload: true, policy, request: "task with $(literal); data" };
  const preview = await shell.preview(body); assert.equal(calls.length, 0);
  assert.equal(preview.command.args.at(-1), body.request); assert.ok(preview.command.args.includes(`${dir}:/workspace`));
  const result = await shell.launch(preview.ticket); assert.equal(result.status, "created"); assert.equal(calls[0].opts.timeout, 120000);
  assert.equal(JSON.stringify(records.list()).includes(body.request), false);
  assert.equal(result.policyHash.length, 64); assert.ok(result.name.startsWith("xeno-"));
  await assert.rejects(shell.launch(preview.ticket), /already used/);
  const changed = await shell.preview(body); fs.writeFileSync(policy, "changed");
  await assert.rejects(shell.launch(changed.ticket), /policy changed/); assert.equal(calls.length, 1);
  await assert.rejects(shell.preview({ ...body, upload: false }), /Confirm/);
  await assert.rejects(shell.preview({ ...body, image: "--unsafe" }), /image reference/);
  await assert.rejects(shell.preview({ ...body, agent: "arbitrary-command" }), /Choose Pi/);
  await assert.rejects(shell.preview({ ...body, provider: "a;rm" }), /provider name/);
  await assert.rejects(shell.preview({ ...body, request: "api_key=PRIVATE" }), /credentials/);
});

test("OpenShell WSL upload path translation is literal and bounded", async (t) => {
  const dir = temp(t), calls = [];
  const shell = createOpenShell({ records: createRecords(null), platform: "win32", run: async (file, args) => { calls.push({ file, args }); return { stdout: "/mnt/c/Projects/example\n" }; } });
  const preview = await shell.preview({ backend: "wsl", distro: "Ubuntu", agent: "pi", image: "pi:1", source: dir, upload: true, request: "Review" });
  assert.equal(calls[0].args.at(-3), "wslpath"); assert.equal(calls[0].args.at(-1), fs.realpathSync(dir));
  assert.ok(preview.command.args.includes("/mnt/c/Projects/example:/workspace"));
});

test("OpenShell failed creation is reconcilable and policy denial is separate from refusals", async () => {
  const records = createRecords(null);
  const shell = createOpenShell({ records, platform: "linux", run: async () => { throw Object.assign(new Error("timeout"), { stderr: "policy_denied TOKEN=PRIVATE" }); } });
  const result = await shell.launch((await shell.preview({ agent: "pi", image: "pi:1", request: "Review" })).ticket);
  assert.equal(result.status, "creation-unknown"); assert.equal(result.eventKind, "execution-policy-denial");
  assert.equal(JSON.stringify(records.list()).includes("PRIVATE"), false);
  assert.equal((await shell.status({ backend: "native" })).ready, false);
});

test("OpenShell status, inspection and deletion target only recorded jobs", async () => {
  const records = createRecords(null), calls = [];
  const shell = createOpenShell({ records, platform: "linux", run: async (_, args) => { calls.push(args); return { stdout: args.includes("list") ? '{"sandboxes":[]}' : args.includes("--output") ? '{"phase":"Ready"}' : args.includes("logs") ? "policy_denied api_key=PRIVATE" : "test" }; } });
  assert.equal((await shell.status({ backend: "native" })).ready, true);
  const value = await shell.launch((await shell.preview({ agent: "claude", image: "claude:1", request: "Review" })).ticket);
  const inspection = await shell.inspect(value.id); assert.equal(inspection.sandbox.phase, "Ready"); assert.ok(inspection.logs.includes("[redacted]"));
  assert.equal(JSON.stringify(records.list()).includes("logs"), false);
  assert.equal((await shell.remove(value.id)).status, "deleted");
  await shell.remove(value.id); assert.equal(calls.filter((a) => a.includes("delete")).length, 1);
  await assert.rejects(shell.inspect(value.id), /deleted/); await assert.rejects(shell.remove("../foreign"), /No such/);
});

test("the subprocess adapter passes shell-like text literally and enforces timeout", async () => {
  const result = await runCommand(process.execPath, ["-e", "process.stdout.write(process.argv[1])", "$(literal);&data"]);
  assert.equal(result.stdout, "$(literal);&data");
  await assert.rejects(runCommand(process.execPath, ["-e", "setTimeout(()=>{}, 10000)"], { timeout: 50 }));
});

const jsonl = (entries) => entries.map((e) => JSON.stringify(e)).join("\n");
const header = { type: "session", version: 3, id: "pi-1", timestamp: "2026-10-13T12:00:00Z", cwd: "/projects/demo" };
test("Pi metadata drops all prompt, argument and result text and tracks successful calls", () => {
  const content = jsonl([header, { type: "message", id: "a", parentId: null, message: { role: "user", content: "PRIVATE_PROMPT" } }, { type: "message", id: "b", parentId: "a", message: { role: "assistant", model: "demo", usage: { totalTokens: 100, cost: { total: 0.01 } }, content: [{ type: "toolCall", id: "tool-1", arguments: { secret: "PRIVATE_ARGUMENT" } }] } }, { type: "message", id: "c", parentId: "b", message: { role: "toolResult", toolCallId: "tool-1", isError: false, content: "PRIVATE_RESULT" } }]);
  const result = piSessionMetadata(content, "/a.jsonl");
  assert.equal(result.status, "recorded"); assert.equal(result.pending, 0); assert.equal(result.tools, 1); assert.equal(result.failures, 0); assert.equal(result.tokens, 100);
  assert.equal(JSON.stringify(result).includes("PRIVATE"), false);
});

test("Pi latest branch ignores old failed attempts, tolerates partial tails, and leaves pending ambiguous", () => {
  const content = jsonl([header, { type: "message", id: "old", parentId: null, message: { role: "toolResult", isError: true } }, { type: "message", id: "new", parentId: null, message: { role: "assistant", content: [{ type: "toolCall", id: "wait" }] } }]) + '\n{"partial":';
  const result = piSessionMetadata(content, "/a.jsonl");
  assert.equal(result.pending, 1); assert.equal(result.failures, 0); assert.equal(result.skipped, 1); assert.equal(result.status, "awaiting-result");
  assert.match(result.note, /does not establish/);
  assert.throws(() => piSessionMetadata(jsonl([{ ...header, version: 99 }]), "/a"), /newer/);
  assert.throws(() => piSessionMetadata("{}", "/a"), /Not a Pi/);
});

test("Pi scanner reports missing, malformed and oversized sources without changing them", async (t) => {
  const dir = temp(t); fs.mkdirSync(path.join(dir, "project"));
  const file = path.join(dir, "project", "session.jsonl"); const original = jsonl([header]); fs.writeFileSync(file, original);
  fs.writeFileSync(path.join(dir, "broken.jsonl"), "bad"); fs.writeFileSync(path.join(dir, "large.jsonl"), "x".repeat(1025));
  const scanned = await scanPi(dir, { maxBytes: 1024 }); assert.equal(scanned.records.length, 1); assert.equal(scanned.warnings.length, 2);
  assert.equal(fs.readFileSync(file, "utf8"), original);
  assert.equal((await scanPi(path.join(dir, "missing"))).records.length, 0);
  assert.ok((await scanPi(dir, { maxFiles: 1 })).warnings.length);
});

test("Pi run import is a bounded metadata projection, deduplicated and persisted; scanning is opt-in", async (t) => {
  const dir = temp(t), home = path.join(dir, "home"); fs.mkdirSync(path.join(home, ".pi", "agent", "sessions"), { recursive: true });
  fs.writeFileSync(path.join(home, ".pi", "agent", "sessions", "test.jsonl"), jsonl([header]));
  const opts = { dir: path.join(dir, "state"), home, platform: "linux", decider: { status: async () => ({ key: "missing" }) } };
  const service = createIntegrations(opts); assert.equal((await service.pi({})).total, 0);
  service.enablePi(true); assert.equal((await service.pi({})).total, 1);
  assert.equal((await createIntegrations(opts).status()).piOn, true);
  const value = { run: { id: "run-1", status: "completed", model: "demo", prompt: "PRIVATE", usage: { totalTokens: 123, estimatedCost: 0.2 } }, artifacts: [{ content: "PRIVATE" }], diagnostics: [{ message: "PRIVATE" }], trace: { context: "PRIVATE" } };
  assert.equal(JSON.stringify(piExportMetadata(value)).includes("PRIVATE"), false);
  const imported = service.importPi(value); assert.equal(imported.tokens, 123); service.importPi(value); assert.equal((await service.pi({})).total, 2);
  assert.equal(JSON.stringify(await service.pi({})).includes("PRIVATE"), false);
  service.enablePi(false); assert.equal((await service.pi({})).total, 1);
  assert.throws(() => service.importPi({ run: {} }), /JSON run export/); assert.throws(() => service.enablePi("true"), /whether/);
  assert.equal(clean("Bearer PRIVATE token=PRIVATE"), "[redacted] [redacted]");
});

test("execution profile discovery handles Windows UTF-16 output and never selects Docker's internal distribution", async () => {
  const raw = [..."docker-desktop\r\nUbuntu-24.04\r\n"].join("\0") + "\0";
  assert.deepEqual(await executionProfiles({ platform: "win32", run: async () => ({ stdout: raw }) }), [{ backend: "wsl", distro: "Ubuntu-24.04" }]);
  assert.deepEqual(await executionProfiles({ platform: "win32", run: async () => { throw new Error("missing"); } }), []);
  assert.deepEqual(await executionProfiles({ platform: "linux" }), [{ backend: "native", distro: "" }]);
});

test("record retention preserves all remote jobs and limits completed metadata", () => {
  let clock = 0; const records = createRecords(null, { limit: 2, now: () => ++clock });
  const active = records.add({ kind: "sandbox", status: "created" });
  for (let i = 0; i < 5; i++) records.add({ kind: "decision", status: "selected" });
  assert.equal(records.list().length, 3); assert.ok(records.list().some((r) => r.id === active.id));
});

test("OpenShell retains a job until gateway listing confirms deletion, and bounds active launches", async () => {
  const records = createRecords(null);
  const shell = createOpenShell({ records, platform: "linux", run: async () => ({ stdout: '{"sandboxes":[{"name":"still-cleaning"}]}' }) });
  const record = await shell.launch((await shell.preview({ agent: "pi", image: "pi:1", request: "Review" })).ticket);
  assert.equal((await shell.remove(record.id)).status, "deletion-requested");
  for (let i = 0; i < 63; i++) records.add({ kind: "sandbox", status: "created" });
  await assert.rejects(shell.launch((await shell.preview({ agent: "pi", image: "pi:1", request: "Review" })).ticket), /64 maximum/);
});

test("Pi scanner skips symlink files and folders", async (t) => {
  const dir = temp(t), outside = temp(t); fs.writeFileSync(path.join(outside, "private.jsonl"), jsonl([header]));
  try { fs.symlinkSync(outside, path.join(dir, "linked"), process.platform === "win32" ? "junction" : "dir"); }
  catch (e) { if (e.code === "EPERM") { t.skip("Symlink creation is unavailable."); return; } throw e; }
  assert.equal((await scanPi(dir)).records.length, 0);
});
