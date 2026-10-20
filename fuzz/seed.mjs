// Writes the fuzz targets' seed inputs, fuzz/seeds/<target>/, from the
// repo's test fixtures and the demo's made-up world (see "Fuzzing" in
// docs/guide/development.md). Nothing here reads anyone's real data, and
// the script stops before writing a seed that holds this machine's home
// folder, user name or temp folder.
//
//   node fuzz/seed.mjs
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.join(repo, "fuzz", "seeds");
const T = Date.parse("2026-10-05T12:00:00.000Z");

// The demo writes its world under the temp folder, by port: a folder of
// its own here, gone when the seeds are written.
const tmp = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-seeds-")));
process.env.TMPDIR = tmp;
process.env.XENONET_PORT = "47999";
const { createWorld } = await import("../core/demo.js");
const world = createWorld(T);
const { VENDOR } = await import("../test/fixtures/vendor.js");
const hooks = await import("../test/fixtures/hook-payloads.js");
const numbat = await import("../test/fixtures/numbat.js");

const HOME = "/home/nova";
const clean = (text) => text.split(world.root).join(`${HOME}/demo`).split(tmp).join("/tmp/demo");
// The demo's own people are made up; anyone else's name stays out.
const user = os.userInfo().username;
const forbidden = [os.homedir(), tmp, world.root, ["nova", "ada"].includes(user) ? "" : user].filter((s) => s && s.length > 2);

fs.rmSync(out, { recursive: true, force: true });
let written = 0;
function seed(target, name, ...parts) {
  const data = Buffer.concat(parts.map((p) => (typeof p === "number" ? Buffer.from([p]) : Buffer.from(typeof p === "string" ? clean(p) : p))));
  const text = data.toString("latin1");
  for (const f of forbidden) {
    if (text.includes(f)) throw new Error(`${target}/${name} would hold this machine's ${f === user ? "user name" : "folders"}`);
  }
  fs.mkdirSync(path.join(out, target), { recursive: true });
  fs.writeFileSync(path.join(out, target, name), data);
  written++;
}

function files(dir, test) {
  const found = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (test(e.name)) found.push(p);
    }
  };
  if (fs.existsSync(dir)) walk(dir);
  return found;
}

// A transcript's first lines: enough to reach each reader, small enough
// to mutate quickly.
const head = (file, n = 30) => clean(fs.readFileSync(file, "utf8")).split("\n").slice(0, n).join("\n") + "\n";
const jsonl = (records) => records.map((r) => JSON.stringify(r)).join("\n") + "\n";

const claude = files(world.claudeRoot, (n) => n.endsWith(".jsonl")).slice(0, 4);
const codex = files(path.join(world.codexHome, "sessions"), (n) => n.endsWith(".jsonl")).slice(0, 3);
const transcriptLines = VENDOR["anthropic/claude-code/transcript-line"].fits;
const rolloutLines = VENDOR["openai/codex/rollout-line"].fits;

// ---------- json, lines ----------
for (const [i, r] of [...transcriptLines, ...rolloutLines].entries()) seed("json", `vendor-${i}`, JSON.stringify(r));
seed("json", "nested", "[".repeat(600) + "]".repeat(600));
seed("json", "surrogates", JSON.stringify({ a: "\ud83d", b: "x\u2028y" }).replace("\\ud83d", "\\ud83d"));
seed("json", "numbers", "[1e400,-0,0.1,9007199254740993,1e21,5e-7,123456789012345678901234567890]");
claude.forEach((f, i) => seed("lines", `claude-${i}.jsonl`, head(f)));
codex.forEach((f, i) => seed("lines", `codex-${i}.jsonl`, head(f)));
seed("lines", "half-line", jsonl(transcriptLines.slice(0, 2)), '{"type":"user","message":{"content":"caf\u00e9');

