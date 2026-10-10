// Summaries built from the live event history: "while you were away", the
// day's log, and handoff briefs. All deterministic; `narrate` optionally asks
// Codex (ephemeral, read-only) to turn a summary into prose.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const base = (p) => String(p).split(/[\\/]/).pop();

export function summarize(events, { since, until = Date.now(), titleOf = (id) => id, live = null } = {}) {
  const per = new Map();
  const get = (e) => {
    let s = per.get(e.sessionId);
    if (!s) per.set(e.sessionId, (s = { id: e.sessionId, app: e.app, title: titleOf(e.sessionId), turns: 0, prompts: 0, tools: {}, toolCalls: 0, files: new Set(), errors: 0, subagents: 0, first: e.ts, last: e.ts, lastMessage: null }));
    s.last = Math.max(s.last, e.ts);
    return s;
  };
  for (const e of events) {
    if (e.ts < since || e.ts > until) continue;
    const s = get(e);
    if (e.kind === "turn-end") s.turns++;
    else if (e.kind === "prompt") s.prompts++;
    else if (e.kind === "tool") {
      s.toolCalls++;
      s.tools[e.category] = (s.tools[e.category] ?? 0) + 1;
      if (e.category === "edit") for (const f of e.files ?? []) s.files.add(base(f));
    } else if (e.kind === "tool-error") s.errors++;
    else if (e.kind === "subagent-start") s.subagents++;
    else if (e.kind === "message") s.lastMessage = e.detail;
  }
  const liveById = new Map((live?.sessions ?? []).map((s) => [s.id, s]));
  const sessions = [...per.values()]
    .map((s) => ({ ...s, files: [...s.files], needsYou: liveById.get(s.id)?.needsYou ?? null }))
    .sort((a, b) => b.toolCalls + b.turns * 5 - (a.toolCalls + a.turns * 5));
  const totals = sessions.reduce(
    (t, s) => ({ sessions: t.sessions + 1, turns: t.turns + s.turns, toolCalls: t.toolCalls + s.toolCalls, files: t.files + s.files.length, errors: t.errors + s.errors, subagents: t.subagents + s.subagents }),
    { sessions: 0, turns: 0, toolCalls: 0, files: 0, errors: 0, subagents: 0 },
  );
  return { since, until, totals, sessions };
}

export function dayLog(events, { now = Date.now(), titleOf, homeOf = () => null, nameOf = (id) => id, live, repos = [] } = {}) {
  const d = new Date(now);
  const midnight = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const sum = summarize(events, { since: midnight, until: now, titleOf, live });
  const hours = new Array(24).fill(0);
  const projects = new Map();
  for (const e of events) {
    if (e.ts < midnight || e.kind !== "tool") continue;
    hours[new Date(e.ts).getHours()]++;
    const home = homeOf(e.sessionId);
    if (home) projects.set(home, (projects.get(home) ?? 0) + 1);
  }
  const busiest = hours.indexOf(Math.max(...hours));
  const today = events.filter((e) => e.ts >= midnight);
  return {
    date: d.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" }),
    ...sum,
    tokens: live?.today ?? 0,
    hours,
    busiestHour: hours[busiest] ? busiest : null,
    firstAt: today[0]?.ts ?? null,
    lastAt: today[today.length - 1]?.ts ?? null,
    topProjects: [...projects.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5).map(([id, n]) => ({ id, name: nameOf(id), toolCalls: n })),
    waiting: (live?.sessions ?? []).filter((s) => s.needsYou && s.needsYou.kind !== "review").length,
    uncommitted: repos.filter((r) => r.dirty > 0),
  };
}

export function summaryText(sum, label) {
  const lines = [`${label}: ${sum.totals.sessions} sessions, ${sum.totals.turns} turns, ${sum.totals.toolCalls} tool calls, ${sum.totals.files} files edited, ${sum.totals.errors} errors.`];
  for (const s of sum.sessions.slice(0, 12)) {
    const tools = Object.entries(s.tools).map(([k, v]) => `${k} ${v}`).join(", ");
    lines.push(`- ${s.app} "${s.title}": ${s.turns} turns, ${s.toolCalls} tool calls (${tools || "none"})${s.files.length ? `, edited ${s.files.slice(0, 6).join(", ")}` : ""}${s.errors ? `, ${s.errors} errors` : ""}${s.needsYou ? `; waiting for you: ${s.needsYou.text}` : ""}${s.lastMessage ? `; last said: ${s.lastMessage}` : ""}`);
  }
  return lines.join("\n");
}

const clipText = (s, n) => (s && s.length > n ? s.slice(0, n - 1) + "…" : s ?? "");

export function handoffBrief({ node, notes, target }) {
  const from = node.app === "codex" ? "a Codex session" : "a Claude Code session";
  const goal = notes.firstPrompt?.text;
  const latest = notes.prompts?.[notes.prompts.length - 1]?.text;
  const last = notes.messages?.[notes.messages.length - 1]?.text;
  const files = [...new Set([...(notes.edits ?? []), ...(notes.files ?? [])])].slice(0, 12);
  const parts = [
    `Continue the work from ${from} titled "${node.title}".`,
    "",
    `Folder: ${node.worktree || node.cwd || "unknown"}${node.branch ? `, on branch ${node.branch}` : ""}`,
  ];
  if (goal) parts.push("", `Original goal: ${clipText(goal, 700)}`);
  if (latest && latest !== goal) parts.push("", `Most recent request: ${clipText(latest, 600)}`);
  if (last) parts.push("", "Where it left off:", clipText(last, target === "codex" ? 900 : 1500));
  if (files.length) parts.push("", "Files it worked on:", ...files.map((f) => `- ${f}`));
  if (notes.needsYou?.kind === "question") parts.push("", `It was waiting on this question from you: ${clipText(notes.needsYou.text, 400)}`);
  parts.push("", "Before changing anything, check the repository's current state with git status and git diff. Then carry on from where it left off.");
  return parts.join("\n");
}

// Ask Codex for a short narrative. --ephemeral keeps it out of your thread
// history; read-only sandbox in a temp folder keeps it from touching anything.
export function narrate(codexBin, text, opts) {
  const prompt = `Write a short status note for the person running these AI coding sessions. In 3 to 6 sentences, say what the sessions finished, what needs their attention first, and anything that looks stuck or risky. Refer to sessions by their titles. Write plainly: short sentences, active voice, everyday words, no em dashes. No headings, bullet points or preamble. Do not run any commands.\n\nActivity data:\n${text}`;
  return askCodex(codexBin, prompt, opts);
}

// One `codex exec` turn on `prompt`, resolving to its last message. With a
// JSON Schema in `schema`, Codex answers in that shape (--output-schema).
export function askCodex(codexBin, prompt, { schema = null, timeoutMs = 120000 } = {}) {
  return new Promise((resolve, reject) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-codex-"));
    const out = path.join(dir, "last.txt");
    const args = ["exec", "--ephemeral", "--skip-git-repo-check", "-s", "read-only", "--color", "never", "-C", dir, "-o", out];
    if (schema) {
      fs.writeFileSync(path.join(dir, "schema.json"), JSON.stringify(schema));
      args.push("--output-schema", path.join(dir, "schema.json"));
    }
    const child = spawn(codexBin, [...args, "-"], { windowsHide: true });
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
