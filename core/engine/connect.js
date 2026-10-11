// Connecting your agents: what XenoNet adds to Claude Code's, Codex's and
// Grok Build's own settings so every session on this machine, in every
// project, can use the engine. Nothing changes until you confirm in XenoNet, which first shows
// exactly what will be added (preview), and it can all be taken out again
// (remove).
//
//   Claude Code  the xenonet MCP server at user scope (through `claude mcp`),
//                and HTTP hooks in ~/.claude/settings.json
//   Codex        the xenonet MCP server (through `codex mcp`), and command
//                hooks in ~/.codex/hooks.json, which Codex asks you to trust
//                once (/hooks)
//   Grok Build   the xenonet MCP server as a table in ~/.grok/config.toml,
//                written by XenoNet itself (`grok mcp add` rewrites the file
//                and drops its comments; see toml.js), and command hooks in
//                a file of XenoNet's own, ~/.grok/hooks/xenonet.json, which
//                Grok runs without asking, as it does all of ~/.grok/hooks.
//                GROK_HOME moves both.
//
// Both run the xenonet command (cli/xenonet.js: `xenonet mcp`, `xenonet hook <app>`)
// on the app's own runtime (Electron run as Node), so they work wherever
// XenoNet does, Node installed or not. XenoNet's hooks are told apart from yours
// by their URL or command, and a copy of each settings file is kept the
// first time XenoNet changes it (<file>.before-xenonet).
//
// Each XenoNet on this machine (the installed one, XenoNet Dev) is told apart
// by its port: connecting or disconnecting one changes its own entries and
// leaves another's alone, and its MCP servers have names of their own
// (xenonet for the installed XenoNet, xenonet-dev for XenoNet Dev). Entries whose
// files are gone, such as a deleted checkout's, are anyone's to clear.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { onPath, tilde, pathStyle } from "../paths.js";
import { parseJson } from "../vendor.js";
import { INSTANCES } from "../instances.js";
import { readServer, withServer, withoutServer, foreignServer } from "./toml.js";

// The names a XenoNet's MCP servers take: the installed XenoNet's xenonet and
// xenonet-console, another's with its instance (xenonet-dev).
export function serverNames(port) {
  const id = Object.entries(INSTANCES).find(([, v]) => v.port === port)?.[0] ?? String(port);
  const base = port === INSTANCES.installed.port ? "xenonet" : `xenonet-${id}`;
  return { xenonet: base, console: `${base}-console` };
}

const CLAUDE_EVENTS = [["SessionStart"], ["UserPromptSubmit"], ["PreToolUse", "Edit|Write|MultiEdit|NotebookEdit"], ["PostToolUse"], ["Stop"], ["SessionEnd"], ["Notification"]];
const CODEX_EVENTS = [["SessionStart"], ["UserPromptSubmit"], ["PreToolUse", "apply_patch|Edit|Write"], ["PostToolUse"], ["Stop"], ["SessionEnd"]];
// Grok Build's events, by its names. Its matchers name tools exactly
// (Claude Code's Edit and Write stand for its search_replace too), and its
// Notification's say which: a permission prompt, or a session gone idle.
// StopCancelled and StopFailure end a turn as Stop does, the ways Stop
// doesn't report.
const GROK_EVENTS = [
  ["SessionStart"],
  ["UserPromptSubmit"],
  ["PreToolUse", "Edit|Write|MultiEdit|search_replace|write_file|create_file|delete_file|apply_patch"],
  ["PostToolUse"],
  ["PostToolUseFailure"],
  ["Stop"],
  ["StopCancelled"],
  ["StopFailure"],
  ["Notification", "permission_prompt|idle_prompt"],
  ["SessionEnd"],
];
export const APPS = ["claude", "codex", "grok"];

const quote = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;

// A settings file, or {} when there's none. One that can't be read, or
// isn't JSON, is left alone, and the error names it with the home folder
// as ~ and says where it goes wrong without quoting it: it can hold tokens.
function readJson(file, home) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    if (err.code === "ENOENT") return {};
    throw new Error(`XenoNet can't read ${tilde(file, home)} (${err.code ?? tilde(err.message, home)}), so it leaves it alone`);
  }
  try {
    return parseJson(text);
  } catch (err) {
    throw new Error(`${tilde(file, home)} ${err.message}, so XenoNet leaves it alone`);
  }
}

