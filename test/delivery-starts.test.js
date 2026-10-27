// Starting a session (core/delivery/starts.js) and who may (reach.js's
// start): a request's app, mode, folder and prompt, its first prompt framed
// with who asked, each app's command with the prompt never an argument, the
// limits, the folders a machine takes, what a run's output says of its
// session and its sign-in, and the answers in words. npm run test:rust runs
// it against the Rust twins too.
import { test } from "node:test";
import assert from "node:assert/strict";
import { decide, senderOf, ACTIONS } from "../core/delivery/reach.js";
import { request, framed, command, admit, asker, kept, folder, signedOut, sessionOf, answerWords, signInWords, signInToken, SIGN_IN_TARGET, APPS, PROMPT_MAX, TARGET } from "../core/delivery/starts.js";

const ID = "0190d3e0-0000-7000-8000-00000000ab01";

test("a start comes from a window host: you strongly, sessions and actors under start: network", () => {
  assert.deepEqual(ACTIONS, ["message", "wake", "start"]);
  assert.equal(TARGET, "xenonet:start");
  const from = (id, window, strong) => ({ ...senderOf(id), window, strong });
  const d = (s, start) => decide("start", s, "", { start });
  const you = from("you", true, true);
  assert.deepEqual(d(you, "you"), { ok: true });
  assert.deepEqual(d(you, "network"), { ok: true });
  assert.deepEqual(d(from("you", true, false), "network"), { ok: false, why: "a start as you comes only from a window host's console, checked by its window key" });
  for (const s of [from("@laptop:claude:aaaaaaaa-0001", true, false), from("actor:muse", true, true)]) {
    assert.deepEqual(d(s, "you"), { ok: false, why: "only you start sessions here: its start setting is you" });
    assert.deepEqual(d(s, "network"), { ok: true });
  }
  assert.deepEqual(d({ ...you, window: false, host: "desk" }, "network"), { ok: false, why: "desk isn't a window host, and only window hosts start sessions" });
  assert.deepEqual(d(you, "off"), { ok: false, why: "this machine starts no sessions: its start setting is off" });
  assert.deepEqual(d(you, "nonsense"), { ok: true }, "anything else is you");
  // senderOf names the machine a window host, weakly checked.
  assert.deepEqual(senderOf("@studio:actor:muse"), { kind: "actor", id: "@studio:actor:muse", host: "studio", window: true, strong: false });
});

test("wake: network wakes without your allowance", () => {
  const s = senderOf("@laptop:actor:muse");
  assert.deepEqual(decide("wake", s, "claude:x", { consent: {} }), { ok: false, why: "no consent" });
  assert.deepEqual(decide("wake", s, "claude:x", { consent: {}, wake: "network" }), { ok: true });
  assert.deepEqual(decide("wake", s, "claude:x", { consent: {}, wake: "consent" }), { ok: false, why: "no consent" });
});

test("a request says its app, mode, folder and prompt", () => {
  const r = request({ prompt: "  Fix the login form  " });
  assert.deepEqual(r, { ok: true, request: { folder: "~", app: "claude", mode: null, prompt: "Fix the login form" } });
  assert.equal(request({ app: "grok", mode: "plan", folder: "project:atlas", prompt: "x" }).request.mode, "plan");
  assert.equal(request({ mode: "manual", prompt: "x" }).request.mode, null);
  assert.deepEqual(request({ mode: "bypassPermissions", prompt: "x" }), { ok: false, why: "XenoNet never starts a session that bypasses its permission checks" });
  assert.equal(request({ mode: "yolo", prompt: "x" }).ok, false);
  assert.equal(request({ app: "codex", mode: "plan", prompt: "x" }).ok, false);
  assert.equal(request({ app: "opencode", prompt: "x" }).ok, false);
  assert.deepEqual(request({ prompt: " " }), { ok: false, why: "say what the new session should do: a prompt" });
  assert.equal(request({ prompt: "x".repeat(PROMPT_MAX + 1) }).ok, false);
  assert.deepEqual(APPS, ["claude", "codex", "grok"]);
});

test("the first prompt says who asked, with the title made safe", () => {
  assert.equal(framed({ kind: "you", id: "actor:you" }, "inix", "Go."), "[XenoNet] Started by you, at inix's console.\n\nGo.");
  assert.equal(framed({ kind: "you" }, null, "Go."), "[XenoNet] Started by you, at this machine's console.\n\nGo.");
  assert.equal(
    framed({ kind: "session", id: "claude:aaaaaaaa-0001", title: "Fix «it»\n\u001b[31mnow" }, "inix", "Go."),
    "[XenoNet] inix says this was asked by the session «Fix \"it\" now» (claude:aaaaaaaa-0001). It came from an agent, not from your user.\n\nGo.",
  );
  assert.match(framed({ kind: "actor", id: "actor:muse" }, null, "Go."), /^\[XenoNet\] This was asked by the actor «actor:muse» \(actor:muse\) on this machine\./);
});

test("each app starts its own way, and the prompt is never an argument", () => {
  const r = (app, mode = null) => ({ folder: "~", app, mode, prompt: "-rm -rf" });
  assert.deepEqual(command(r("claude", "plan"), ID, "/w/atlas", { settings: "/s/listen.json", server: "xenonet" }), [
    "claude",
    ["-p", "--session-id", ID, "--settings", "/s/listen.json", "--permission-mode", "plan", "--permission-prompt-tool", "mcp__xenonet__permission"],
  ]);
  assert.deepEqual(command(r("claude"), ID, "/w", { server: "xenonet-dev" })[1], ["-p", "--session-id", ID, "--permission-prompt-tool", "mcp__xenonet-dev__permission"]);
  assert.deepEqual(command(r("codex"), ID, "/w/atlas"), ["codex", ["exec", "--json", "-C", "/w/atlas", "-"]]);
  assert.deepEqual(command(r("grok", "manual"), ID, "/w", { promptFile: "/s/runs/p.txt" })[1], ["--prompt-file", "/s/runs/p.txt", "-s", ID, "--output-format", "json", "--permission-mode", "default"]);
  for (const app of APPS) assert.ok(!command(r(app), ID, "/w", { promptFile: "/p" })[1].some((a) => a.includes("rm -rf")), app);
});

