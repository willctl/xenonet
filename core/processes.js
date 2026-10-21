// How XenoNet tells each app's processes apart: the apps the registry gives
// `processes`, each with its own rules here, by its id.
//
//   program(p, base)  whether a program is one of the app's own, from its
//                     path (with / for \) and its file's name
//   script(args)      whether Node or Bun running these arguments runs the
//                     app's package
//   pinsByHook        the app sends its hooks from its own process, which
//                     its maker signs, so they pin a session to that
//                     process (engine/verify.js)
//   anyProcess        any of the app's processes may have any of its
//                     sessions open, so they aren't pinned to one
//
// The team that signs an app is the registry's (team). procs.js reads the
// processes with these, and engine/verify.js checks that a request in a
// session's name comes from its app's process.
import path from "node:path";
import { APPS } from "./apps.js";

export const PROCESSES = {
  // The CLI, the native install under ~/.local/share/claude/versions, or
  // the one Claude desktop runs from its claude.app.
  claude: {
    program: (p, base) => base === "claude" || /\/claude\/versions\/[^/]+$/.test(p),
    script: (args) => /@anthropic-ai\/claude-code\/cli\.js/.test(args),
    pinsByHook: true,
  },
  // The CLI, or the one the ChatGPT app bundles.
  codex: {
    program: (_p, base) => base === "codex" || /^codex-(aarch64|x86_64)/.test(base),
    script: (args) => /@openai\/codex\//.test(args),
  },
  // The grok binary its installer puts in ~/.grok/downloads and links
  // from ~/.grok/bin (as grok, and as agent, which Cursor's CLI is called
  // too, so that name counts only there).
  grok: {
    program: (p, base) => base === "grok" || base === "grok.exe" || /^grok-(macos|linux|windows)-(aarch64|arm64|x86_64|x64)(\.exe)?$/.test(base) || /\/\.grok\/(bin|downloads)\/[^/]+$/.test(p),
  },
  // Its one binary, however it was installed. Every OpenCode reads the
  // same database and can open any session.
  opencode: {
    program: (_p, base) => base === "opencode" || base === "opencode.exe",
    anyProcess: true,
  },
};

// The apps whose processes XenoNet tells apart, in the registry's order,
// each with its name and its rules: those with `processes` that have rules
// here. A test can give its own registry and rules.
export const agentApps = (registry = APPS, rules = PROCESSES) => registry.filter((a) => a.has?.includes("processes") && Object.hasOwn(rules, a.id)).map((a) => ({ id: a.id, name: a.name, ...rules[a.id] }));

const AGENTS = agentApps();

// The agent app a program is, from its path: the first app whose program it
// is, in the registry's order. Programs run by Node or Bun are told by their
// arguments, when given.
export function agentOf(program, args = "", agents = AGENTS) {
  const p = String(program ?? "").replace(/\\/g, "/");
  const base = path.posix.basename(p);
  for (const a of agents) if (a.program(p, base)) return a.id;
  if (/^(node|bun)$/.test(base)) for (const a of agents) if (a.script?.(args)) return a.id;
  return null;
}
