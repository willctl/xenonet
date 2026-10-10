// Decisions handed to a decision model (core/decide.js, core/classify.js):
// the key only in the credential store and only in the request's header,
// zero data retention asked, no ids of yours sent, a log line with the
// model, time and cost and never what was asked; and the projects folder's
// picks as one choice a folder, keeping only what the model is sure of.
// OpenRouter and the key are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createDecider, KEY_TARGET, DEFAULT_MODEL } from "../core/decide.js";
import { createMemorySecrets } from "../core/secret-store.js";
import { jevQuestions, picksFromAnswers, askDeciderForPicks, CONFIDENT } from "../core/classify.js";

const KEY = "sk-or-v1-CANARY0123456789abcdefABCDEF";

function fakeOpenRouter(answer = () => ({})) {
  const calls = [];
  const fetch = async (url, init) => {
    const body = JSON.parse(init.body);
    calls.push({ url, headers: init.headers, body });
    return { ok: true, status: 200, json: async () => ({ id: "dec_1", model: "typesafe/jev-1.13-20260917", provider: "TypeSafe", answers: answer(body), usage: { input_tokens: 900, output_tokens: 0, cost: 0.00004 } }) };
  };
  return { calls, fetch };
}

test("the key goes to the credential store and only into the request's header; zero data retention asked; the log says how long and how much, never what", async () => {
  const secrets = createMemorySecrets();
  const lines = [];
  const log = { info: (code, msg) => lines.push([code, msg]), warn: (code, msg) => lines.push([code, msg]) };
  const or = fakeOpenRouter((b) => Object.fromEntries(Object.keys(b.questions).map((k) => [k, { type: "choice", choice: "project", confidence: 0.9, probabilities: {} }])));
  const d = createDecider({ secrets, fetch: or.fetch, log, env: {} });
  assert.deepEqual(await d.status(), { model: DEFAULT_MODEL, key: "missing", store: secrets.kind });
  await assert.rejects(d.decide({ kind: "x", state: {}, questions: {} }), (e) => e.code === "no-key");
  await assert.rejects(d.storeKey("sk-ant-not-openrouter-0000000000"), /isn't an OpenRouter key/);
  assert.deepEqual(await d.storeKey(` ${KEY} `), { key: "stored", store: secrets.kind });
  assert.equal(await secrets.get(KEY_TARGET), KEY);
  const r = await d.decide({ kind: "projects", state: { folders: ["secret-plans"] }, questions: { f0: { type: "choice", instructions: "?", criteria: { a: "a", b: "b" } } } });
  assert.deepEqual([r.model, r.cost, r.answers.f0.choice], ["typesafe/jev-1.13-20260917", 0.00004, "project"]);
  const call = or.calls[0];
  assert.equal(call.url, "https://openrouter.ai/api/alpha/decisions");
  assert.equal(call.headers.authorization, `Bearer ${KEY}`);
  assert.deepEqual(call.body.provider, { zdr: true });
  assert.ok(!("user" in call.body) && !("session_id" in call.body) && !("trace" in call.body), "no ids of yours");
  assert.ok(!JSON.stringify(call.body).includes(KEY), "never in the body");
  assert.deepEqual(lines.map(([code]) => code), ["decide.projects"]);
  assert.ok(!lines.some(([, msg]) => /secret-plans/.test(msg) || msg.includes(KEY)), "the log has neither what was asked nor the key");
  await d.forgetKey();
  assert.equal((await d.status()).key, "missing");
});

test("an OpenRouter that refuses or can't be reached is said plainly, a bad key as such", async () => {
  const secrets = createMemorySecrets();
  await secrets.set(KEY_TARGET, KEY);
  const d401 = createDecider({ secrets, env: {}, fetch: async () => ({ ok: false, status: 401, json: async () => ({ error: { message: "No auth credentials found" } }) }) });
  await assert.rejects(d401.decide({ kind: "x", state: {}, questions: {} }), (e) => e.code === "bad-key" && /No auth credentials/.test(e.message));
  const down = createDecider({ secrets, env: {}, fetch: async () => { throw new Error("ECONNREFUSED"); } });
  await assert.rejects(down.decide({ kind: "x", state: {}, questions: {} }), (e) => e.code === "unreachable");
});

test("the projects folder as one choice a folder: repos left out, picks kept only when sure and when they change something", async () => {
  const outline = [
    { path: "atlas", xenonet: "repo", markers: [], sessions: 3 },
    { path: "examples", xenonet: "project", markers: ["package.json"], sessions: 0 },
    { path: "tools", xenonet: "project", markers: ["README.md"], sessions: 2 },
    { path: "scratch", xenonet: "folder", markers: [], sessions: 0 },
    ...Array.from({ length: 45 }, (_, i) => ({ path: `lab/${i}`, xenonet: "folder", markers: [], sessions: 0 })),
  ];
  const batches = jevQuestions(outline);
  assert.equal(batches.length, 2, "a big folder goes in more than one request");
  assert.equal(batches[0].folders[0].path, "examples", "repos aren't asked about");
  const q = batches[0].questions.f0;
  assert.deepEqual([q.type, Object.keys(q.criteria)], ["choice", ["project", "group", "folder"]]);
  assert.match(q.instructions, /"examples".*"project"/);
  const answers = {
    f0: { choice: "folder", confidence: 0.95 }, // folder isn't a pick XenoNet keeps
    f1: { choice: "group", confidence: CONFIDENT + 0.1 },
    f2: { choice: "project", confidence: CONFIDENT - 0.1 }, // not sure enough
  };
  assert.deepEqual(picksFromAnswers(answers, batches[0].folders), [{ path: "tools", as: "group", reason: "the decision model was 85% sure" }]);
  // End to end through a made-up OpenRouter.
  const secrets = createMemorySecrets();
  await secrets.set(KEY_TARGET, KEY);
  const or = fakeOpenRouter((b) => Object.fromEntries(Object.keys(b.questions).map((k) => [k, { choice: k === "f44" ? "project" : "folder", confidence: 0.99 }])));
  const picks = await askDeciderForPicks(createDecider({ secrets, fetch: or.fetch, env: {} }), outline);
  assert.equal(or.calls.length, 2);
  assert.deepEqual(picks, [{ path: "lab/41", as: "project", reason: "the decision model was 99% sure" }], "keys line up across requests");
  assert.ok(or.calls.every((c) => c.body.state.folders.length === outline.length), "each request knows the whole folder");
});