// ---------- telemetry, transcript_files ----------
claude.forEach((f, i) => {
  seed("telemetry", `claude-${i}`, 0, head(f, 60));
  seed("telemetry", `claude-hooks-${i}`, 4, head(f, 20));
  seed("transcript_files", `claude-${i}`, 0, head(f, 60), "\0", JSON.stringify({ customTitle: "Fix the tokenizer" }));
});
codex.forEach((f, i) => {
  seed("telemetry", `codex-${i}`, 1, head(f, 60));
  seed("transcript_files", `codex-${i}`, 1, head(f, 60));
});
seed("telemetry", "vendor-claude", 2, jsonl(transcriptLines));
seed("telemetry", "vendor-codex", 1, jsonl(rolloutLines));
seed("telemetry", "hook-bodies", 4, jsonl([hooks.claude.postToolUse("0190d3e0-0000-7000-8000-000000000001", "/w/atlas"), hooks.claude.stop("0190d3e0-0000-7000-8000-000000000001", "/w/atlas")]));
seed("transcript_files", "pasted-cut", 0, jsonl([{ type: "user", uuid: "u1", timestamp: "2026-10-05T12:00:00.000Z", message: { content: '<pasted_content id="\u20ac\u20ac">' } }]));
seed("transcript_files", "pasted", 0, jsonl([{ type: "user", uuid: "u1", timestamp: "2026-10-05T12:00:00.000Z", message: { content: 'see\n<pasted_content id="a1b2">\nbody\n</pasted_content id="a1b2">' } }]));

// ---------- schema ----------
// Vendor sources by index, as the target lists them (sorted, like
// lenient::vendor_sources); XenoNet's own documents after.
const vendorSources = files(path.join(repo, "schemas", "vendor"), (n) => /^v[1-9]\d*\.json$/.test(n))
  .map((f) => path.relative(path.join(repo, "schemas", "vendor"), path.dirname(f)).split(path.sep).join("/"))
  .filter((s, i, all) => all.indexOf(s) === i)
  .sort();
for (const [source, { fits, odd }] of Object.entries(VENDOR)) {
  const i = vendorSources.indexOf(source);
  if (i < 0) continue;
  [...fits, ...odd.map((o) => o.record)].slice(0, 6).forEach((r, k) => {
    const body = JSON.stringify(r);
    if (body !== undefined) seed("schema", `${source.replaceAll("/", "-")}-${k}`, (2 * i) % 256, 1, body);
  });
}
const state = clean(fs.readFileSync(path.join(world.stateDir, "state.json"), "utf8"));
for (let i = 1; i < 80; i += 2) seed("schema", `state-${i}`, i, 1, state);

// ---------- base ----------
const base = [
  [0, "caf\u00e9 \u{1F600} x\0-2\0 5"],
  [0, '{"10":1,"b":2,"2":3,"a":[1,[2,[3]]]}\0, '],
  [1, "2026-10-05T12:00:00.000Z"],
  [1, "+275760-09-13T00:00:00.000Z"],
  [1, "2026-02-30 12:00:00.123456+0200"],
  [2, "/Users/nova/Projects/atlas\0/Users/nova\0/usr/bin:/bin"],
  [2, "C:\\Users\\nova\\Projects\0C:\\Users\\nova\0.exe;.cmd"],
  [3, "C:\\a\\b\0C:\\a\\c\\d\0C:\\"],
  [11, "/a/b/../c\0../d\0/home/nova"],
  [4, "git@github.com:atlas-co/atlas-api.git"],
  // What crashed before, so each run tries it again.
  [4, "gh:\u00e9abc"],
  [3, "C:\\a\u0000C:\\\u0130\u0130"],
  [4, "https://user@github.com:443/nova-dev/ledger.git"],
  [4, "@vm1:actor:grok-bot\0vm1\0self"],
  [4, "https://xenonet.dev/schemas/state/v2.json\u00002"],
  [5, "claude-opus-5-5\u0000120\u0000300"],
  [5, "us.anthropic.claude-sonnet-5-5-v1:0[1m]\u0000100\u0000200"],
  [6, "token sk-live-CANARY0123456789abcdef in /home/nova/x\0/home/nova\0{\"a\":{\"b\":1}}"],
  [6, '[{"at":1,"level":"error","code":"x"},{"level":"warn","code":"x"}]\0/home/nova'],
  [7, "1234.5678\u00003\u0000things"],
];
base.forEach(([pick, text], i) => seed("base", `base-${i}`, pick, text));