test("twenty starts an hour on a machine, and ten from one machine", () => {
  const now = 10_000_000;
  const h = Array.from({ length: 10 }, (_, i) => [now - i * 1000, "inix"]);
  assert.equal(admit(h, "inix", now), "inix asks for at most 10 sessions an hour here");
  assert.equal(admit(h, "", now), null);
  h.push(...Array.from({ length: 10 }, () => [now - 5, ""]));
  assert.equal(admit(h, "studio", now), "this machine starts at most 20 sessions an hour");
  assert.equal(admit(h, "studio", now + 3600e3), null);
  assert.equal(kept(h, now + 3600e3 - 6000).length, 16);
  // Each asker here has its own ten.
  const mine = Array.from({ length: 10 }, () => [now, asker("claude:a")]);
  assert.equal(admit(mine, asker("claude:a"), now), "claude:a asks for at most 10 sessions an hour here");
  assert.equal(admit(mine, asker("claude:b"), now), null);
  assert.equal(admit(mine, asker("you"), now), null);
  assert.equal(admit(Array.from({ length: 10 }, () => [now, asker("you")]), asker("you"), now), "you ask for at most 10 sessions an hour here");
});

test("a folder is home, a map id or a path the machine reports", () => {
  const known = [["project:atlas", "/home/jo/src/atlas"], ["zone:notes", "/home/jo/notes"]];
  const f = (a) => folder(a, "/home/jo", known, ["~/work/ledger"], { windows: false });
  assert.deepEqual(f("~"), { ok: true, path: "/home/jo" });
  assert.deepEqual(f(""), { ok: true, path: "/home/jo" });
  assert.deepEqual(f("project:atlas"), { ok: true, path: "/home/jo/src/atlas" });
  assert.deepEqual(f("~/src/atlas/"), { ok: true, path: "/home/jo/src/atlas" });
  assert.deepEqual(f("/home/jo/notes"), { ok: true, path: "/home/jo/notes" });
  assert.deepEqual(f("~/work/ledger"), { ok: true, path: "/home/jo/work/ledger" });
  assert.deepEqual(f("/home/jo/work/ledger"), { ok: true, path: "/home/jo/work/ledger" });
  assert.deepEqual(f("/etc"), { ok: false, why: "/etc isn't a project or folder on this machine's map: name one by its id or path, or ~" });
  assert.equal(f("~/src").ok, false);
  assert.equal(f("/HOME/jo/notes").ok, false, "case counts, off Windows");
  // On Windows: its separator, and either slash and any case alike.
  const w = (a) => folder(a, "C:\\Users\\jo", [["project:atlas", "C:\\Users\\jo\\src\\Atlas"]], ["~\\work\\Ledger"], { windows: true });
  assert.deepEqual(w("~/work/ledger"), { ok: true, path: "C:\\Users\\jo\\work\\Ledger" });
  assert.deepEqual(w("c:/users/jo/src/atlas/"), { ok: true, path: "C:\\Users\\jo\\src\\Atlas" });
  assert.deepEqual(w("~\\SRC\\atlas"), { ok: true, path: "C:\\Users\\jo\\src\\Atlas" });
  assert.equal(w("~/src").ok, false);
});

test("a run says its session, and whether Claude Code is signed out", () => {
  assert.equal(sessionOf("claude", `{"type":"system","subtype":"init","session_id":"${ID}"}\n{"type":"result"}`), `claude:${ID}`);
  assert.equal(sessionOf("codex", `{"type":"thread.started","thread_id":"${ID}"}`), `codex:${ID}`);
  assert.equal(sessionOf("claude", 'not json\n{"session_id":"../x"}'), null);
  assert.ok(signedOut("Error: OAuth session expired. Please run /login"));
  assert.ok(signedOut("Not logged in · Please run /login"));
  assert.ok(!signedOut('{"type":"result","subtype":"success"}'));
});

test("the answers in words", () => {
  assert.equal(answerWords("started", "neo-rex", { session: `claude:${ID}`, folder: "~/src/atlas" }), `[XenoNet] Started claude:${ID} on neo-rex, in ~/src/atlas. Message it as you would any session.`);
  assert.equal(answerWords("refused", "neo-rex", { reason: "only you start sessions here" }), "[XenoNet] No session was started on neo-rex: only you start sessions here.");
  assert.equal(answerWords("ended", "this machine", { session: "claude:x", exit: 0, minutes: 4 }), "[XenoNet] claude:x ended (exit 0) after 4 min.");
  assert.equal(signInWords("neo-rex"), "Claude Code on neo-rex needs a sign-in for XenoNet's runs: run xenonet runs sign-in there");
});

test("a sign-in for XenoNet's runs is what claude setup-token prints, kept under XenoNet's own name", () => {
  const token = `sk-ant-oat01-${"a".repeat(80)}`;
  assert.deepEqual(signInToken(`  ${token}\n`), { ok: true, token });
  assert.deepEqual(signInToken(""), { ok: false, why: "paste the token claude setup-token printed" });
  assert.equal(signInToken("hunter2").ok, false);
  assert.equal(signInToken(`${token} extra`).ok, false, "one line, no spaces");
  assert.equal(SIGN_IN_TARGET, "XenoNet/Claude Code token for XenoNet's runs");
});
