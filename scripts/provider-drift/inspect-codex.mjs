// Inspecting a version of the Codex CLI the way XenoNet reads it
// (core/codex.js, core/lines.js), with no account and nothing of the
// machine's:
//
//   - its protocol's JSON Schemas (codex app-server generate-json-schema)
//     against XenoNet's schemas, the methods XenoNet calls and the params it
//     sends;
//   - one turn of `codex exec` against the stand-in Responses API
//     (fake-api.mjs), in a made-up CODEX_HOME (sandbox.mjs), whose rollout
//     is read through XenoNet's reader for rollouts;
//   - `codex app-server` over that home: initialize, thread/list and
//     thread/items/list as XenoNet sends them, and a rename;
//   - the state database the run left, as XenoNet opens it.
//
// The turn calls only a tool that changes nothing (update_plan, or
// get_goal), so no command runs and no sandbox is set up.
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { CodexLine } from "../../core/lines.js";
import { codexVersionOf, satisfies } from "../../core/vendor.js";
import { startFakeApi } from "./fake-api.mjs";
import { makeSandbox, run, filesUnder } from "./sandbox.mjs";
import { npmInstall } from "./releases.mjs";
import { openAppServer } from "./app-server.mjs";
import { readers, checkLines, expect, CODEX_LINES } from "./checks.mjs";
import { methodsOf, codexMethodsIn } from "./types.mjs";
import { PROMPT, RENAMED, REPO, coreSources, typeFindings, steps, xenonetSchema } from "./common.mjs";

const PKG = "@openai/codex";
const ROLLOUT = "openai/codex/rollout-line";
const APP_SERVER = "openai/codex/app-server";
const STATE_DB = "openai/codex/state-db";

// What core/codex.js sends and opens, read from its source so the check
// follows it: the source kinds thread/list asks for, and the state
// database's number.
export function codexConstants(text = fs.readFileSync(path.join(REPO, "core", "codex.js"), "utf8")) {
  let sourceKinds = null;
  try {
    sourceKinds = JSON.parse(/const SOURCE_KINDS = (\[[^\]]*\])/.exec(text)?.[1] ?? "null");
  } catch {}
  const stateDb = Number(/const STATE_DB = (\d+)/.exec(text)?.[1] ?? NaN);
  return { sourceKinds, stateDb: Number.isFinite(stateDb) ? stateDb : null };
}

// An app-server's refusal of a method it doesn't have, as core/codex.js
// tells it (saysUnsupported).
const unsupported = (err) => !!err?.rpc && (err.rpc.code === -32601 || /not (yet )?supported|unsupported|not implemented|unknown method|method not found/i.test(err.rpc.message ?? ""));

// The made-up home's config: the stand-in as the model provider, nothing
// approved or sandboxed (the turn runs no command), and no plugin sync.
export const codexConfig = (apiUrl) => `model = "xenonet-stand-in"
model_provider = "xenonet_stand_in"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[features]
plugins = false
remote_plugin = false

[model_providers.xenonet_stand_in]
name = "XenoNet stand-in"
base_url = "${apiUrl}/v1"
wire_api = "responses"
env_key = "XENONET_STAND_IN_KEY"
`;

// A rollout's expectations, from its lines as XenoNet reads them.
export function rolloutExpectations(lines, version) {
  const view = lines.filter((l) => l && typeof l === "object").map((l) => new CodexLine(l));
  const payloads = view.map((l) => [l.type, l.payload]);
  const prompt = view.some((l) => (l.userMessage ?? l.userEvent ?? "").includes(PROMPT));
  const call = payloads.find(([t, p]) => t === "response_item" && p.type === "function_call");
  const output = payloads.some(([t, p]) => t === "response_item" && p.type === "function_call_output" && (!call || p.call_id === call[1].call_id));
  const tokens = payloads.find(([t, p]) => t === "event_msg" && p.type === "token_count" && p.info?.total_token_usage)?.[1];
  return [
    expect(ROLLOUT, version == null || lines[0]?.type === "session_meta" && lines[0]?.payload?.cli_version === version, `the first line is session_meta, with the version (${version ?? "?"})`),
    expect(ROLLOUT, prompt, "the prompt is a completed user message or a user_message event"),
    expect(ROLLOUT, !!call && output, "the tool call and its output are function_call and function_call_output response items"),
    expect(ROLLOUT, tokens?.info?.total_token_usage?.total_tokens > 0, "token_count events carry info.total_token_usage"),
    expect(ROLLOUT, payloads.some(([t, p]) => t === "event_msg" && p.type === "task_complete"), "the turn ends with a task_complete event"),
  ];
}