// ---------- sources ----------
const appServer = VENDOR["openai/codex/app-server"].fits;
seed("sources", "stdout", 0, jsonl([{ id: 1, result: { data: appServer.slice(0, 2) } }, { id: 2, method: "item/approval", params: {} }, { id: 3, error: { code: -32601, message: "Method not found" } }]), "\0", "warning: x\nerror: y\n");
appServer.slice(0, 4).forEach((t, i) => seed("sources", `thread-${i}`, 1, JSON.stringify(t), "\0", JSON.stringify("rollout:3")));
seed("sources", "rollout", 2, jsonl(rolloutLines));
seed("sources", "claude", 3, jsonl(VENDOR["anthropic/agent-sdk/session-listing"].fits), "\0", jsonl(VENDOR["anthropic/claude-desktop/session"].fits));
seed("sources", "messages", 3, jsonl(VENDOR["anthropic/agent-sdk/session-message"].fits));
seed("sources", "git", 4, "## main...origin/main [ahead 1, behind 2]\n M src/a.ts\n?? new.md\nR  old -> new\n", "\0", "a\nb");
seed("sources", "git-config", 4, "global\0filter.lfs.smudge\ngit-lfs smudge -- %f\0local\0core.fsmonitor\ntrue\0");
seed("sources", "usage", 5, JSON.stringify(VENDOR["anthropic/oauth/usage"].fits[0] ?? {}), "\0", JSON.stringify(VENDOR["anthropic/claude-desktop/plan-usage"].fits[0]));
seed("sources", "credentials", 5, JSON.stringify({ claudeAiOauth: { accessToken: "made-up", expiresAt: T + 3600e3 } }));
seed("sources", "tool-edit", 6, JSON.stringify({ file_path: "/w/atlas/a.ts", old_string: "a", new_string: "b" }), "\0", "Edit");
seed("sources", "tool-patch", 6, JSON.stringify({ input: "*** Begin Patch\n*** Update File: src/a.ts\n@@\n-a\n+b\n*** End Patch" }), "\0", "apply_patch");
seed("sources", "tool-mcp", 6, JSON.stringify({ pages: ["atlas/refunds"] }), "\0", "mcp__agent-wiki__read");
seed("sources", "refusals", 7, jsonl([{ id: "x", provider: "anthropic", kind: "refusal", at: T, model: "claude-opus-5-5" }]), "\0", JSON.stringify({ since: 0, limit: 5 }));
seed("sources", "runs", 8, "actor:grok-bot\trun-1\tclaude\tFix it\tworking\nactor:grok-bot\trun-1\tclaude\t\tdone\n", "\0", '"a"\n"a"\nnull\n');

// ---------- native ----------
const gemini = VENDOR["google/gemini-cli/chat-record"].fits;
seed("native", "gemini", 0, JSON.stringify(gemini[0] ?? {}));
seed("native", "gemini-lines", 0, jsonl(gemini));
seed("native", "copilot", 1, jsonl(VENDOR["github/copilot-cli/session-event"].fits));
const oc = VENDOR["anomaly/opencode/db"].fits[0] ?? {};
const cols = (names) => names.map((name) => ({ name }));
seed("native", "opencode", 2, JSON.stringify({ s: cols(["id", "parent_id", "directory", "title", "version", "time_created", "time_updated", "time_archived"]), p: cols(["session_id", "time_created", "data"]), v: { v: "1.19.3" }, sessions: oc.session ?? [], parts: oc.edits ?? [] }));
seed("native", "cursor", 3, jsonl(VENDOR["anysphere/cursor/hook-input"].fits.map((r) => ({ at: T - 1000, ...r }))));
seed("native", "numbat", 4, numbat.ndjson(numbat.STREAM(T)));

// ---------- wiki ----------
const pages = files(path.join(world.wikiDir, "pages"), (n) => n.endsWith(".md"));
const logs = files(path.join(world.wikiDir, "log"), (n) => n.endsWith(".md"));
const notes = files(path.join(world.wikiDir, "inbox"), (n) => n.endsWith(".md"));
const read = (f) => (f ? clean(fs.readFileSync(f, "utf8")) : "");
pages.forEach((p, i) => seed("wiki", `page-${i}`, read(p), "\0", read(logs[0]).replaceAll(/\d{4}-\d{2}-\d{2}/g, "2026-10-05"), "\0", read(notes[0])));
seed("wiki", "frontmatter", '---\ntitle: "Refunds"\ntags: [atlas-api, "labs/geofence"]\nprojects:\n  - atlas-api\nupdated: 2026-10-05T12:00:00.000Z\n---\nSee [[atlas-api]] and [[q4-close|the close]].\n');

