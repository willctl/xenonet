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
//   Grok Build   the xenonet MCP server as tables in ~/.grok/config.toml,
//                which XenoNet writes itself, and command hooks in a file of
//                XenoNet's own, ~/.grok/hooks/xenonet.json
//
// Both run the xenonet binary (`xenonet mcp`, `xenonet hook <app>`): installed
// XenoNet's launcher, which never moves (components/runtime.js's xenonetBin).
// Entries an earlier XenoNet wrote run Node and cli/xenonet.js instead;
// they're still its own, so connecting again replaces them and disconnecting
// takes them out. XenoNet's hooks are told apart from yours by their URL or
// command, and a copy of each settings file is kept the first time XenoNet
// changes it (<file>.before-xenonet).
//
// Each XenoNet on this machine (the installed one, XenoNet Dev) is told apart
// by its port: connecting or disconnecting one changes its own entries and
// leaves another's alone, and its MCP servers have names of their own
// (xenonet for the installed XenoNet, xenonet-dev for XenoNet Dev). Entries whose
// files are gone, such as a deleted checkout's, are anyone's to clear.
//
// XenoNet's plugin for Claude Code (plugins/xenonet) carries the same server
// and hooks for the installed XenoNet, so they're kept to one: while the
// plugin is installed and turned on, connecting leaves Claude Code to it and
// takes connect's own copies out; and while connect's own hook for an event
// is in Claude Code's settings, the server ignores the plugin's copy of it
// (hookConnected), so an event never arrives twice.
//
// Which apps connect goes through are the app registry's: those that have
// `connect`, in its order, each through the config format its entry names
// (FORMATS).
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { onPath, tilde } from "../paths.js";
import { parseJson } from "../vendor.js";
import { INSTANCES } from "../instances.js";
import { xenonetBin } from "../components/runtime.js";
import { APPS } from "../apps.js";
import { readServer, withServer, withoutServer, foreignServer } from "./toml.js";

// The names a XenoNet's MCP servers take: the installed XenoNet's xenonet and
// xenonet-console, another's with its instance (xenonet-dev).
export function serverNames(port) {
  const id = Object.entries(INSTANCES).find(([, v]) => v.port === port)?.[0] ?? String(port);
  const base = port === INSTANCES.installed.port ? "xenonet" : `xenonet-${id}`;
  return { xenonet: base, console: `${base}-console` };
}

// XenoNet's plugin for Claude Code, which Claude Code knows as
// xenonet@<marketplace> once it's installed.
const PLUGIN = "xenonet";

// What the preview says while connect's own entries are in Claude Code's
// settings beside the plugin's.
const TWICE = "xenonet connect's own entries are in Claude Code's settings beside the plugin's. XenoNet ignores the plugin's copy of any hook connect added, so each event still arrives once. Connecting takes connect's entries out.";
// What the preview says when the plugin can't find the command it runs.
const NO_COMMAND = "XenoNet's plugin runs the xenonet command, which isn't on your PATH";
// What the preview says of entries an earlier XenoNet wrote.
const EARLIER = "XenoNet's entries here run cli/xenonet.js with Node, as an earlier XenoNet wrote them. Connecting again has them run the xenonet binary.";

const quote = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
// Names as a sentence lists them: "A, B and C".
const andList = (names) => (names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names.join(""));
const psQuote = (s) => `'${String(s).replace(/'/g, "''")}'`;

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

