// xenonet-launcher 3 (protocol 1)
//
// XenoNet's stable command launcher, at <state>/runtime/launcher/cli/xenonet.js
// beside a node of its own. Claude Code's, Codex's and Grok Build's hooks
// and MCP servers, the xenonet command on PATH, other programs' command lines and
// OpenAI's tunnel-client all run this path, so it never moves, and what
// they run doesn't change when XenoNet's components do
// (docs/design/contracts.md, section 12). XenoNet writes it again only when
// it ships a newer one (the version above), atomically.
//
//   xenonet hook <app>   passes a hook event to the running XenoNet, itself,
//                      without starting the Tools component: quick, and it
//                      always exits 0 and prints nothing when XenoNet isn't
//                      there, so a hook never fails an agent's turn
//   xenonet --launcher   what this launcher is, as JSON: its version and
//                      protocol, and the Tools version it runs
//   anything else      runs the selected Tools version's cli/xenonet.js
//
// CommonJS, and only Node's own modules, so it runs on any node the
// launcher has.
"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");

const LAUNCHER = 3;
const PROTOCOL = 1;
const dir = path.resolve(__dirname, "../..");
const args = process.argv.slice(2);

function tools() {
  try {
    const p = JSON.parse(fs.readFileSync(path.join(dir, "components/tools/current.json"), "utf8"));
    if (!/^\d+\.\d+\.\d+$/.test(p.version) || !/^[a-f0-9]{64}$/.test(p.sha256)) return null;
    const root = path.join(dir, "components/tools", `${p.version}-${p.sha256.slice(0, 16)}`);
    return fs.existsSync(path.join(root, "cli/xenonet.js")) ? { version: p.version, root } : null;
  } catch {
    return null;
  }
}

// A hook: the event on stdin goes to XenoNet's /api/hook/<app>, and what
// XenoNet answers goes to stdout, the hook output format the apps read.
// Grok Build runs Claude Code's hooks too, and says so in GROK_HOOK_EVENT:
// the Claude Code one stays quiet there, since Grok's own speaks for the
// session. Grok's own gets its session and event from the environment when
// the event doesn't say them (cli/commands/hook.js does the same).
async function hook(app) {
  app = app === "claude" || app === "grok" ? app : "codex";
  const flag = args.indexOf("--port");
  const port = Number(flag >= 0 ? args[flag + 1] : process.env.XENONET_PORT) || 47840;
  let input = "";
  let out = "";
  try {
    process.stdin.setEncoding("utf8");
    for await (const chunk of process.stdin) input += chunk;
    if (app === "claude" && process.env.GROK_HOOK_EVENT) return process.exit(0);
    if (app === "grok") input = withGrokEnv(input);
    const res = await fetch(`http://127.0.0.1:${port}/api/hook/${app}`, { method: "POST", headers: { "content-type": "application/json" }, body: input || "{}", signal: AbortSignal.timeout(4000) });
    if (res.status === 200) out = await res.text();
  } catch {}
  process.stdout.write(out, () => process.exit(0));
}

function withGrokEnv(input) {
  let body;
  try {
    body = JSON.parse(input || "{}");
  } catch {
    return input;
  }
  if (!body || typeof body !== "object" || Array.isArray(body)) return input;
  if (!body.sessionId && process.env.GROK_SESSION_ID) body.sessionId = process.env.GROK_SESSION_ID;
  if (!body.hookEventName && process.env.GROK_HOOK_EVENT) body.hookEventName = process.env.GROK_HOOK_EVENT;
  return JSON.stringify(body);
}

if (args[0] === "hook" && typeof fetch === "function") {
  hook(args[1]);
} else if (args[0] === "--launcher") {
  const t = tools();
  process.stdout.write(`${JSON.stringify({ launcher: LAUNCHER, protocol: PROTOCOL, tools: t ? t.version : null, node: process.version })}\n`);
} else {
  const t = tools();
  if (!t) {
    // A hook on a node too old for fetch still never fails its agent.
    if (args[0] === "hook") process.exit(0);
    process.stderr.write("XenoNet's Tools aren't installed here, or their files are missing. Open XenoNet to repair them, or install XenoNet again.\n");
    process.exit(1);
  }
  const env = { ...process.env, XENONET_RUNTIME_DIR: dir, XENONET_LAUNCHER_EXEC: process.execPath, XENONET_LAUNCHER_ROOT: path.resolve(__dirname, "..") };
  delete env.ELECTRON_RUN_AS_NODE;
  const child = spawn(path.join(t.root, process.platform === "win32" ? "node.exe" : "node"), [path.join(t.root, "cli/xenonet.js"), ...args], { env, stdio: "inherit", windowsHide: true });
  child.on("error", (e) => {
    process.stderr.write(`XenoNet couldn't start its Tools: ${e.message}\n`);
    process.exit(1);
  });
  child.on("exit", (code) => process.exit(code ?? 1));
  for (const signal of ["SIGTERM", "SIGINT"]) process.on(signal, () => child.kill(signal));
}
