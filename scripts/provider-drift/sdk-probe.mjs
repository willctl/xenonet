// Runs the Agent SDK the way XenoNet does, in a made-up home (sandbox.mjs
// sets this process's environment), and prints what it returned as JSON:
//
//   node sdk-probe.mjs <the SDK's entry, as a file URL> <project folder> <model>
//
// It has the SDK's own Claude Code run one turn against the stand-in API
// (ANTHROPIC_BASE_URL), so there's a real transcript to read; then lists the
// sessions, reads the messages, renames the session and lists again, as
// core/claude.js and XenoNet's renaming do. Each step says what went wrong
// rather than stopping the rest.
const [entry, project, model] = process.argv.slice(2);
const out = { exports: [], steps: {} };
const step = async (name, fn) => {
  try {
    out.steps[name] = { ok: true, value: await fn() };
  } catch (err) {
    out.steps[name] = { ok: false, error: String(err?.message ?? err).slice(0, 500) };
  }
  return out.steps[name];
};

const sdk = await import(entry);
out.exports = Object.keys(sdk)
  .filter((k) => typeof sdk[k] === "function")
  .sort();

let sessionId = null;
await step("query", async () => {
  const messages = [];
  const q = sdk.query({
    prompt: "Check the plan.",
    options: {
      cwd: project,
      model,
      maxTurns: 4,
      settingSources: [],
      // No CLAUDE.md found above the folder, no hooks, no keychain.
      extraArgs: { bare: null, "strict-mcp-config": null },
      env: { ...process.env },
    },
  });
  for await (const m of q) {
    messages.push(m.type + (m.subtype ? `:${m.subtype}` : ""));
    if (m.session_id) sessionId = m.session_id;
  }
  return { messages, sessionId };
});

const listing = await step("listSessions", () => sdk.listSessions());
sessionId ??= listing.value?.[0]?.sessionId ?? null;
if (sessionId) {
  await step("getSessionMessages", () => sdk.getSessionMessages(sessionId));
  if (typeof sdk.getSessionInfo === "function") await step("getSessionInfo", () => sdk.getSessionInfo(sessionId));
  if (typeof sdk.renameSession === "function") {
    await step("renameSession", async () => {
      await sdk.renameSession(sessionId, "Renamed by the drift check");
      return null;
    });
    await step("listSessionsAfterRename", () => sdk.listSessions());
  }
}
out.sessionId = sessionId;
process.stdout.write(JSON.stringify(out));