// Claude Code's settings (claude-settings): the xenonet MCP server at user
// scope, through `claude mcp`, kept in .claude.json; HTTP hooks in
// settings.json; and XenoNet's plugin for Claude Code, which carries both
// for the installed XenoNet.
function claudeSettings(app, k) {
  const { env, home, port, platform, names, mcp, run } = k;
  const dir = env.CLAUDE_CONFIG_DIR ?? path.join(home, ".claude");
  const settingsFile = path.join(dir, "settings.json");
  // Claude Code keeps user-scope MCP servers in .claude.json, in
  // CLAUDE_CONFIG_DIR when that's set.
  const stateFile = env.CLAUDE_CONFIG_DIR ? path.join(env.CLAUDE_CONFIG_DIR, ".claude.json") : path.join(home, ".claude.json");
  // Claude Code's list of the plugins it has installed.
  const pluginsFile = path.join(dir, "plugins", "installed_plugins.json");
  // Whether XenoNet's plugin, which reaches the installed XenoNet, is for
  // this XenoNet.
  const pluginPort = port === INSTANCES.installed.port;
  const hookUrl = `http://127.0.0.1:${port}/api/hook/${app.id}`;
  const isXenoNet = (h) => (h?.type === "http" && new RegExp(`^http://(127\\.0\\.0\\.1|localhost):\\d+/api/hook/${app.id}$`).test(h.url ?? "")) || k.xenonetCommand(app.id)(h);
  const ours = k.mine(isXenoNet);
  // Claude Code takes HTTP hooks, which cost nothing to run, except at
  // SessionStart, which takes only commands: there it runs hook.js.
  const hookSet = k.hookSet(app, (event) => (event === "SessionStart" ? { type: "command", command: platform === "win32" ? k.windowsHook(app.id) : k.hookCommand(app.id), ...(platform === "win32" ? { shell: "powershell" } : {}), timeout: 10 } : { type: "http", url: hookUrl, timeout: 5 }));

  // XenoNet's plugin for Claude Code, as Claude Code knows it
  // (xenonet@<marketplace>), when it's installed and turned on in Claude
  // Code's user settings. The plugin reaches the installed XenoNet, so only
  // that XenoNet counts it. A list of plugins that can't be read counts as
  // none.
  const pluginKey = (settings) => {
    if (!pluginPort) return null;
    const enabled = settings?.enabledPlugins;
    if (!enabled || typeof enabled !== "object" || Array.isArray(enabled)) return null;
    let installed;
    try {
      installed = readJson(pluginsFile, home).plugins;
    } catch {
      return null;
    }
    if (!installed || typeof installed !== "object" || Array.isArray(installed)) return null;
    return Object.entries(enabled).find(([key, on]) => key.includes("@") && key.slice(0, key.indexOf("@")) === PLUGIN && on === true && Array.isArray(installed[key]) && installed[key].length > 0)?.[0] ?? null;
  };

  return {
    cli: "claude",
    server: async (_cli, name) => readJson(stateFile, home).mcpServers?.[name],

    async entry({ cli, server, console }) {
      const settings = readJson(settingsFile, home);
      const plugin = pluginKey(settings);
      const mcpWhat = `The ${names.xenonet} MCP server, for every project`, mcpDetail = `${k.command} mcp (XENONET_PORT=${port})`;
      const hooksWhat = `Hooks: ${k.eventList(app)}`, hooksDetail = `posting to ${hookUrl}; SessionStart runs ${k.command} hook ${app.id}`;
      const ownHooks = k.ownHooksOf(settings.hooks, isXenoNet).length > 0;
      let twice = false;
      const changes = [...k.consoleChange(app.id, console)];
      if (plugin) {
        // The plugin carries the server and the hooks; connect's own copies
        // beside it are to come out.
        const carried = `XenoNet's plugin for Claude Code (${plugin})`;
        if (k.serverState(server) === "missing") changes.push({ id: "mcp", what: mcpWhat, where: carried, state: "present", via: "plugin", detail: `xenonet mcp, from your PATH (XENONET_PORT=${port})` });
        else {
          twice = true;
          changes.push({ id: "mcp", what: mcpWhat, where: "Claude Code's user settings, beside the plugin's copy", state: "remove", detail: mcpDetail });
        }
        if (ownHooks) {
          twice = true;
          changes.push({ id: "hooks", what: hooksWhat, where: `${tilde(settingsFile, home)}, beside the plugin's copies`, state: "remove", detail: hooksDetail });
        } else changes.push({ id: "hooks", what: hooksWhat, where: carried, state: "present", via: "plugin", detail: `posting to ${hookUrl}?via=plugin; SessionStart runs xenonet hook ${app.id}` });
      } else {
        changes.push({ id: "mcp", what: mcpWhat, where: "Claude Code's user settings, through claude mcp add-json --scope user", state: k.serverState(server), detail: mcpDetail });
        changes.push({ id: "hooks", what: hooksWhat, where: tilde(settingsFile, home), state: k.hooksState(settings, ours, hookSet, isXenoNet), detail: hooksDetail });
      }
      return {
        app: app.id,
        name: app.name,
        cli,
        changes,
        notes: [
          // With the plugin, connecting adds nothing through the claude
          // command, so whether it's there doesn't matter.
          ...(cli || plugin ? [] : ["XenoNet can't find the claude command, so it can add the hooks but not the MCP server. Install Claude Code's CLI, or add the server by hand."]),
          ...(plugin ? [`Claude Code has XenoNet's plugin (${plugin}), which carries the ${names.xenonet} MCP server and the hooks, so connecting leaves them to it.`] : []),
          ...(twice ? [TWICE] : []),
          ...(k.earlier(settings.hooks, isXenoNet, server) ? [EARLIER] : []),
          ...k.besides(k.others(settings.hooks, isXenoNet)),
        ],
        broken: [...k.broken(server, settings.hooks, ours), ...(plugin && !k.cli("xenonet") ? [NO_COMMAND] : [])],
      };
    },

    async apply() {
      const settings = readJson(settingsFile, home);
      // With XenoNet's plugin there, connect's own copies of the server and
      // the hooks come out, and none go in.
      const plugin = !!pluginKey(settings);
      if (!plugin) writeJson(settingsFile, mergeHooks(settings, ours, hookSet));
      else if (k.ownHooksOf(settings.hooks, isXenoNet).length) writeJson(settingsFile, mergeHooks(settings, ours));
      const bin = k.cli("claude");
      if (bin) {
        await run(bin, ["mcp", "remove", names.xenonet, "--scope", "user"]).catch(() => {});
        if (!plugin) await run(bin, ["mcp", "add-json", names.xenonet, JSON.stringify(mcp), "--scope", "user"]);
        if (k.consoleMcp) {
          await run(bin, ["mcp", "remove", names.console, "--scope", "user"]).catch(() => {});
          await run(bin, ["mcp", "add-json", names.console, JSON.stringify({ ...mcp, args: [...mcp.args, "console"] }), "--scope", "user"]);
        }
      }
    },

    async removeHooks() {
      writeJson(settingsFile, mergeHooks(readJson(settingsFile, home), ours));
    },

    async removeServer(cli, name) {
      if (cli) await run(cli, ["mcp", "remove", name, "--scope", "user"]).catch(() => {});
    },

    // Whether this XenoNet's own Claude Code hook for `event`, as connect
    // adds it, is in Claude Code's settings and can run (a command's files
    // are all there). While it is, the copy XenoNet's plugin posts stands
    // by, so the event reaches XenoNet once. Settings that can't be read
    // hold no hook Claude Code runs.
    async hookConnected(event) {
      let settings;
      try {
        settings = readJson(settingsFile, home);
      } catch {
        return false;
      }
      const groups = settings.hooks?.[event];
      if (!Array.isArray(groups)) return false;
      return groups.flatMap((g) => g?.hooks ?? []).some((h) => isXenoNet(h) && (k.portOf(h) === port || k.portOf(h) == null) && k.hookFiles({ x: [{ hooks: [h] }] }, () => true).every((f) => fs.existsSync(f)));
    },
  };
}

