// core/mcp-info.js in Rust (crates/xenonet-app/src/mcp_info.rs): a catalog
// of an MCP server for the console, without configuration or credentials.
// Collecting one is a Rust state machine; this asks Codex or the server
// what it says to, and hands back the answer.
import { native } from "../index.js";
import { peer } from "../peer.js";

const { config } = await import(peer("config.js"));
const { codexClient } = await import(peer("codex.js"));
const { discoverMcp } = await import(peer("mcp-discovery.js"));

// What collecting reads of the settings, and of each session.
const settingsOf = (s) => ({ claudeConfigFile: s?.claudeConfigFile, claudeDesktopDir: s?.claudeDesktopDir, homeDir: s?.homeDir, grokHome: s?.grokHome });
const sessionOf = (s) => s && { id: s.id, app: s.app, title: s.title, status: s.status, cwd: s.cwd, mcp: s.mcp };

export const publicUrl = (value, originOnly = false) => native.appMcpPublicUrl(value == null ? null : String(value), !!originOnly) ?? null;

export const displaySchema = (value, depth = 0) => native.appMcpDisplaySchema(value, Number(depth));

export const displayCatalog = (raw = {}) => native.appMcpDisplayCatalog(raw);

// read(file) is the JS's own JSON reader when it's left out: the Rust reads.
export function createMcpInfo({ request = (method, params) => codexClient.request(method, params), discover = discoverMcp, read = null, settings = config, now = Date.now } = {}) {
  const cache = new Map();
  const pending = new Map();
  async function collect(server, sessions) {
    const picked = settingsOf(settings);
    let s = native.appMcpBegin(String(server), sessions.map(sessionOf), picked, Number(now()), read);
    while (!("done" in s.next)) {
      let answer;
      try {
        answer = { ok: await (s.next.request ? request(s.next.request.method, s.next.request.params) : discover(s.next.discover)) };
      } catch (err) {
        answer = { error: { name: err?.name ?? null, code: err?.code ?? null } };
      }
      s = native.appMcpStep(s.state, answer, picked, read);
    }
    return s.next.done;
  }
  return {
    async get(server, sessions = [], { refresh = false } = {}) {
      const hit = cache.get(server);
      if (!refresh && hit && native.appMcpFresh(Number(hit.checkedAt), Number(now()))) return hit;
      if (!pending.has(server)) pending.set(server, collect(server, sessions).then((r) => (cache.set(server, r), r)).finally(() => pending.delete(server)));
      return pending.get(server);
    },
  };
}

export const demoMcpInfo = (server) => native.appDemoMcpInfo(String(server), Date.now());
