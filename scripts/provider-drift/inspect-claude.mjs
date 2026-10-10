// Inspecting a version of the Agent SDK, and of Claude Code, the way XenoNet
// reads them (core/claude.js, core/lines.js), with no account and nothing
// of the machine's: each runs one turn against the stand-in Messages API
// (fake-api.mjs) in a made-up home (sandbox.mjs), and what it wrote and
// returned is read through XenoNet's own readers (checks.mjs).
//
// Claude Code runs with --bare, so it looks for no CLAUDE.md above the
// project folder, reads no keychain and runs no hooks: the made-up home
// can sit under a real one (a temporary folder on Windows does) without
// the run reading anything there.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { ClaudeLine } from "../../core/lines.js";
import { startFakeApi } from "./fake-api.mjs";
import { makeSandbox, run, filesUnder } from "./sandbox.mjs";
import { npmInstall, installedPackage } from "./releases.mjs";
import { readers, checkLines, expect, CLAUDE_LINES } from "./checks.mjs";
import { schemaFromDts, functionsOf, importsFrom } from "./types.mjs";
import { PROMPT, MODEL, RENAMED, coreSources, typeFindings, steps, platformBinary } from "./common.mjs";

const SDK = "@anthropic-ai/claude-agent-sdk";
const CLI = "@anthropic-ai/claude-code";
const LISTING = "anthropic/agent-sdk/session-listing";
const MESSAGE = "anthropic/agent-sdk/session-message";
const TRANSCRIPT = "anthropic/claude-code/transcript-line";
const CONFIG = "anthropic/claude-code/config";

const API_KEY = "sk-ant-stand-in-for-the-drift-check";

// The kinds of line the SDK's own session functions write (renameSession,
// tagSession), as opposed to its Claude Code's.
const SDK_LINES = new Set(["custom-title", "tag"]);
const typeOfLine = (text) => {
  try {
    return JSON.parse(text)?.type ?? null;
  } catch {
    return null;
  }
};

// A turn's expectations, from its transcript's lines as XenoNet reads them:
// your prompt, the Read call and its result, the reply's tokens, and the
// turn's end.
export function transcriptExpectations(lines, version) {
  const view = lines.filter((l) => l && typeof l === "object").map((l) => new ClaudeLine(l));
  const assistant = view.filter((l) => l.type === "assistant");
  const user = view.filter((l) => l.type === "user");
  const usage = assistant.map((l) => l.usage).find(Boolean);
  return [
    expect(TRANSCRIPT, user.some((l) => !l.meta && !l.hasResults && l.text.includes(PROMPT)), "the prompt is a user line with its text"),
    expect(TRANSCRIPT, assistant.some((l) => l.blocks.some((b) => b?.type === "tool_use" && b.name === "Read")), "the Read call is a tool_use block in an assistant line"),
    expect(TRANSCRIPT, user.some((l) => l.results?.length), "its result is a tool_result in a user line"),
    expect(TRANSCRIPT, usage?.input === 12 && usage?.output === 8, "the reply's tokens are read from message.usage"),
    expect(TRANSCRIPT, assistant.some((l) => l.endTurn), "the turn ends with stop_reason end_turn"),
    ...(version ? [expect(TRANSCRIPT, lines.some((l) => l?.version === version), `lines say the version that wrote them (${version})`)] : []),
  ];
}

// ---------- the Agent SDK ----------

