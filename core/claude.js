// Claude Code adapter. Sessions and transcripts come from the Agent SDK (the
// supported interface). Desktop-only details (starred, archived, worktree,
// lineage) come from Claude desktop's own session files, which are
// undocumented, so every field from them is optional. All three are read
// leniently against their schemas (vendor.js): a sample of each listing
// checked whole, what differs kept as drift.
import fs from "node:fs";
import path from "node:path";
import { listSessions, getSessionMessages, renameSession } from "@anthropic-ai/claude-agent-sdk";
import { config } from "./config.js";
import { excerpt } from "./units.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, agentSdkVersion } from "./vendor.js";

const listings = reader("anthropic/agent-sdk/session-listing");
const messages = reader("anthropic/agent-sdk/session-message");
const desktopFiles = reader("anthropic/claude-desktop/session");
// Where each desktop session's file is, by its desktop id, as the last
// listing found them, for renaming one (renameClaudeSession).
const desktopFileOf = new Map();

function readDesktopSessions() {
  const byCliId = new Map();
  const root = config.claudeDesktopDir && path.join(config.claudeDesktopDir, "claude-code-sessions");
  if (!root || !fs.existsSync(root)) return byCliId;
  // Sessions sit in claude-code-sessions/<account>/<organization>/, so the
  // folders say which Claude account each ran as (see identities.js).
  const walk = (dir, parts) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, ent.name);
      if (ent.isDirectory() && parts.length < 3 && !ent.name.startsWith("deleted_")) walk(full, [...parts, ent.name]);
      else if (ent.isFile() && /^local_.+\.json$/.test(ent.name)) {
        try {
          const d = desktopFiles.sample(JSON.parse(fs.readFileSync(full, "utf8")));
          if (!d) continue;
          const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
          if (parts.length === 2 && parts.every((p) => uuid.test(p))) d.account = { account: parts[0], org: parts[1] };
          if (d.cliSessionId) byCliId.set(d.cliSessionId, d);
          if (typeof d.sessionId === "string") desktopFileOf.set(d.sessionId, full);
        } catch {}
      }
    }
  };
  try {
    walk(root, []);
  } catch {}
  return byCliId;
}

export async function listClaudeSessions() {
  const [listed, desktop] = await Promise.all([listSessions(), Promise.resolve(readDesktopSessions())]);
  const version = agentSdkVersion();
  const sdkSessions = listed.map((s) => listings.sample(s, { version })).filter(Boolean);
  const desktopIdToCli = new Map([...desktop.values()].map((d) => [d.sessionId, d.cliSessionId]));
  return sdkSessions.map((s) => {
    const d = desktop.get(s.sessionId);
    const parentCli = d?.spawnedFrom?.sessionId ? desktopIdToCli.get(d.spawnedFrom.sessionId) : null;
    return {
      id: `claude:${s.sessionId}`,
      app: "claude",
      rawId: s.sessionId,
      desktopId: d?.sessionId ?? null,
      inDesktop: !!d,
      title: d?.title || s.customTitle || s.summary || s.firstPrompt?.slice(0, 80) || "(untitled session)",
      preview: s.firstPrompt ?? null,
      // Worktree sessions belong to the repo they were branched from.
      cwd: d?.originCwd || s.cwd || null,
      worktree: d?.worktreePath ?? null,
      createdAt: s.createdAt ?? d?.createdAt ?? null,
      updatedAt: Math.max(s.lastModified ?? 0, d?.lastActivityAt ?? 0) || null,
      archived: !!d?.isArchived,
      starred: !!d?.isStarred,
      model: d?.model ?? null,
      effort: d?.effort ?? null,
      permissionMode: d?.permissionMode ?? null,
      branch: d?.branch ?? s.gitBranch ?? null,
      tag: s.tag ?? null,
      sizeBytes: s.fileSize ?? null,
      parentId: parentCli ? `claude:${parentCli}` : null,
      parentKind: parentCli ? "spawn" : null,
      claudeAccount: d?.account ?? null,
    };
  });
}

