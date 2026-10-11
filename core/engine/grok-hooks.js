// Grok Build's hook events as the engine reads hooks (engine.js's hook()).
// Grok sends its own envelope: camelCase keys (sessionId, toolName,
// toolInput, stopHookActive), hookEventName in its snake_case
// (post_tool_use) and hook_event_name in Claude Code's PascalCase
// (PostToolUse). What it does with an answer differs from Claude Code's
// too, which the engine follows (docs/design/grok-build.md):
//
//   SessionStart       stdout ignored: nothing to say then
//   UserPromptSubmit   additionalContext discarded: nothing to say then
//   PreToolUse         additionalContext arrives with the call's result
//   PostToolUse(Failure) additionalContext arrives with the result
//   Stop               decision "block" and a reason keep the turn going,
//                      the reason given to the model as your message; a
//                      Stop also fires when the session closes (reason
//                      channel_closed or shutdown), whose answer is ignored
//
// Events inside a subagent's own session carry subagentType.

// A PascalCase event name from Grok's snake_case one (post_tool_use →
// PostToolUse), for an envelope without Claude Code's spelling.
const pascal = (name) =>
  String(name ?? "")
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join("");

// The event in the engine's terms: { event, session_id, cwd, tool_name,
// tool_input, stop_hook_active, reason, subagent, notification, message }.
// `env` is the hook process's, where Grok puts GROK_SESSION_ID and
// GROK_HOOK_EVENT, for an envelope that doesn't say them.
export function grokHookInput(body = {}, env = {}) {
  const b = body && typeof body === "object" ? body : {};
  const grokName = b.hookEventName ?? env.GROK_HOOK_EVENT;
  const event = typeof b.hook_event_name === "string" && /^[A-Z]/.test(b.hook_event_name) ? b.hook_event_name : grokName ? pascal(grokName) : null;
  const id = [b.sessionId, b.session_id, env.GROK_SESSION_ID].find((x) => typeof x === "string" && x);
  return {
    event,
    session_id: id ?? null,
    cwd: typeof b.cwd === "string" ? b.cwd : typeof b.workspaceRoot === "string" ? b.workspaceRoot : null,
    tool_name: b.toolName ?? b.tool_name ?? null,
    tool_input: b.toolInput ?? b.tool_input ?? null,
    stop_hook_active: b.stopHookActive === true || b.stop_hook_active === true,
    reason: typeof b.reason === "string" ? b.reason : null,
    subagent: typeof b.subagentType === "string" && b.subagentType ? b.subagentType : null,
    notification: b.notificationType ?? b.notification_type ?? null,
    message: typeof b.message === "string" ? b.message : null,
  };
}

// Whether a hook's event came from Grok Build: its envelope, which carries
// Grok's own spelling of the event and its workspace root, or the hook
// process's environment, where Grok names the event. Grok reads Claude
// Code's settings too (~/.claude/settings.json), so XenoNet's Claude Code
// hooks run in its sessions; those pass them over, since Grok's own
// (~/.grok/hooks/xenonet.json) speak for them.
export const fromGrok = (body = {}, env = {}) => (!!body && typeof body === "object" && (typeof body.hookEventName === "string" || typeof body.workspaceRoot === "string")) || typeof env.GROK_HOOK_EVENT === "string";