export async function inspectAgentSdk({ provider, version, work }) {
  const s = steps();
  const r = readers();
  const findings = [];
  const info = {};
  const dir = await s.run(`npm install ${SDK}@${version}`, () => npmInstall(SDK, version, work));
  if (!dir) return { steps: s.list, findings, read: r.read(), info };
  try {
    const root = path.join(dir, "node_modules", ...SDK.split("/"));
    const pkg = installedPackage(dir, SDK);
    info.version = pkg.version;
    info.claudeCodeVersion = pkg.claudeCodeVersion ?? null;

    // Its types, and the functions XenoNet imports from it.
    const dts = await s.run("read sdk.d.ts", () => fs.readFileSync(path.join(root, pkg.types ?? "sdk.d.ts"), "utf8"));
    if (dts) {
      const doc = schemaFromDts(dts, provider.types.map((t) => t.provider));
      findings.push(...typeFindings(provider.types, () => doc));
      const declared = functionsOf(dts);
      const used = importsFrom(coreSources(), SDK);
      info.imports = used;
      for (const name of used)
        if (!declared.includes(name)) findings.push({ source: LISTING, kind: "missing-export", severity: "change", path: name, variant: null, detail: "XenoNet imports it, and the SDK no longer exports it", origin: "api" });
    }

    // A turn through the SDK's own Claude Code, then the SDK's session
    // functions on what it wrote, in a child process with the made-up home.
    const box = makeSandbox(work);
    const readme = path.join(box.project, "README.md");
    fs.writeFileSync(readme, "# A made-up project\n");
    const api = await startFakeApi({ readFile: readme });
    try {
      const probe = await s.run("run a turn, then list, read and rename the session", async () => {
        const res = await run(process.execPath, [fileURLToPath(new URL("./sdk-probe.mjs", import.meta.url)), pathToFileURL(path.join(root, "sdk.mjs")).href, box.project, MODEL], {
          env: { ...box.env, ANTHROPIC_BASE_URL: api.url, ANTHROPIC_API_KEY: API_KEY },
          cwd: box.project,
          timeoutMs: 180_000,
        });
        if (res.code !== 0) throw new Error(`the probe exited ${res.code}${res.timedOut ? " (timed out)" : ""}: ${res.stderr.trim().slice(-300)}`);
        return JSON.parse(res.stdout);
      });
      if (probe) {
        info.exports = probe.exports;
        const st = probe.steps;
        // The turn is the check's own doing; the calls after it are what
        // XenoNet makes.
        if (st.query) s.list.push({ step: "the SDK's query() runs a turn", ok: st.query.ok, detail: st.query.ok ? null : st.query.error });
        for (const [name, step] of Object.entries(st))
          if (!step.ok && name !== "query") {
            const used = (info.imports ?? []).includes(name.replace(/AfterRename$/, ""));
            findings.push({ source: name.startsWith("getSessionMessages") ? MESSAGE : LISTING, kind: "call-failed", severity: used ? "change" : "info", path: name, variant: null, detail: `failed: ${step.error}`, origin: "api" });
          }
        const listings = r.reader(LISTING);
        const messages = r.reader(MESSAGE);
        for (const item of st.listSessions?.value ?? []) listings.check(item, { version: info.version });
        for (const item of st.listSessionsAfterRename?.value ?? []) listings.check(item, { version: info.version });
        if (st.getSessionInfo?.value) listings.check(st.getSessionInfo.value, { version: info.version });
        for (const m of st.getSessionMessages?.value ?? []) messages.check(m, { version: info.version, kind: m?.type });
        const listed = (st.listSessions?.value ?? []).find((x) => x?.sessionId === probe.sessionId);
        findings.push(expect(LISTING, !!listed && listed.summary === PROMPT, "listSessions lists the session, with the prompt as its summary"));
        const blocks = (st.getSessionMessages?.value ?? []).flatMap((m) => (Array.isArray(m?.message?.content) ? m.message.content : []));
        findings.push(expect(MESSAGE, blocks.some((b) => b?.type === "tool_use" && b.name === "Read") && blocks.some((b) => b?.type === "tool_result"), "getSessionMessages gives the Read call and its result as content blocks"));
        // Renaming, which a person does from XenoNet: a change when XenoNet
        // imports renameSession, a note while it doesn't yet.
        const required = (info.imports ?? []).includes("renameSession");
        if (st.renameSession) {
          const renamed = (st.listSessionsAfterRename?.value ?? []).find((x) => x?.sessionId === probe.sessionId);
          findings.push(expect(LISTING, renamed?.customTitle === RENAMED, "after renameSession, listSessions gives the new title as customTitle", { required }));
        } else findings.push(expect(LISTING, false, "the SDK exports renameSession", { required }));
      }
      // The lines the SDK itself added to the transcript (a rename's title).
      // The rest were written by its Claude Code, which the Claude Code
      // check reads.
      const t = r.reader(TRANSCRIPT);
      for (const file of filesUnder(path.join(box.claudeConfig, "projects"), /\.jsonl$/)) {
        const own = fs
          .readFileSync(file, "utf8")
          .split("\n")
          .filter((l) => SDK_LINES.has(typeOfLine(l)));
        if (own.length) checkLines(t, own.join("\n"), CLAUDE_LINES);
      }
    } finally {
      await api.close();
      box.remove();
    }
    return { steps: s.list, findings: [...findings, ...r.findings()], read: r.read(), info };
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

// ---------- Claude Code ----------

export async function inspectClaudeCode({ version, work }) {
  const s = steps();
  const r = readers();
  const findings = [];
  const info = {};
  const dir = await s.run(`npm install ${CLI}@${version}`, () => npmInstall(CLI, version, work));
  if (!dir) return { steps: s.list, findings, read: r.read(), info };
  const box = makeSandbox(work);
  try {
    const bin = platformBinary(dir, CLI, "claude");
    if (!bin) {
      s.skip("run claude", `no ${CLI} binary for ${process.platform}-${process.arch}`);
      return { steps: s.list, findings, read: r.read(), info };
    }
    const said = await s.run(
      "claude --version",
      async () => {
        const res = await run(bin, ["--version"], { env: box.env, cwd: box.project, timeoutMs: 60_000 });
        if (res.code !== 0) throw new Error(`exited ${res.code}: ${res.stderr.trim().slice(-200)}`);
        return res.stdout.trim();
      },
      (out) => out,
    );
    info.version = /(\d+\.\d+\.\d+[\w.+-]*)/.exec(said ?? "")?.[1] ?? null;

    const readme = path.join(box.project, "README.md");
    fs.writeFileSync(readme, "# A made-up project\n");
    const api = await startFakeApi({ readFile: readme });
    try {
      const args = ["-p", PROMPT, "--bare", "--strict-mcp-config", "--disable-slash-commands", "--output-format", "json", "--model", MODEL, "--max-turns", "4"];
      await s.run("run a turn against the stand-in API", async () => {
        const res = await run(bin, args, { env: { ...box.env, ANTHROPIC_BASE_URL: api.url, ANTHROPIC_API_KEY: API_KEY }, cwd: box.project, timeoutMs: 180_000 });
        if (res.code !== 0) throw new Error(`claude -p exited ${res.code}${res.timedOut ? " (timed out)" : ""}: ${(res.stderr || res.stdout).trim().slice(-300)}`);
        info.requests = api.requests.map((q) => `${q.method} ${q.path} → ${q.reply ?? "?"}`);
        return null;
      });
    } finally {
      await api.close();
    }

    const files = filesUnder(path.join(box.claudeConfig, "projects"), /\.jsonl$/);
    const lines = [];
    for (const file of files) lines.push(...checkLines(r.reader(TRANSCRIPT), fs.readFileSync(file, "utf8"), CLAUDE_LINES).lines);
    info.transcriptLines = lines.length;
    if (!files.length) findings.push(expect(TRANSCRIPT, false, "a transcript is written under projects/"));
    else findings.push(...transcriptExpectations(lines, info.version));

    // ~/.claude.json, which an API key run writes without an account.
    const configFile = path.join(box.claudeConfig, ".claude.json");
    if (fs.existsSync(configFile)) {
      try {
        r.reader(CONFIG).check(JSON.parse(fs.readFileSync(configFile, "utf8")));
      } catch {
        r.reader(CONFIG).unparsable(null);
      }
    }
    return { steps: s.list, findings: [...findings, ...r.findings()], read: r.read(), info };
  } finally {
    box.remove();
    fs.rmSync(dir, { recursive: true, force: true });
  }
}