// ---------- http, hooks, state ----------
const wire = (method, target, headers, body = "") => `${method} ${target}\n${Object.entries(headers).map(([k, v]) => `${k}: ${v}`).join("\n")}\n\n${body}`;
const gateway = JSON.parse(fs.readFileSync(path.join(repo, "crates/xenonet-service/tests/fixtures/gateway.json"), "utf8"));
gateway.cases.slice(0, 40).forEach((c, i) => seed("http", `gateway-${i}`, 1, wire(c.request.method, c.request.path, c.request.headers, c.request.body ?? "").replaceAll("127.0.0.1:PORT", "127.0.0.1:47999")));
const S1 = "0190d3e0-0000-7000-8000-000000000001";
const local = { host: "127.0.0.1:47999", "content-type": "application/json", "x-xenonet-api-version": "1" };
const claudeHooks = [hooks.claude.postToolUse(S1, "/w/atlas"), hooks.claude.preToolUse(S1, "/w/atlas", "/w/atlas/src/a.ts"), hooks.claude.stop(S1, "/w/atlas"), hooks.claude.subagentStop(S1, "/w/atlas")];
const codexHooks = [hooks.codex.postToolUse(S1, "/w/atlas")];
claudeHooks.forEach((h, i) => {
  seed("http", `hook-claude-${i}`, 0, wire("POST", "/api/hook/claude", local, JSON.stringify(h)));
  seed("hooks", `claude-${i}`, 0, JSON.stringify(h), "\0", "src/**");
});
codexHooks.forEach((h, i) => {
  seed("http", `hook-codex-${i}`, 0, wire("POST", "/api/hook/codex", local, JSON.stringify(h)));
  seed("hooks", `codex-${i}`, 1, JSON.stringify(h), "\0", "/w/atlas/src/*.ts");
});
seed("hooks", "patch", 1, JSON.stringify({ session_id: S1, hook_event_name: "PreToolUse", cwd: "/w/atlas", tool_name: "apply_patch", tool_input: { command: ["apply_patch", "*** Begin Patch\n*** Update File: src/a.ts\n@@\n-a\n+b\n*** Add File: ../b.ts\n+x\n*** End Patch"] } }), "\0", "src/");
seed("http", "listen", 0, wire("POST", "/api/hook/claude/listen?seconds=5", local, JSON.stringify(hooks.claude.stop(S1, "/w/atlas"))));
for (const [m, t] of [["GET", "/api/state?scope=7d"], ["GET", "/api/search?q=refund&limit=5"], ["GET", "/api/refusals?since=0&provider=anthropic"], ["GET", "/api/actors/requests"], ["POST", "/api/engine/send"], ["GET", "/api/hosts/vm1/graph"], ["POST", "/api/layout"], ["GET", "/api/stats?session=claude%3A0190d3e0-0000-7000-8000-000000000001"]]) {
  seed("http", `route-${m}-${t.replace(/[^a-z]+/gi, "-")}`, 0, wire(m, t, local, m === "POST" ? JSON.stringify({ to: "claude:x", text: "hi", positions: { core: { x: 1, y: 2, z: 3 } } }) : ""));
}
seed("state", "demo", state, "\0", JSON.stringify({ core: { x: 1, y: 2, z: 3 }, "project:/home/nova/projects/atlas-api": null }));
seed("state", "v1", JSON.stringify({ layout: { core: { x: 1, y: 2, z: 3 } }, acks: { [`claude:${S1}`]: T }, marks: {}, projects: { rules: { "/home/nova/projects/labs": { path: "/home/nova/Projects/labs", as: "group", at: T } }, codex: { ranAt: T, ms: 9000, fingerprint: "0123456789abcdef", sent: 12, picks: [{ path: "labs/snippets", as: "project", reason: "Its own pyproject.toml." }] } }, identities: { names: { "github:github.com/atlas-co": "work" }, hidden: {}, pins: {}, known: {}, signins: { claude: [{ id: "claude:a:b", from: T - 1000, to: T }] } }, theme: "phosphor" }));
seed("state", "future", JSON.stringify({ version: 3, layout: {} }));