function writeJson(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (fs.existsSync(file) && !fs.existsSync(`${file}.before-xenonet`)) fs.copyFileSync(file, `${file}.before-xenonet`);
  const tmp = `${file}.tmp-xenonet`;
  fs.writeFileSync(tmp, JSON.stringify(data, null, 2) + "\n");
  fs.renameSync(tmp, file);
}

// A settings object's "hooks" with XenoNet's taken out (`ours` tells them
// apart), and with `add` put back in when given.
export function mergeHooks(settings, ours, add = null) {
  const hooks = { ...(settings.hooks ?? {}) };
  for (const [event, groups] of Object.entries(hooks)) {
    const kept = (Array.isArray(groups) ? groups : [])
      .map((g) => ({ ...g, hooks: (g.hooks ?? []).filter((h) => !ours(h)) }))
      .filter((g) => g.hooks.length);
    if (kept.length) hooks[event] = kept;
    else delete hooks[event];
  }
  for (const [event, group] of Object.entries(add ?? {})) hooks[event] = [...(hooks[event] ?? []), group];
  const out = { ...settings, hooks };
  if (!Object.keys(hooks).length) delete out.hooks;
  return out;
}

// Whether every group in `add` is already in place.
const hooksInPlace = (settings, ours, add) => Object.entries(add).every(([event, group]) => (settings.hooks?.[event] ?? []).some((g) => (g.matcher ?? null) === (group.matcher ?? null) && g.hooks?.some((h) => ours(h) && JSON.stringify(h) === JSON.stringify(group.hooks[0]))));