// Codex's hooks (codex-hooks): the xenonet MCP server through `codex mcp`,
// and command hooks in hooks.json, which Codex asks you to trust once.
function codexHooks(app, k) {
  const { env, home, port, platform, names, mcp, serverEnv, run } = k;
  const hooksFile = path.join(env.CODEX_HOME ?? path.join(home, ".codex"), "hooks.json");
  const isXenoNet = k.xenonetCommand(app.id);
  const ours = k.mine(isXenoNet);
  // Codex allows SessionEnd hooks 3 seconds at most, and says so on every
  // run if asked for more.
  const hookSet = k.hookSet(app, (event) => ({ type: "command", command: k.hookCommand(app.id), ...(platform === "win32" ? { commandWindows: k.windowsHook(app.id) } : {}), timeout: event === "SessionEnd" ? 3 : 10 }));
  const add = (bin, name, extra = []) => run(bin, ["mcp", "add", name, ...Object.entries(serverEnv).flatMap(([key, v]) => ["--env", `${key}=${v}`]), "--", mcp.command, ...mcp.args, ...extra]);

  return {
    cli: "codex",
    async server(cli, name) {
      const r = cli ? await run(cli, ["mcp", "get", name, "--json"]).catch(() => null) : null;
      if (!r) return null;
      try {
        const s = JSON.parse(r);
        return s.transport ?? s;
      } catch {
        return { command: null };
      }
    },

    async entry({ cli, server, console }) {
      return {
        app: app.id,
        name: app.name,
        cli,
        changes: [
          ...k.consoleChange(app.id, console),
          { id: "mcp", what: `The ${names.xenonet} MCP server, for every project`, where: "Codex's config, through codex mcp add", state: k.serverState(server), detail: `${k.command} mcp (XENONET_PORT=${port})` },
          { id: "hooks", what: `Hooks: ${k.eventList(app)}`, where: tilde(hooksFile, home), state: k.hooksState(readJson(hooksFile, home), ours, hookSet, isXenoNet), detail: `running ${k.command} hook ${app.id}` },
        ],
        notes: [
          ...(cli ? [] : ["XenoNet can't find the codex command, so it can add the hooks but not the MCP server."]),
          "Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's.",
          ...(k.earlier(readJson(hooksFile, home).hooks, isXenoNet, server) ? [EARLIER] : []),
          ...k.besides(k.others(readJson(hooksFile, home).hooks, isXenoNet)),
        ],
        broken: k.broken(server, readJson(hooksFile, home).hooks, ours),
      };
    },

    async apply() {
      writeJson(hooksFile, mergeHooks(readJson(hooksFile, home), ours, hookSet));
      const bin = k.cli("codex");
      if (bin) {
        await run(bin, ["mcp", "remove", names.xenonet]).catch(() => {});
        await add(bin, names.xenonet);
        if (k.consoleMcp) {
          await run(bin, ["mcp", "remove", names.console]).catch(() => {});
          await add(bin, names.console, ["console"]);
        }
      }
    },

    async removeHooks() {
      if (fs.existsSync(hooksFile)) writeJson(hooksFile, mergeHooks(readJson(hooksFile, home), ours));
    },

    async removeServer(cli, name) {
      if (cli) await run(cli, ["mcp", "remove", name]).catch(() => {});
    },
  };
}