// ---------- hosts ----------
const golden = JSON.parse(fs.readFileSync(path.join(repo, "test/golden/state.json"), "utf8"));
const graph = { ...golden.graph, nodes: golden.graph.nodes.slice(0, 12), links: (golden.graph.links ?? []).slice(0, 12) };
const hello = { t: "hello", schema: "host/hello/v1", protocol: 1, version: "0.4.0", host: { id: "vm1", key: "9a2b0000000000000000000000000002" }, platform: "linux", demo: false, now: T, sinceMs: 7 * 86400e3, detail: false };
const snapshot = { t: "snapshot", at: T, graph, live: { at: T, sessions: [] }, engine: null };
const frames = (fs) => fs.map((f) => JSON.stringify(f).replaceAll("<world>", "/home/nova")).join("\n") + "\n";
seed("hosts", "link", 128, frames([hello, snapshot, { t: "event", name: "live", data: { sessions: [] } }, { t: "time", id: 1, now: T + 5 }, { t: "res", id: 1, status: 200, body: {} }, { t: "ping" }, { t: "bye", code: 0 }]));
seed("hosts", "hello-only", 0, frames([hello]));
seed("hosts", "relay", 1, frames([{ t: "get", id: 1, path: "/api/graph" }, { t: "get", id: 2, path: "/api/mcp?server=agent-wiki" }, { t: "time", id: 3 }, { t: "get", id: 4, path: "/api/../etc", target: "nova@devbox", id_: "vm1", port: 47840, since: "7d" }]), "xenonet relay --since 7d\n");

// ---------- misc ----------
seed("misc", "rpc", 1, jsonl([{ id: 1, result: { ok: true } }, { id: 2, method: "status", params: {} }, { event: "progress", value: 0.5 }]));
seed("misc", "package", 1, JSON.stringify({ a: "1.2.3", b: "1.10.0", version: "1.2.3", platforms: { "darwin-arm64": { url: "https://example.com/x.tar.gz", sha256: "0".repeat(64), size: 10 } } }));
seed("misc", "read", 2, JSON.stringify({ request_id: "req_0123456789abcdef", operation: "sessions", arguments: { limit: 5, cursor: "abc" } }));
seed("misc", "tunnel", 3, "tunnel: 1234-abcd\ncredentials-file: /home/nova/.cloudflared/x.json\ningress:\n  - service: http://127.0.0.1:47840\n");
seed("misc", "tokens", 4, `Bearer xenonet_gateway_${"a".repeat(64)}\0S-1-5-21-1`);
seed("misc", "acl", 4, "O:BAG:SYD:PAI(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;S-1-5-21-2)\0S-1-5-21-1");
seed("misc", "theme", 5, JSON.stringify({ midnight: { extends: "phosphor", colors: { accent: "#ff00aa" } }, loop: { extends: "loop" }, over: { colors: { a: 1 } } }));
{
  // A small tar of a file and a folder, as the component tests make one.
  const header = (name, flag, size) => {
    const h = Buffer.alloc(512);
    h.write(name, 0);
    h.write("0000644\0", 100);
    h.write(size.toString(8).padStart(11, "0") + "\0", 124);
    h.write("14670000000\0", 136);
    h.write(flag, 156);
    h.write("ustar\0" + "00", 257);
    h.fill(" ", 148, 156);
    let sum = 0;
    for (const b of h) sum += b;
    h.write(sum.toString(8).padStart(6, "0") + "\0 ", 148);
    return h;
  };
  const body = Buffer.from("// tools\n");
  const pad = (b) => Buffer.concat([b, Buffer.alloc((512 - (b.length % 512)) % 512)]);
  const pax = Buffer.from("19 mtime=1791230000\n");
  seed("misc", "tar", 0, Buffer.concat([header("cli/", "5", 0), header("PaxHeader", "x", pax.length), pad(pax), header("cli/xenonet.js", "0", body.length), pad(body), Buffer.alloc(1024)]));
}

// ---------- jsregex ----------
seed("jsregex", "session", 0, "claude:0190d3e0-0000-7000-8000-000000000001");
seed("jsregex", "host", 2, "@vm1:project:/home/nova/projects/atlas-api");
seed("jsregex", "compile", 1, "^(?:@[a-z][a-z0-9-]{1,31}:)?(claude|codex):(?=.)[^\\s]+$", "\0", "@vm1:claude:x");
seed("jsregex", "classes", 1, "[\\w\\d\\s\\u00e9-\\u00ff]+?(?<!x)\\b", "\0", "café au lait");

fs.rmSync(tmp, { recursive: true, force: true });
process.stderr.write(`${written} seeds in ${path.relative(repo, out)}\n`);