// Claude Code's sessions as a gatherer (core/blocks/gatherer.js): listed
// again on every refresh, timed, and empty after a listing that failed
// (the map keeps the sessions it had; see state.js). `gather` stands in for
// the listing, as the demo's does.
export const claudeSessionsGatherer = ({ gather = listClaudeSessions, clock } = {}) => new Gatherer({ name: "claude-sessions", gather, fallback: [], clock });

// Renames a session where Claude keeps its name. Claude Code's transcript
// gets a custom title through the Agent SDK, as Claude desktop writes its own
// renames. A session in Claude desktop is renamed in the desktop's record
// too, which the desktop reads when it starts; otherwise it would put its
// old title back into the transcript the next time the session runs there.
// Says where the name went: claude, and desktop when its record was written.
export async function renameClaudeSession({ rawId, desktopId }, title) {
  await renameSession(rawId, title);
  if (!desktopId) return { claude: true, desktop: false };
  try {
    await renameDesktopRecord(desktopId, rawId, title);
    return { claude: true, desktop: true };
  } catch (err) {
    return { claude: true, desktop: false, desktopError: String(err?.message ?? err) };
  }
}

// The desktop's record of the session, with only its title changed and the
// title marked as yours, so the desktop's own naming doesn't replace it. The
// file is undocumented: it's read again just before the write, and left
// alone unless it's still the same session's.
export async function renameDesktopRecord(desktopId, cliId, title, file = desktopFileOf.get(desktopId)) {
  if (!file) throw new Error("XenoNet hasn't found Claude desktop's record of this session");
  const record = JSON.parse(await fs.promises.readFile(file, "utf8"));
  if (record?.sessionId !== desktopId || record.cliSessionId !== cliId) throw new Error("Claude desktop's record is of another session now");
  record.title = title;
  record.titleSource = "user";
  const tmp = `${file}.xenonet-${process.pid}.tmp`;
  await fs.promises.writeFile(tmp, JSON.stringify(record));
  // Windows refuses the rename for a moment while the desktop has the file open.
  for (let attempt = 0; ; attempt++) {
    try {
      return await fs.promises.rename(tmp, file);
    } catch (err) {
      if (attempt >= 4 || !["EPERM", "EBUSY", "EACCES"].includes(err.code)) {
        await fs.promises.rm(tmp, { force: true });
        throw err;
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  }
}

function blockText(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.map((c) => (c.type === "text" ? c.text : c.type === "image" ? "[image]" : "")).join("\n");
}

function normalizeMessage(m) {
  const ts = m.timestamp ? Date.parse(m.timestamp) : null;
  const content = m.message?.content;
  const out = [];
  if (typeof content === "string") {
    out.push({ id: m.uuid, ts, role: m.type === "assistant" ? "assistant" : m.type === "user" ? "user" : "system", text: content });
    return out;
  }
  for (const [i, b] of (content ?? []).entries()) {
    const id = `${m.uuid}:${i}`;
    if (b.type === "text") out.push({ id, ts, role: m.type === "assistant" ? "assistant" : "user", text: b.text });
    else if (b.type === "thinking") out.push({ id, ts, role: "reasoning", text: b.thinking ?? "" });
    else if (b.type === "tool_use") out.push({ id, ts, role: "tool", kind: "tool_use", title: b.name, text: excerpt(JSON.stringify(b.input, null, 2), 2500) });
    else if (b.type === "tool_result") out.push({ id, ts, role: "tool", kind: "tool_result", title: "result", status: b.is_error ? "failed" : "completed", text: excerpt(blockText(b.content)) });
  }
  return out;
}

// Newest page first; `cursor` is the count of messages already shown from the end.
export async function readClaudeSession(rawId, { cursor = null, limit = 120 } = {}) {
  const all = await getSessionMessages(rawId);
  const shown = Number(cursor ?? 0);
  const end = all.length - shown;
  const start = Math.max(0, end - limit);
  const version = agentSdkVersion();
  const page = all.slice(start, end).map((m) => messages.sample(m, { version, kind: m?.type })).filter(Boolean);
  const entries = page.flatMap(normalizeMessage).filter((e) => e.text || e.title);
  return { entries, cursor: start > 0 ? String(all.length - start) : null, total: all.length };
}