export async function inspectCodex({ provider, version, work }) {
  const s = steps();
  const r = readers();
  const findings = [];
  const info = {};
  const dir = await s.run(`npm install ${PKG}@${version}`, () => npmInstall(PKG, version, work));
  if (!dir) return { steps: s.list, findings, read: r.read(), info };
  const box = makeSandbox(work);
  const bin = path.join(dir, "node_modules", "@openai", "codex", "bin", "codex.js");
  const codex = (args, opts = {}) => run(process.execPath, [bin, ...args], { env: box.env, cwd: box.project, timeoutMs: 120_000, ...opts });
  const constants = codexConstants();
  try {
    const said = await s.run(
      "codex --version",
      async () => {
        const res = await codex(["--version"]);
        if (res.code !== 0) throw new Error(`exited ${res.code}: ${res.stderr.trim().slice(-200)}`);
        return res.stdout.trim();
      },
      (out) => out,
    );
    info.version = /(\d+\.\d+\.\d+[\w.+-]*)/.exec(said ?? "")?.[1] ?? null;

    // ----- the protocol's types -----
    const out = path.join(box.root, "schema");
    const generated = await s.run("codex app-server generate-json-schema", async () => {
      const res = await codex(["app-server", "generate-json-schema", "--experimental", "--out", out]);
      if (res.code !== 0) throw new Error(`exited ${res.code}: ${res.stderr.trim().slice(-200)}`);
      return null;
    });
    if (generated !== undefined) {
      const load = (f) => {
        try {
          return JSON.parse(fs.readFileSync(path.join(out, f), "utf8"));
        } catch {
          return null;
        }
      };
      const v2 = load("codex_app_server_protocol.v2.schemas.json");
      const v1 = load("codex_app_server_protocol.schemas.json");
      findings.push(...typeFindings(provider.types, (pair) => (pair.bundle === "v1" ? v1 : v2)));
      const clientRequest = load("ClientRequest.json");
      const methods = methodsOf(clientRequest);
      info.methods = methods.size;
      const used = codexMethodsIn(coreSources());
      info.calls = used;
      for (const m of used) if (!methods.has(m)) findings.push({ source: APP_SERVER, kind: "missing-method", severity: "change", path: m, variant: null, detail: "XenoNet calls it, and the app-server no longer declares it", origin: "api" });
      // The params XenoNet sends.
      const defs = v2?.definitions ?? {};
      for (const [method, params] of Object.entries(provider.params ?? {})) {
        const def = defs[methods.get(method)];
        if (!def) continue;
        for (const p of params)
          if (!def.properties?.[p]) findings.push({ source: APP_SERVER, kind: "missing-param", severity: "change", path: `${method} ${p}`, variant: null, detail: "XenoNet sends it, and the app-server no longer takes it", origin: "api" });
      }
      const kinds = defs.ThreadSourceKind?.enum;
      if (Array.isArray(kinds) && constants.sourceKinds)
        for (const k of constants.sourceKinds)
          if (!kinds.includes(k)) findings.push({ source: APP_SERVER, kind: "missing-param", severity: "change", path: "thread/list sourceKinds", variant: null, detail: `XenoNet asks for ${JSON.stringify(k)}, and the app-server no longer lists it`, origin: "api" });
    }

    // ----- a turn -----
    const api = await startFakeApi();
    fs.writeFileSync(path.join(box.codexHome, "config.toml"), codexConfig(api.url));
    fs.writeFileSync(path.join(box.project, "README.md"), "# A made-up project\n");
    const env = { ...box.env, XENONET_STAND_IN_KEY: "stand-in" };
    try {
      await s.run("codex exec a turn against the stand-in API", async () => {
        const res = await codex(["exec", "--skip-git-repo-check", PROMPT], { env, input: "", timeoutMs: 180_000 });
        if (res.code !== 0) throw new Error(`codex exec exited ${res.code}${res.timedOut ? " (timed out)" : ""}: ${res.stderr.trim().slice(-300)}`);
        info.requests = api.requests.map((q) => `${q.method} ${q.path} → ${q.reply ?? "?"}`);
        return null;
      });
    } finally {
      await api.close();
    }
    const rollouts = filesUnder(path.join(box.codexHome, "sessions"), /^rollout-.*\.jsonl$/);
    const lines = [];
    for (const file of rollouts) lines.push(...checkLines(r.reader(ROLLOUT), fs.readFileSync(file, "utf8"), CODEX_LINES).lines);
    info.rolloutLines = lines.length;
    if (!rollouts.length) findings.push(expect(ROLLOUT, false, "a rollout is written under sessions/"));
    else findings.push(...rolloutExpectations(lines, info.version));

    // ----- the app-server, over the home the turn left -----
    await s.run("codex app-server: initialize, thread/list, thread/items/list, a rename", async () => {
      const srv = openAppServer(process.execPath, [bin], { env, cwd: box.project });
      const answers = r.reader(APP_SERVER);
      try {
        const init = await srv.request("initialize", { clientInfo: { name: "xenonet", title: "XenoNet", version: "0.0.0" }, capabilities: { experimentalApi: true } });
        srv.notify("initialized");
        const v = codexVersionOf(init?.userAgent);
        answers.check({ initialize: init }, { version: v });
        findings.push(expect(APP_SERVER, v === info.version, `initialize's userAgent says the version (${v ?? "none"})`));
        const list = async () => {
          const threads = [];
          for (const archived of [false, true]) {
            const res = await srv.request("thread/list", { archived, limit: 100, cursor: null, sourceKinds: constants.sourceKinds ?? undefined, useStateDbOnly: true });
            answers.check({ "thread/list": res }, { version: v });
            threads.push(...(res?.data ?? []));
          }
          return threads;
        };
        const threads = await list();
        const thread = threads.find((t) => t?.preview?.includes(PROMPT));
        findings.push(expect(APP_SERVER, !!thread, "thread/list lists the turn's thread, with the prompt as its preview"));
        if (thread) {
          // Older app-servers don't list a thread's items; XenoNet reads the
          // thread from its rollout then (core/codex.js), so that's a note.
          try {
            const items = await srv.request("thread/items/list", { threadId: thread.id, limit: 50, cursor: null, sortDirection: "desc" });
            answers.check({ "thread/items/list": items }, { version: v });
            const types = (items?.data ?? []).map((e) => e?.item?.type);
            findings.push(expect(APP_SERVER, types.includes("userMessage") && types.includes("agentMessage"), "thread/items/list gives the prompt and the reply as userMessage and agentMessage items"));
          } catch (err) {
            if (!unsupported(err)) throw err;
            findings.push(expect(APP_SERVER, false, "thread/items/list isn't answered in this version; XenoNet reads the thread from its rollout instead", { required: false }));
          }
          // Renaming, which a person does from XenoNet: a change when XenoNet
          // calls it, a note while it doesn't yet.
          const required = codexMethodsIn(coreSources()).includes("thread/name/set");
          let renamed = null;
          let why = "";
          try {
            await srv.request("thread/name/set", { threadId: thread.id, name: RENAMED });
            renamed = (await list()).find((t) => t?.id === thread.id);
          } catch (err) {
            why = `: ${err.message}`;
          }
          findings.push(expect(APP_SERVER, renamed?.name === RENAMED, `after thread/name/set, thread/list gives the new name${why}`, { required }));
        }
        info.notifications = [...new Set(srv.notifications.map((n) => n.method))].sort();
      } finally {
        await srv.close();
      }
      return null;
    });

    // ----- the state database, as XenoNet opens it -----
    const db = r.reader(STATE_DB);
    const range = xenonetSchema(STATE_DB)?.["x-xenonet"]?.vendorVersions ?? null;
    const files = fs.readdirSync(box.codexHome).filter((n) => /^state_\d+\.sqlite$/.test(n));
    info.stateDbs = files;
    for (const name of files) {
      const n = /^state_(\d+)\.sqlite$/.exec(name)[1];
      if (range && !satisfies(n, range)) db.pick(n);
    }
    const mine = constants.stateDb != null && `state_${constants.stateDb}.sqlite`;
    findings.push(expect(STATE_DB, !!mine && files.includes(mine), `Codex keeps its state in ${mine || "the database XenoNet opens"}`));
    if (mine && files.includes(mine)) {
      const handle = new DatabaseSync(path.join(box.codexHome, mine), { readOnly: true });
      try {
        db.check({ projects: handle.prepare("select id, name from projects").all(), project_roots: handle.prepare("select project_id, path from project_roots order by position").all() }, { version: String(constants.stateDb) });
      } catch {
        for (const [table, columns] of [["projects", ["id", "name"]], ["project_roots", ["project_id", "path"]]]) {
          let have = [];
          try {
            have = handle.prepare(`pragma table_info(${table})`).all().map((c) => c.name);
          } catch {}
          if (!have.length) db.missing(table, String(constants.stateDb));
          else for (const c of columns) if (!have.includes(c)) db.missing(`${table}[].${c}`, String(constants.stateDb));
        }
      } finally {
        handle.close();
      }
    }
    return { steps: s.list, findings: [...findings, ...r.findings()], read: r.read(), info };
  } finally {
    box.remove();
    fs.rmSync(dir, { recursive: true, force: true });
  }
}
