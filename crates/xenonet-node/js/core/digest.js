// core/digest.js in Rust (crates/xenonet-state/src/digest.rs): summaries
// built from the live event history ("while you were away", the day's log,
// handoff briefs). Rust asks the callbacks given here as the JS did. Asking
// Codex is a child process, which stays here: Rust gives its arguments and
// the prompt.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { native } from "../index.js";

export function summarize(events, { since, until = Date.now(), titleOf = (id) => id, live = null } = {}) {
  return native.digestSummarize(events, Number(since), Number(until), titleOf, live);
}

export function dayLog(events, { now = Date.now(), titleOf, homeOf = () => null, nameOf = (id) => id, live, repos = [] } = {}) {
  return native.digestDayLog(events, Number(now), titleOf ?? null, homeOf, nameOf, live, repos);
}

export const summaryText = (sum, label) => native.summaryText(sum, String(label));

export const handoffBrief = ({ node, notes, target }) => native.handoffBrief(node, notes, target == null ? null : String(target));

export const narrate = (codexBin, text, opts) => askCodex(codexBin, native.narratePrompt(String(text)), opts);

const yourCodexHome = () => path.resolve(process.env.CODEX_HOME ?? path.join(os.homedir(), ".codex"));

// One `codex exec` turn on `prompt`, resolving to its last message. With a
// JSON Schema in `schema`, Codex answers in that shape (--output-schema).
// It runs isolated, in a Codex home of its own that shares only the sign-in
// of yours (`codexHome`): Rust gives its arguments and makes that home.
export function askCodex(codexBin, prompt, { schema = null, timeoutMs = 120000, codexHome = yourCodexHome() } = {}) {
  return new Promise((resolve, reject) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-codex-"));
    const out = path.join(dir, "last.txt");
    let schemaFile = null;
    if (schema) {
      schemaFile = path.join(dir, "schema.json");
      fs.writeFileSync(schemaFile, JSON.stringify(schema));
    }
    let home;
    try {
      home = native.codexRunHome(dir, String(codexHome));
    } catch (err) {
      fs.rmSync(dir, { recursive: true, force: true });
      return reject(err);
    }
    const child = spawn(codexBin, native.codexExecArgs(dir, out, schemaFile), { windowsHide: true, env: { ...process.env, CODEX_HOME: home } });
    const timer = setTimeout(() => child.kill(), timeoutMs);
    let stderr = "";
    child.stderr.on("data", (d) => (stderr += d).length > 4000 && (stderr = stderr.slice(-4000)));
    // It ends once: with an exit, or with an error when codex couldn't start
    // (and then never exits).
    let ended = false;
    const end = (settle) => {
      if (ended) return;
      ended = true;
      clearTimeout(timer);
      let msg = "";
      try {
        msg = fs.readFileSync(out, "utf8").trim();
      } catch {}
      fs.rmSync(dir, { recursive: true, force: true });
      settle(msg);
    };
    child.on("error", (err) => end(() => reject(err)));
    child.on("exit", (code) => end((msg) => (msg ? resolve(msg) : reject(new Error(`codex exec exited ${code}: ${stderr.slice(-300)}`)))));
    // A codex that quits before reading all of a long prompt breaks the pipe;
    // its exit says what happened.
    child.stdin.on("error", () => {});
    child.stdin.end(prompt);
  });
}