export function createConnect({ port, root, demo = false, exec = process.execPath, runAsNode = false, home = os.homedir(), env = process.env, platform = process.platform, run = runFile }) {
  const entry = pathStyle(root).join(root, "cli", "xenonet.js");
  const serverEnv = { XENONET_PORT: String(port), ...(runAsNode ? { ELECTRON_RUN_AS_NODE: "1" } : {}) };
  const consoleMcp = !!env.XENONET_LAUNCHER_EXEC;
  const mcp = { type: "stdio", command: exec, args: [entry, "mcp"], env: serverEnv };
  const hookUrl = `http://127.0.0.1:${port}/api/hook/claude`;
  const claudeSettings = path.join(env.CLAUDE_CONFIG_DIR ?? path.join(home, ".claude"), "settings.json");
  // Claude Code keeps user-scope MCP servers in .claude.json, in
  // CLAUDE_CONFIG_DIR when that's set.
  const claudeState = env.CLAUDE_CONFIG_DIR ? path.join(env.CLAUDE_CONFIG_DIR, ".claude.json") : path.join(home, ".claude.json");
  const codexHooks = path.join(env.CODEX_HOME ?? path.join(home, ".codex"), "hooks.json");
  const grokHome = env.GROK_HOME ?? path.join(home, ".grok");
  const grokConfig = path.join(grokHome, "config.toml");
  const grokHooks = path.join(grokHome, "hooks", "xenonet.json");
  const hookCommand = (app) => `${Object.entries(serverEnv).map(([k, v]) => `${k}=${v}`).join(" ")} ${quote(exec)} ${quote(entry)} hook ${app}`;
  const psQuote = (s) => `'${String(s).replace(/'/g, "''")}'`;
  const windowsHook = (app) => {
    const script = `# XenoNet hook ${app}\n${Object.entries(serverEnv).map(([k, v]) => `$env:${k}=${psQuote(v)}`).join("; ")}; & ${psQuote(exec)} ${psQuote(entry)} hook ${psQuote(app)}; exit $LASTEXITCODE`;
    return `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ${Buffer.from(script, "utf16le").toString("base64")}`;
  };
  const hookText = (h) => {
    const command = h.commandWindows ?? h.command ?? "";
    const encoded = /-EncodedCommand ([A-Za-z0-9+/=]+)$/.exec(command)?.[1];
    return encoded ? Buffer.from(encoded, "base64").toString("utf16le") : command;
  };

  // Any XenoNet's entries, whichever XenoNet (port) or checkout added them, in
  // this form or the hook.js of earlier versions.
  const xenonetCommand = (app) => (h) => h?.type === "command" && (new RegExp(`(cli[\\\\/]xenonet\\.js['"]? hook|(cli|server[\\\\/]engine)[\\\\/]hook\\.js['"]?) ${app}$`).test(h.command ?? "") || (hookText(h).startsWith(`# XenoNet hook ${app}\n`) && hookText(h).endsWith(` hook '${app}'; exit $LASTEXITCODE`)));
  const xenonetClaudeHook = (h) => (h?.type === "http" && /^http:\/\/(127\.0\.0\.1|localhost):\d+\/api\/hook\/claude$/.test(h.url ?? "")) || xenonetCommand("claude")(h);
  // Which XenoNet an entry is for: its port, from its URL or XENONET_PORT; null
  // for an early form that didn't say.
  const portOf = (h) => {
    const m = h?.type === "http" ? /:(\d+)\/api\/hook\//.exec(h.url ?? "") : /XENONET_PORT=['"]?(\d+)/.exec(hookText(h));
    return m ? Number(m[1]) : null;
  };
  // An entry is this XenoNet's when it's for this port, or says no port, or
  // runs files that are gone; another live XenoNet's stays as it is.
  const mine = (isXenoNet) => (h) => isXenoNet(h) && (portOf(h) === port || portOf(h) == null || hookFiles({ x: [{ hooks: [h] }] }, isXenoNet).some((f) => !fs.existsSync(f)));
  const ourClaudeHook = mine(xenonetClaudeHook);
  const ourCodexHook = mine(xenonetCommand("codex"));
  const ourGrokHook = mine(xenonetCommand("grok"));
  // Other XenoNets connected here too, by port, for the preview.
  const others = (hooks, isXenoNet) => [...new Set(Object.values(hooks ?? {}).flat().flatMap((g) => g.hooks ?? []).filter((h) => isXenoNet(h) && !mine(isXenoNet)(h)).map(portOf))].sort();
  const names = serverNames(port);
  // Claude Code takes HTTP hooks, which cost nothing to run, except at
  // SessionStart, which takes only commands: there it runs hook.js.
  const claudeHook = (event) => (event === "SessionStart" ? { type: "command", command: platform === "win32" ? windowsHook("claude") : hookCommand("claude"), ...(platform === "win32" ? { shell: "powershell" } : {}), timeout: 10 } : { type: "http", url: hookUrl, timeout: 5 });
  const claudeHooks = Object.fromEntries(CLAUDE_EVENTS.map(([event, matcher]) => [event, { ...(matcher ? { matcher } : {}), hooks: [claudeHook(event)] }]));
  // Codex allows SessionEnd hooks 3 seconds at most, and says so on every
  // run if asked for more.
  const codexHookSet = Object.fromEntries(CODEX_EVENTS.map(([event, matcher]) => [event, { ...(matcher ? { matcher } : {}), hooks: [{ type: "command", command: hookCommand("codex"), ...(platform === "win32" ? { commandWindows: windowsHook("codex") } : {}), timeout: event === "SessionEnd" ? 3 : 10 }] }]));
  // Grok runs a hook's command with sh -c, and on Windows under the shell it
  // finds there (PowerShell, or Git Bash), so the Windows form is one
  // command line; it reads no field for another shell. Its SessionEnd hooks
  // get a second and a half unless they say.
  const grokHookSet = Object.fromEntries(GROK_EVENTS.map(([event, matcher]) => [event, { ...(matcher ? { matcher } : {}), hooks: [{ type: "command", command: platform === "win32" ? windowsHook("grok") : hookCommand("grok"), timeout: event === "SessionEnd" ? 3 : 10 }] }]));
  // The xenonet MCP server as Grok has it. Grok puts a session's id in place
  // of {{session_id}} in a remote server's headers; if it does in a local
  // server's environment too, the server knows its session without asking
  // (mcp/xenonet.js), and if it doesn't, the server asks XenoNet, which
  // knows from Grok's hooks which session each grok process holds.
  const grokServer = (args = mcp.args) => ({ command: mcp.command, args, env: { ...serverEnv, XENONET_GROK_SESSION: "{{session_id}}" } });

  // A CLI on PATH (XenoNet's PATH is your login shell's).
  const cli = (name) => onPath(name, { env, platform, canRun: (f) => fs.accessSync(f) });
  // Grok Build: grok on PATH, or where its installer puts it, which a
  // shell's PATH may not have yet.
  const grokCli = () => cli("grok") ?? [path.join(grokHome, "bin", platform === "win32" ? "grok.exe" : "grok")].find((f) => fs.existsSync(f)) ?? null;

  // Files a connected entry runs that no longer exist (a moved or deleted
  // checkout, an uninstalled app).
  const gone = (command, args = []) => [command, ...args.filter((a) => /\.m?js$/.test(a))].filter((f) => f && path.isAbsolute(f) && !fs.existsSync(f));
  // Only the paths among its quoted words: a Windows hook quotes its port
  // and app too ('47840', 'claude'), which are no files to look for.
  const hookFiles = (hooks, ours) => Object.values(hooks ?? {}).flat().flatMap((g) => g.hooks ?? []).filter((h) => ours(h) && h.type === "command").flatMap((h) => [...hookText(h).matchAll(/'((?:[^']|'')+)'/g)].map((m) => m[1].replace(/''/g, "'")).filter((f) => path.isAbsolute(f) || path.win32.isAbsolute(f)));

  // The xenonet server as each app has it: missing, present (as XenoNet would
  // add it) or different.
  const serverState = (s, args = mcp.args) => (!s ? "missing" : s.command === mcp.command && JSON.stringify(s.args) === JSON.stringify(args) && s.env?.XENONET_PORT === mcp.env.XENONET_PORT ? "present" : "different");
  async function codexEntry(bin, name = names.xenonet) {
    const r = bin ? await run(bin, ["mcp", "get", name, "--json"]).catch(() => null) : null;
    if (!r) return null;
    try {
      const s = JSON.parse(r);
      return s.transport ?? s;
    } catch {
      return { command: null };
    }
  }

  // Grok's config.toml as it is, or "" when there's none. One that can't
  // be read is left alone, as a settings file is.
  const grokText = () => {
    try {
      return fs.readFileSync(grokConfig, "utf8");
    } catch (err) {
      if (err.code === "ENOENT") return "";
      throw new Error(`XenoNet can't read ${tilde(grokConfig, home)} (${err.code ?? tilde(err.message, home)}), so it leaves it alone`);
    }
  };
  // A server in Grok's config: missing, present (as XenoNet would write
  // it) or different; Grok's {{session_id}} for this server's own.
  const grokServerState = (text, name, args = mcp.args) => {
    const s = readServer(text, name);
    if (!s) return foreignServer(text, name) ? "different" : "missing";
    const want = grokServer(args);
    return s.command === want.command && JSON.stringify(s.args) === JSON.stringify(want.args) && s.env?.XENONET_PORT === want.env.XENONET_PORT && s.env?.XENONET_GROK_SESSION === want.env.XENONET_GROK_SESSION ? "present" : "different";
  };

  async function preview() {
    const claude = cli("claude"), codex = cli("codex"), grok = grokCli();
    const claudeEntry = readJson(claudeState, home).mcpServers?.[names.xenonet];
    const codexServer = await codexEntry(codex);
    const consoleServers = consoleMcp ? { claude: readJson(claudeState, home).mcpServers?.[names.console], codex: await codexEntry(codex, names.console) } : {};
    const consoleChange = (app) => consoleMcp ? [{ id: "console", what: `The ${names.console} MCP server, to operate Desktop`, where: `${app} user settings`, state: serverState(consoleServers[app], [...mcp.args, "console"]), detail: `${command} mcp console (XENONET_PORT=${port})` }] : [];
    // Another XenoNet on this machine connected too (XenoNet Dev beside the
    // installed one): its entries stay as they are.
    const besides = (list) => list.map((p) => `Another XenoNet (port ${p}${Object.values(INSTANCES).find((i) => i.port === p) ? `, ${Object.values(INSTANCES).find((i) => i.port === p).name}` : ""}) is connected here too; its entries stay as they are.`);
    const broken = (entry, hooks, ours) => [...(entry ? gone(entry.command, entry.args) : []), ...hookFiles(hooks, ours).filter((f) => !fs.existsSync(f))].map((f) => `${tilde(f, home)} is gone`);
    const command = `${tilde(exec, home)} ${tilde(entry, home)}`;
    return {
      port,
      demo,
      apps: [
        {
          app: "claude",
          name: "Claude Code",
          cli: claude,
          changes: [
            ...consoleChange("claude"),
            { id: "mcp", what: `The ${names.xenonet} MCP server, for every project`, where: "Claude Code's user settings, through claude mcp add-json --scope user", state: serverState(claudeEntry), detail: `${command} mcp (XENONET_PORT=${port})` },
            { id: "hooks", what: `Hooks: ${CLAUDE_EVENTS.map(([e, m]) => (m ? `${e} (edits)` : e)).join(", ")}`, where: tilde(claudeSettings, home), state: hooksInPlace(readJson(claudeSettings, home), ourClaudeHook, claudeHooks) ? "present" : "missing", detail: `posting to ${hookUrl}; SessionStart runs ${command} hook claude` },
          ],
          notes: [...(claude ? [] : ["XenoNet can't find the claude command, so it can add the hooks but not the MCP server. Install Claude Code's CLI, or add the server by hand."]), ...besides(others(readJson(claudeSettings, home).hooks, xenonetClaudeHook))],
          broken: broken(claudeEntry, readJson(claudeSettings, home).hooks, ourClaudeHook),
        },
        {
          app: "codex",
          name: "Codex",
          cli: codex,
          changes: [
            ...consoleChange("codex"),
            { id: "mcp", what: `The ${names.xenonet} MCP server, for every project`, where: "Codex's config, through codex mcp add", state: serverState(codexServer), detail: `${command} mcp (XENONET_PORT=${port})` },
            { id: "hooks", what: `Hooks: ${CODEX_EVENTS.map(([e, m]) => (m ? `${e} (edits)` : e)).join(", ")}`, where: tilde(codexHooks, home), state: hooksInPlace(readJson(codexHooks, home), ourCodexHook, codexHookSet) ? "present" : "missing", detail: `running ${command} hook codex` },
          ],
          notes: [
            ...(codex ? [] : ["XenoNet can't find the codex command, so it can add the hooks but not the MCP server."]),
            "Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's.",
            ...besides(others(readJson(codexHooks, home).hooks, xenonetCommand("codex"))),
          ],
          broken: broken(codexServer, readJson(codexHooks, home).hooks, ourCodexHook),
        },
        grokApp(grok, command, consoleMcp, besides, broken),
      ],
    };
  }

  // Grok Build's part of the preview. `installed`: whether Grok Build is
  // here, so connecting everything takes it in only then.
  function grokApp(grok, command, withConsole, besides, broken) {
    const text = grokText();
    const hooks = readJson(grokHooks, home);
    const entry = readServer(text, names.xenonet);
    return {
      app: "grok",
      name: "Grok Build",
      cli: grok,
      installed: !!grok || fs.existsSync(grokConfig),
      changes: [
        ...(withConsole ? [{ id: "console", what: `The ${names.console} MCP server, to operate Desktop`, where: tilde(grokConfig, home), state: grokServerState(text, names.console, [...mcp.args, "console"]), detail: `${command} mcp console (XENONET_PORT=${port})` }] : []),
        { id: "mcp", what: `The ${names.xenonet} MCP server, for every project`, where: tilde(grokConfig, home), state: grokServerState(text, names.xenonet), detail: `${command} mcp (XENONET_PORT=${port}), as [mcp_servers.${names.xenonet}]; the rest of the file stays as it is` },
        { id: "hooks", what: `Hooks: ${GROK_EVENTS.map(([e, m]) => (m && e === "PreToolUse" ? `${e} (edits)` : e)).join(", ")}`, where: tilde(grokHooks, home), state: hooksInPlace(hooks, ourGrokHook, grokHookSet) ? "present" : "missing", detail: `running ${command} hook grok` },
      ],
      notes: [
        ...(grok ? [] : ["XenoNet can't find Grok Build (grok on your PATH, or in ~/.grok/bin). Connecting it writes its settings all the same, for when you install it."]),
        ...(foreignServer(text, names.xenonet) ? [`${tilde(grokConfig, home)} names mcp_servers.${names.xenonet} in a form XenoNet doesn't edit. Take it out, or write it as a [mcp_servers.${names.xenonet}] table, and connect again.`] : []),
        "Grok Build runs the hooks in ~/.grok/hooks without asking. It reads Claude Code's settings too, and XenoNet's Claude Code hooks pass over its sessions.",
        ...besides(others(hooks.hooks, xenonetCommand("grok"))),
      ],
      broken: broken(entry ? { command: entry.command, args: entry.args ?? [] } : null, hooks.hooks, ourGrokHook),
    };
  }

  // Writes Grok's config.toml: the same text with `edit` made, through a
  // temporary file, keeping its mode, and a copy of it as it was the first
  // time XenoNet changes it.
  function writeGrokConfig(edit) {
    const before = grokText();
    const after = edit(before);
    if (after === before) return;
    fs.mkdirSync(path.dirname(grokConfig), { recursive: true });
    let mode = 0o600;
    try {
      mode = fs.statSync(grokConfig).mode & 0o777;
      if (!fs.existsSync(`${grokConfig}.before-xenonet`)) fs.copyFileSync(grokConfig, `${grokConfig}.before-xenonet`);
    } catch {}
    const tmp = `${grokConfig}.tmp-xenonet`;
    fs.writeFileSync(tmp, after, { mode });
    fs.renameSync(tmp, grokConfig);
  }

  // XenoNet's hook file for Grok: written as the settings files are, and
  // taken out once no XenoNet's hooks are left in it.
  function writeGrokHooks(add) {
    const merged = mergeHooks(readJson(grokHooks, home), ourGrokHook, add);
    if (!add && !Object.keys(merged).length) {
      fs.rmSync(grokHooks, { force: true });
      return;
    }
    writeJson(grokHooks, merged);
  }

  async function apply(apps = ["claude", "codex"]) {
    // Agents connected to the demo would see only its made-up sessions.
    if (demo) throw new Error("this is the demo, so connect Claude Code, Codex and Grok Build from XenoNet, with your own sessions");
    if (apps.includes("claude")) {
      writeJson(claudeSettings, mergeHooks(readJson(claudeSettings, home), ourClaudeHook, claudeHooks));
      const bin = cli("claude");
      if (bin) {
        await run(bin, ["mcp", "remove", names.xenonet, "--scope", "user"]).catch(() => {});
        await run(bin, ["mcp", "add-json", names.xenonet, JSON.stringify(mcp), "--scope", "user"]);
        if (consoleMcp) {
          await run(bin, ["mcp", "remove", names.console, "--scope", "user"]).catch(() => {});
          await run(bin, ["mcp", "add-json", names.console, JSON.stringify({ ...mcp, args: [...mcp.args, "console"] }), "--scope", "user"]);
        }
      }
    }
    if (apps.includes("codex")) {
      writeJson(codexHooks, mergeHooks(readJson(codexHooks, home), ourCodexHook, codexHookSet));
      const bin = cli("codex");
      if (bin) {
        await run(bin, ["mcp", "remove", names.xenonet]).catch(() => {});
        await run(bin, ["mcp", "add", names.xenonet, ...Object.entries(serverEnv).flatMap(([k, v]) => ["--env", `${k}=${v}`]), "--", mcp.command, ...mcp.args]);
        if (consoleMcp) {
          await run(bin, ["mcp", "remove", names.console]).catch(() => {});
          await run(bin, ["mcp", "add", names.console, ...Object.entries(serverEnv).flatMap(([k, v]) => ["--env", `${k}=${v}`]), "--", mcp.command, ...mcp.args, "console"]);
        }
      }
    }
    if (apps.includes("grok")) {
      writeGrokConfig((text) => {
        let out = withServer(text, names.xenonet, grokServer());
        if (consoleMcp) out = withServer(out, names.console, grokServer([...mcp.args, "console"]));
        return out;
      });
      writeGrokHooks(grokHookSet);
    }
    return preview();
  }

  async function remove() {
    writeJson(claudeSettings, mergeHooks(readJson(claudeSettings, home), ourClaudeHook));
    if (fs.existsSync(codexHooks)) writeJson(codexHooks, mergeHooks(readJson(codexHooks, home), ourCodexHook));
    if (fs.existsSync(grokHooks)) writeGrokHooks(null);
    // Only this XenoNet's servers: another's, by their names, stay.
    if (fs.existsSync(grokConfig)) writeGrokConfig((text) => withoutServer(consoleMcp ? withoutServer(text, names.console) : text, names.xenonet));
    const claude = cli("claude"), codex = cli("codex");
    if (claude) await run(claude, ["mcp", "remove", names.xenonet, "--scope", "user"]).catch(() => {});
    if (codex) await run(codex, ["mcp", "remove", names.xenonet]).catch(() => {});
    if (consoleMcp) {
      if (claude) await run(claude, ["mcp", "remove", names.console, "--scope", "user"]).catch(() => {});
      if (codex) await run(codex, ["mcp", "remove", names.console]).catch(() => {});
    }
    return preview();
  }

  return { preview, apply, remove };
}

function runFile(file, args) {
  return new Promise((resolve, reject) =>
    execFile(file, args, { timeout: 20000, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) => (err ? reject(new Error(stderr.trim() || err.message)) : resolve(stdout))),
  );
}