// A text file, or "" when there's none. One that can't be read is left
// alone, as a settings file is.
function readText(file, home) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (err) {
    if (err.code === "ENOENT") return "";
    throw new Error(`XenoNet can't read ${tilde(file, home)} (${err.code ?? tilde(err.message, home)}), so it leaves it alone`);
  }
}

// Grok Build's settings (grok-hooks): the xenonet MCP server as tables in
// config.toml, which XenoNet writes itself (`grok mcp add` rewrites the file
// and drops its comments; see toml.js), and command hooks in Claude Code's
// format in a file of XenoNet's own, hooks/xenonet.json, which Grok runs
// without asking, as it does every file in its hooks folder. GROK_HOME
// moves both. Grok gives a session's MCP servers its id (GROK_SESSION_ID),
// so the server knows whose it is (mcp/xenonet.js).
function grokHooks(app, k) {
  const { env, home, port, platform, names, mcp } = k;
  const grokHome = env.GROK_HOME ?? path.join(home, ".grok");
  const configFile = path.join(grokHome, "config.toml");
  const hooksFile = path.join(grokHome, "hooks", "xenonet.json");
  const isXenoNet = k.xenonetCommand(app.id);
  const ours = k.mine(isXenoNet);
  // Grok runs a hook's command with sh -c, and on Windows under the shell
  // it finds there, so the Windows form is one command line. Its
  // SessionEnd hooks get little time.
  const hookSet = k.hookSet(app, (event) => ({ type: "command", command: platform === "win32" ? k.windowsHook(app.id) : k.hookCommand(app.id), timeout: event === "SessionEnd" ? 3 : 10 }));
  const config = () => readText(configFile, home);
  const foreign = (name) => `${tilde(configFile, home)} names mcp_servers.${name} in a form XenoNet doesn't edit. Take it out, or write it as a [mcp_servers.${name}] table, and connect again.`;

  // Writes config.toml as `edit` makes it: through a copy of the file (so
  // it keeps its mode), with a copy of it as it was the first time XenoNet
  // changes it.
  function writeConfig(edit) {
    const before = config();
    const after = edit(before);
    if (after === before) return;
    fs.mkdirSync(path.dirname(configFile), { recursive: true });
    const tmp = `${configFile}.tmp-xenonet`;
    if (fs.existsSync(configFile)) {
      if (!fs.existsSync(`${configFile}.before-xenonet`)) fs.copyFileSync(configFile, `${configFile}.before-xenonet`);
      fs.copyFileSync(configFile, tmp);
    }
    fs.writeFileSync(tmp, after);
    fs.renameSync(tmp, configFile);
  }

  // XenoNet's hooks file: written as the settings files are, and taken out
  // once no XenoNet's hooks are left in it.
  function writeHooks(add) {
    const merged = mergeHooks(readJson(hooksFile, home), ours, add);
    if (!add && !Object.keys(merged).length) return void fs.rmSync(hooksFile, { force: true });
    writeJson(hooksFile, merged);
  }

  // Whether Grok Build is here: its command, or its folder.
  const installed = () => !!k.cli("grok") || fs.existsSync(grokHome);

  return {
    cli: "grok",
    installed,
    // The server as config.toml has it, or null.
    server: async (_cli, name) => readServer(config(), name),

    async entry({ cli, server, console }) {
      const text = config();
      const hooks = readJson(hooksFile, home);
      return {
        app: app.id,
        name: app.name,
        cli,
        // Connecting every app leaves it out until it's here.
        installed: installed(),
        changes: [
          ...k.consoleChange(app.id, console),
          { id: "mcp", what: `The ${names.xenonet} MCP server, for every project`, where: tilde(configFile, home), state: foreignServer(text, names.xenonet) ? "different" : k.serverState(server), detail: `${k.command} mcp (XENONET_PORT=${port}), as [mcp_servers.${names.xenonet}]; the rest of the file stays as it is` },
          { id: "hooks", what: `Hooks: ${k.eventList(app)}`, where: tilde(hooksFile, home), state: k.hooksState(hooks, ours, hookSet, isXenoNet), detail: `running ${k.command} hook ${app.id}` },
        ],
        notes: [
          ...(cli ? [] : ["XenoNet can't find the grok command. Connecting writes Grok Build's settings all the same, for when you install it."]),
          ...(foreignServer(text, names.xenonet) ? [foreign(names.xenonet)] : []),
          "Grok Build runs the hooks in its hooks folder without asking.",
          ...k.besides(k.others(hooks.hooks, isXenoNet)),
        ],
        broken: k.broken(server, hooks.hooks, ours),
      };
    },

    async apply() {
      writeHooks(hookSet);
      writeConfig((text) => {
        let out = withServer(text, names.xenonet, mcp);
        if (k.consoleMcp) out = withServer(out, names.console, { ...mcp, args: [...mcp.args, "console"] });
        return out;
      });
    },

    async removeHooks() {
      if (fs.existsSync(hooksFile)) writeHooks(null);
    },

    // Only this XenoNet's servers, by their names: another's stay.
    async removeServer(_cli, name) {
      if (fs.existsSync(configFile)) writeConfig((text) => withoutServer(text, name));
    },
  };
}

