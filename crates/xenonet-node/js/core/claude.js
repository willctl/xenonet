// core/claude.js in Rust (crates/xenonet-sources/src/claude.rs): the Claude
// Code adapter. Sessions and transcripts come from the Agent SDK, whose
// calls stay here, as do the lenient checks of what it and Claude
// desktop's files say (vendor.js's readers). Rust reads Claude desktop's
// session files, merges them into the SDK's listing, turns a transcript's
// messages into what the console shows, pages through them, and checks
// and edits the desktop's record when a session is renamed.
import fs from "node:fs";
import path from "node:path";
import { listSessions, getSessionMessages, renameSession } from "@anthropic-ai/claude-agent-sdk";
import { config } from "../../../../core/config.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, agentSdkVersion } from "./vendor.js";
import { native } from "../index.js";

const listings = reader("anthropic/agent-sdk/session-listing");
const messages = reader("anthropic/agent-sdk/session-message");
const desktopFiles = reader("anthropic/claude-desktop/session");
// Where each desktop session's file is, by its desktop id, as the last
// listing found them, for renaming one (renameClaudeSession).
const desktopFileOf = new Map();

// Claude desktop's records, by the CLI session each is of. Sessions sit in
// claude-code-sessions/<account>/<organization>/, so the folders say which
// Claude account each ran as (see identities.js).
function readDesktopSessions() {
  const root = config.claudeDesktopDir && path.join(config.claudeDesktopDir, "claude-code-sessions");
  const records = [];
  for (const f of root ? native.claudeDesktopFiles(root) : []) {
    // A file that isn't JSON is passed over.
    if (f.data === undefined) continue;
    try {
      const d = desktopFiles.sample(f.data);
      if (d) records.push([f.file, f.parts, d]);
    } catch {}
  }
  const { desktop, files } = native.claudeDesktopIndex(records);
  for (const [id, file] of files) desktopFileOf.set(id, file);
  return desktop;
}

export async function listClaudeSessions() {
  const [listed, desktop] = await Promise.all([listSessions(), Promise.resolve(readDesktopSessions())]);
  const version = agentSdkVersion();
  const sdkSessions = listed.map((s) => listings.sample(s, { version })).filter(Boolean);
  return native.claudeSessions(sdkSessions, desktop);
}

// Claude Code's sessions as a gatherer: listed again on every refresh,
// timed, and empty after a listing that failed. `gather` stands in for the
// listing, as the demo's does.
export const claudeSessionsGatherer = ({ gather = listClaudeSessions, clock } = {}) => new Gatherer({ name: "claude-sessions", gather, fallback: [], clock });

// Renames a session where Claude keeps its name: Claude Code's transcript
// through the Agent SDK, and Claude desktop's record too when the session
// is in the desktop. Says where the name went.
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
// title marked as yours. The file is undocumented: it's read again just
// before the write, and left alone unless it's still the same session's.
export async function renameDesktopRecord(desktopId, cliId, title, file = desktopFileOf.get(desktopId)) {
  if (!file) throw new Error(native.claudeNoRecord());
  const text = native.claudeRenamedRecord(JSON.parse(await fs.promises.readFile(file, "utf8")), desktopId, cliId, title);
  if (text == null) throw new Error(native.claudeOtherSession());
  const tmp = `${file}.xenonet-${process.pid}.tmp`;
  await fs.promises.writeFile(tmp, text);
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

// Newest page first; `cursor` is the count of messages already shown from the end.
export async function readClaudeSession(rawId, { cursor = null, limit = 120 } = {}) {
  const all = await getSessionMessages(rawId);
  const { start, end, cursor: next } = native.claudePage(all.length, cursor, Number(limit));
  const version = agentSdkVersion();
  const page = all.slice(start, end).map((m) => messages.sample(m, { version, kind: m?.type })).filter(Boolean);
  return { entries: native.claudeEntries(page), cursor: next, total: all.length };
}
