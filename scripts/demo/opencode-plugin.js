// XenoNet's plugin for OpenCode (https://opencode.ai/docs/plugins/): it tells
// the XenoNet on this machine, as it happens, whether a session is working or
// idle, whether it waits on a permission, which files it edited, and what
// each finished assistant message used (cost, tokens, model). Copy it to
// .opencode/plugins/xenonet.js in a project, or to
// ~/.config/opencode/plugins/xenonet.js for every project.
//
// Metadata only. It never sends a prompt, a reply, a tool's output or a
// file's content: for a permission, only its id and the name of the tool it
// is for; for an edit, only the file's path. It fails open and fast: when
// XenoNet isn't running, a call is dropped after 800 ms and the plugin stays
// quiet for 30 s before it tries again, and nothing it does can fail a
// session. A XenoNet that doesn't know these events ignores them.
//
// The calls go to XENONET_PORT, else the installed XenoNet's 47840, as the
// session (see opencode-tool.js, which gives it tools to message others).
//
// OpenCode calls every export of a plugin file as a plugin, so this file
// exports the one.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const QUIET_MS = 30e3;
const EDIT_TOOLS = new Set(["edit", "write", "multiedit", "patch", "apply_patch"]);

const base = () => `http://127.0.0.1:${Number(process.env.XENONET_PORT) || 47840}`;
// The API's credential the XenoNet on that port wrote when it started, in
// its runtime folder (XENONET_RUNTIME_DIR, else runtime in XenoNet's state
// folder), which only your account can read. Read at each call, since
// XenoNet writes a new one each time it starts; none when there's none.
const runtimeDir = () => {
  const env = process.env;
  if (env.XENONET_RUNTIME_DIR) return env.XENONET_RUNTIME_DIR;
  const home = os.homedir();
  const state = env.XDG_STATE_HOME
    ? path.join(env.XDG_STATE_HOME, "xenonet")
    : process.platform === "win32"
      ? path.join(env.LOCALAPPDATA ?? path.join(home, "AppData", "Local"), "XenoNet")
      : process.platform === "darwin"
        ? path.join(home, "Library", "Application Support", "XenoNet")
        : path.join(home, ".local", "state", "xenonet");
  return path.join(state, "runtime");
};
const credential = () => {
  try {
    const value = fs.readFileSync(path.join(runtimeDir(), `api-${Number(process.env.XENONET_PORT) || 47840}.cred`), "utf8").trim();
    return /^[a-f0-9]+$/.test(value) ? { "x-xenonet-credential": value } : {};
  } catch {
    return {};
  }
};

const agentId = (sessionID) => `opencode:${String(sessionID).replace(/^ses_/, "ses-")}`;
const str = (v) => (typeof v === "string" && v ? v : null);
const num = (v) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : undefined);

// The files an apply_patch text names, from its headers only.
const patchFiles = (text) => (typeof text === "string" ? [...text.matchAll(/^\*\*\* (?:Add File|Update File|Delete File|Move to): (.+)$/gm)].map((m) => m[1].trim()) : []);

// What an OpenCode bus event says that XenoNet wants, as [sessionID, event]
// pairs in XenoNet's words, or none. Whatever else an event carries stays
// here.
function fromBus(event) {
  const p = event?.properties ?? {};
  switch (event?.type) {
    case "session.status": {
      const type = p.status?.type;
      return str(p.sessionID) && type ? [[p.sessionID, { type: "status", state: type === "idle" ? "idle" : "busy" }]] : [];
    }
    case "session.idle":
      return str(p.sessionID) ? [[p.sessionID, { type: "status", state: "idle" }]] : [];
    case "permission.asked":
    case "permission.updated": {
      const id = str(p.id);
      return str(p.sessionID) && id ? [[p.sessionID, { type: "permission", state: "asked", id, tool: str(p.permission) ?? str(p.type) ?? undefined }]] : [];
    }
    case "permission.replied": {
      const id = str(p.requestID) ?? str(p.permissionID) ?? str(p.id);
      return str(p.sessionID) && id ? [[p.sessionID, { type: "permission", state: "replied", id }]] : [];
    }
    case "message.updated": {
      const m = p.info;
      // Only a finished assistant message: it has its cost by then.
      if (m?.role !== "assistant" || !str(m.id) || !str(m.sessionID) || !m.time?.completed) return [];
      const t = m.tokens ?? {};
      return [[m.sessionID, { type: "message", id: m.id, cost: num(m.cost), tokens: { input: num(t.input), output: num(t.output), reasoning: num(t.reasoning), cacheRead: num(t.cache?.read), cacheWrite: num(t.cache?.write) }, provider: str(m.providerID) ?? undefined, model: str(m.modelID) ?? undefined }]];
    }
    default:
      return [];
  }
}

export const XenoNet = async ({ directory } = {}) => {
  let quietUntil = 0;
  const post = (sessionID, events) => {
    if (!events.length || Date.now() < quietUntil) return;
    // Not awaited: a hook returns at once, and a failure is nobody's.
    Promise.resolve()
      .then(() =>
        fetch(`${base()}/api/engine/opencode-event`, {
          method: "POST",
          headers: { "content-type": "application/json", "x-xenonet-agent": agentId(sessionID), ...credential() },
          body: JSON.stringify({ events, cwd: directory, pid: process.pid }),
          signal: AbortSignal.timeout(800),
        }),
      )
      .then((r) => {
        // Not XenoNet's, or one too old to know the route: stay quiet a while.
        if (r.status === 404) quietUntil = Date.now() + QUIET_MS;
      })
      .catch(() => {
        quietUntil = Date.now() + QUIET_MS;
      });
  };
  return {
    event: async ({ event }) => {
      try {
        for (const [sessionID, e] of fromBus(event)) post(sessionID, [e]);
      } catch {}
    },
    // The files an edit tool changed, once it has: file.edited has no
    // session, and this does.
    "tool.execute.after": async (input) => {
      try {
        if (!str(input?.sessionID) || !EDIT_TOOLS.has(input.tool)) return;
        const a = input.args ?? {};
        const files = [str(a.filePath), str(a.path), ...patchFiles(a.patchText)].filter(Boolean);
        post(input.sessionID, files.map((path) => ({ type: "edit", path })));
      } catch {}
    },
  };
};