// The config formats XenoNet connects an app through, by the name its
// registry entry gives (connect.format). Each is made for one app, with
// what every format shares (k, below), and has:
//
//   cli                      the command its MCP server goes in through
//   server(cli, name)        the MCP server of that name as the app has it
//   entry({ cli, server, console })
//                            the app's part of the preview, given its
//                            command, its xenonet server and its console's
//   apply()                  puts XenoNet's hooks and servers in
//   removeHooks()            takes XenoNet's hooks out
//   removeServer(cli, name)  takes the MCP server of that name out
//   hookConnected(event)     for Claude Code's settings only, which
//                            XenoNet's plugin carries too (see above)
//   installed()              for an app that may not be here (Grok
//                            Build): whether it is, so connecting every
//                            app leaves it out until it is
export const FORMATS = { "claude-settings": claudeSettings, "codex-hooks": codexHooks, "grok-hooks": grokHooks };

// `bin`: the xenonet binary the entries run, xenonetBin's by default.
// `consoleMcp`: whether the console operator's entry is XenoNet's too, as
// for the installed XenoNet, which Desktop registers it for.
// `registry` and `formats`: the apps and the formats connect goes through,
// the app registry's and FORMATS unless a test gives its own.
export function createConnect({ port, root, demo = false, home = os.homedir(), env = process.env, platform = process.platform, bin = xenonetBin({ env, root, platform }), run = runFile, consoleMcp = !!(env.XENONET_LAUNCHER_BIN || env.XENONET_LAUNCHER_EXEC), registry = APPS, formats = FORMATS }) {
  const serverEnv = { XENONET_PORT: String(port) };
  const mcp = { type: "stdio", command: bin, args: ["mcp"], env: serverEnv };
  const names = serverNames(port);
  const hookCommand = (app) => `${Object.entries(serverEnv).map(([key, v]) => `${key}=${v}`).join(" ")} ${quote(bin)} hook ${app}`;
  const windowsHook = (app) => {
    const script = `# XenoNet hook ${app}\n${Object.entries(serverEnv).map(([key, v]) => `$env:${key}=${psQuote(v)}`).join("; ")}; & ${psQuote(bin)} hook ${psQuote(app)}; exit $LASTEXITCODE`;
    return `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ${Buffer.from(script, "utf16le").toString("base64")}`;
  };
  const hookText = (h) => {
    const command = h.commandWindows ?? h.command ?? "";
    const encoded = /-EncodedCommand ([A-Za-z0-9+/=]+)$/.exec(command)?.[1];
    return encoded ? Buffer.from(encoded, "base64").toString("utf16le") : command;
  };

  // Any XenoNet's command entries for an app, whichever XenoNet (port) or
  // checkout added them: the binary's, cli/xenonet.js's with Node, or the
  // hook.js of earlier versions.
  const xenonetCommand = (app) => (h) => h?.type === "command" && (new RegExp(`(cli[\\\\/]xenonet\\.js['"]? hook|(cli|server[\\\\/]engine)[\\\\/]hook\\.js['"]?|[\\\\/]xenonet(\\.exe)?['"]? hook) ${app}$`).test(h.command ?? "") || (hookText(h).startsWith(`# XenoNet hook ${app}\n`) && hookText(h).endsWith(` hook '${app}'; exit $LASTEXITCODE`)));
  // Which XenoNet an entry is for: its port, from its URL or XENONET_PORT; null
  // for an early form that didn't say.
  const portOf = (h) => {
    const m = h?.type === "http" ? /:(\d+)\/api\/hook\//.exec(h.url ?? "") : /XENONET_PORT=['"]?(\d+)/.exec(hookText(h));
    return m ? Number(m[1]) : null;
  };
  // Only the paths among its quoted words: a Windows hook quotes its port
  // and app too ('47840', 'claude'), which are no files to look for.
  const hookFiles = (hooks, ours) => Object.values(hooks ?? {}).flat().flatMap((g) => g.hooks ?? []).filter((h) => ours(h) && h.type === "command").flatMap((h) => [...hookText(h).matchAll(/'((?:[^']|'')+)'/g)].map((m) => m[1].replace(/''/g, "'")).filter((f) => path.isAbsolute(f) || path.win32.isAbsolute(f)));
  // An entry is this XenoNet's when it's for this port, or says no port, or
  // runs files that are gone; another live XenoNet's stays as it is.
  const mine = (isXenoNet) => (h) => isXenoNet(h) && (portOf(h) === port || portOf(h) == null || hookFiles({ x: [{ hooks: [h] }] }, isXenoNet).some((f) => !fs.existsSync(f)));
  // Whether this XenoNet has hooks here, in place or not: its port's, or an
  // early form's that named none.
  const ownHooksOf = (hooks, isXenoNet) => Object.values(hooks ?? {}).flat().flatMap((g) => g.hooks ?? []).filter((h) => isXenoNet(h) && (portOf(h) === port || portOf(h) == null));
  // A CLI on PATH (XenoNet's PATH is your login shell's).
  const cli = (name) => onPath(name, { env, platform, canRun: (f) => fs.accessSync(f) });
  // Files a connected entry runs that no longer exist (a moved or deleted
  // checkout, an uninstalled app).
  const gone = (command, args = []) => [command, ...args.filter((a) => /\.m?js$/.test(a))].filter((f) => f && path.isAbsolute(f) && !fs.existsSync(f));
  // The xenonet server as each app has it: missing, present (as XenoNet would
  // add it) or different.
  const serverState = (s, args = mcp.args) => (!s ? "missing" : s.command === mcp.command && JSON.stringify(s.args) === JSON.stringify(args) && s.env?.XENONET_PORT === mcp.env.XENONET_PORT ? "present" : "different");
  const command = tilde(bin, home);

  // What every format shares.
  const k = {
    port,
    home,
    env,
    platform,
    run,
    consoleMcp,
    names,
    mcp,
    serverEnv,
    // The xenonet binary, with the home folder as ~.
    command,
    cli,
    hookCommand,
    windowsHook,
    xenonetCommand,
    portOf,
    hookFiles,
    mine,
    ownHooksOf,
    serverState,
    // The groups XenoNet adds for an app's hook events, each with its
    // matcher when it takes one, as the app registry has them: hook(event)
    // is the hook in each.
    hookSet: (app, hook) => Object.fromEntries(app.connect.events.map(([event, matcher]) => [event, { ...(matcher ? { matcher } : {}), hooks: [hook(event)] }])),
    // An app's hook events as the preview lists them.
    eventList: (app) => app.connect.events.map(([e, m]) => (m ? `${e} (edits)` : e)).join(", "),
    // A hooks change's state: as XenoNet would add them, or there but not
    // so (an earlier XenoNet's, say), or not there.
    hooksState: (settings, ours, add, isXenoNet) => (hooksInPlace(settings, ours, add) ? "present" : ownHooksOf(settings.hooks, isXenoNet).length ? "different" : "missing"),
    // Whether this XenoNet's entries here are an earlier XenoNet's, which
    // run Node and cli/xenonet.js (or hook.js) rather than the binary: a
    // hook of its own, or its MCP server.
    earlier: (hooks, isXenoNet, server) => ownHooksOf(hooks, isXenoNet).some((h) => h.type === "command" && /xenonet\.js|hook\.js/.test(hookText(h))) || (Array.isArray(server?.args) && server.args.some((a) => /\.m?js$/.test(String(a)))),
    // Other XenoNets connected here too, by port, for the preview.
    others: (hooks, isXenoNet) => [...new Set(Object.values(hooks ?? {}).flat().flatMap((g) => g.hooks ?? []).filter((h) => isXenoNet(h) && !mine(isXenoNet)(h)).map(portOf))].sort(),
    // Another XenoNet on this machine connected too (XenoNet Dev beside the
    // installed one): its entries stay as they are.
    besides: (list) => list.map((p) => `Another XenoNet (port ${p}${Object.values(INSTANCES).find((i) => i.port === p) ? `, ${Object.values(INSTANCES).find((i) => i.port === p).name}` : ""}) is connected here too; its entries stay as they are.`),
    broken: (entry, hooks, ours) => [...(entry ? gone(entry.command, entry.args) : []), ...hookFiles(hooks, ours).filter((f) => !fs.existsSync(f))].map((f) => `${tilde(f, home)} is gone`),
    // The console operator's MCP server, when it's XenoNet's too.
    consoleChange: (app, server) => (consoleMcp ? [{ id: "console", what: `The ${names.console} MCP server, to operate Desktop`, where: `${app} user settings`, state: serverState(server, [...mcp.args, "console"]), detail: `${command} mcp console (XENONET_PORT=${port})` }] : []),
  };

  // The apps connect goes through, each with its format's connect.
  const connected = registry.filter((a) => a.has?.includes("connect") && Object.hasOwn(formats, a.connect?.format ?? "")).map((app) => ({ app, f: formats[app.connect.format](app, k) }));

  // Each app's command first, then its xenonet server, then its console's,
  // then its part of the preview, in the registry's order.
  async function preview() {
    const clis = connected.map(({ f }) => cli(f.cli));
    const servers = [], consoles = [];
    for (const [i, { f }] of connected.entries()) servers.push(await f.server(clis[i], names.xenonet));
    if (consoleMcp) for (const [i, { f }] of connected.entries()) consoles.push(await f.server(clis[i], names.console));
    const apps = [];
    for (const [i, { f }] of connected.entries()) apps.push(await f.entry({ cli: clis[i], server: servers[i], console: consoles[i] }));
    return { port, demo, apps };
  }

  // `apps`: the ids of the apps to connect; every app that's here when
  // it's not given.
  async function apply(apps = null) {
    // Agents connected to the demo would see only its made-up sessions.
    if (demo) throw new Error(`this is the demo, so connect ${andList(connected.map(({ app }) => app.name))} from XenoNet, with your own sessions`);
    const chosen = apps ?? connected.filter(({ f }) => f.installed?.() ?? true).map(({ app }) => app.id);
    for (const { app, f } of connected) if (chosen.includes(app.id)) await f.apply();
    return preview();
  }

  // Each app's hooks first, then its server, then its console's.
  async function remove() {
    for (const { f } of connected) await f.removeHooks();
    const clis = connected.map(({ f }) => cli(f.cli));
    for (const [i, { f }] of connected.entries()) await f.removeServer(clis[i], names.xenonet);
    if (consoleMcp) for (const [i, { f }] of connected.entries()) await f.removeServer(clis[i], names.console);
    return preview();
  }

  // Whether connect's own hook for `event` is in place where XenoNet's
  // plugin carries a copy of it (Claude Code's settings): false where no
  // format says.
  async function hookConnected(event) {
    for (const { f } of connected) if (f.hookConnected) return f.hookConnected(event);
    return false;
  }

  return { preview, apply, remove, hookConnected };
}

function runFile(file, args) {
  return new Promise((resolve, reject) =>
    execFile(file, args, { timeout: 20000, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) => (err ? reject(new Error(stderr.trim() || err.message)) : resolve(stdout))),
  );
}
