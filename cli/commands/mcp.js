// xenonet mcp: the xenonet MCP server on stdio (mcp/xenonet.js). Claude Code and
// Codex start it once your agents are connected. xenonet mcp console is the
// console operator (mcp/console.js), and xenonet mcp assistant the read-only
// server for a remote actor you paired (mcp/assistant.js).
export const help = `Runs the xenonet MCP server on stdio: the engine's tools (whoami, peers,
who_has, send, inbox, claim, release, focus) for Claude Code and Codex. They
start it themselves once you connect them (xenonet connect).

An actor you allowed in XenoNet can start it too, with its token in
XENONET_ACTOR_TOKEN_FILE: it then gets actor_heartbeat, actor_events and
actor_targets as well, and calls the engine as itself.

  xenonet mcp console                 the console operator's tools
  xenonet mcp assistant --grant <id> [--port <n>]
                                    the six read-only tools for a remote
                                    actor you paired in XenoNet's window; a
                                    tunnel client runs it
  xenonet mcp assistant --fixture     the same tools over made-up data, with
                                    no XenoNet and no grant`;

export async function run(args = [], flags = {}) {
  if (args[0] === "console") { await import("../../mcp/console.js"); return; }
  if (args[0] === "assistant") {
    if (args.length > 1) throw new Error("xenonet mcp assistant takes --grant <id> (and --port <n>), or --fixture.");
    const port = flags.port ?? (flags.dev ? 47842 : flags.demo ? 47841 : null);
    const { serve } = await import("../../mcp/assistant.js");
    await serve({ grant: typeof flags.grant === "string" ? flags.grant : null, fixture: flags.fixture === true, port });
    return;
  }
  if (args.length) throw new Error("Use xenonet mcp, xenonet mcp console or xenonet mcp assistant.");
  const { serve } = await import("../../mcp/xenonet.js");
  await serve();
}
