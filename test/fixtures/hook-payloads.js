// Hook events in the shape Claude Code (2.1.292) and Codex send them, as
// their hook references and Codex's hook schema give them, with the ids,
// paths and text made up. A subagent's events come under its session's id,
// and agent_id is what tells them apart from the main thread's (Claude
// Code: "Present only when the hook fires inside a subagent call"; Codex:
// "Subagent hooks use the parent session id"). agent_type alone doesn't:
// Claude Code sends it on the main thread too when the session runs with
// --agent.

export const claude = {
  postToolUse: (session, cwd, more = {}) => ({
    session_id: session,
    transcript_path: `/home/u/.claude/projects/-w-atlas/${session}.jsonl`,
    cwd,
    permission_mode: "default",
    hook_event_name: "PostToolUse",
    tool_name: "Bash",
    tool_input: { command: "npm test", description: "Run the tests" },
    tool_response: { stdout: "# pass 12", stderr: "", interrupted: false, isImage: false },
    tool_use_id: "toolu_01Aa1Bb2Cc3Dd4Ee5Ff6Gg7H",
    ...more,
  }),
  preToolUse: (session, cwd, file, more = {}) => ({
    session_id: session,
    transcript_path: `/home/u/.claude/projects/-w-atlas/${session}.jsonl`,
    cwd,
    permission_mode: "acceptEdits",
    hook_event_name: "PreToolUse",
    tool_name: "Edit",
    tool_input: { file_path: file, old_string: "limit = 10", new_string: "limit = 20", replace_all: false },
    tool_use_id: "toolu_01Hh8Ii9Jj0Kk1Ll2Mm3Nn4O",
    ...more,
  }),
  stop: (session, cwd) => ({
    session_id: session,
    transcript_path: `/home/u/.claude/projects/-w-atlas/${session}.jsonl`,
    cwd,
    permission_mode: "default",
    hook_event_name: "Stop",
    stop_hook_active: false,
    last_assistant_message: "The tests pass.",
  }),
  subagentStart: (session, cwd) => ({
    session_id: session,
    transcript_path: `/home/u/.claude/projects/-w-atlas/${session}.jsonl`,
    cwd,
    hook_event_name: "SubagentStart",
    agent_id: "a0f1e2d3c4b5a6978",
    agent_type: "general-purpose",
  }),
  subagentStop: (session, cwd) => ({
    session_id: session,
    transcript_path: `/home/u/.claude/projects/-w-atlas/${session}.jsonl`,
    cwd,
    permission_mode: "default",
    hook_event_name: "SubagentStop",
    stop_hook_active: false,
    agent_id: "a0f1e2d3c4b5a6978",
    agent_type: "general-purpose",
    agent_transcript_path: `/home/u/.claude/projects/-w-atlas/${session}/subagents/agent-a0f1e2d3c4b5a6978.jsonl`,
    last_assistant_message: "Found three callers.",
  }),
  // What a subagent's tool call adds to any event.
  inSubagent: { agent_id: "a0f1e2d3c4b5a6978", agent_type: "general-purpose" },
  // The main thread of a session started with --agent.
  withAgent: { agent_type: "security-reviewer" },
};

export const codex = {
  postToolUse: (session, cwd, more = {}) => ({
    session_id: session,
    turn_id: "019a7c10-0000-7000-8000-000000000001",
    transcript_path: `/home/u/.codex/sessions/2026/10/10/rollout-2026-10-10T21-58-03-${session}.jsonl`,
    cwd,
    hook_event_name: "PostToolUse",
    model: "gpt-5-codex",
    permission_mode: "default",
    tool_name: "Bash",
    tool_use_id: "call_Q1w2E3r4T5y6U7i8O9p0",
    tool_input: { command: "npm test" },
    tool_response: "# pass 12",
    ...more,
  }),
  userPromptSubmit: (session, cwd, more = {}) => ({
    session_id: session,
    turn_id: "019a7c10-0000-7000-8000-000000000002",
    transcript_path: `/home/u/.codex/sessions/2026/10/10/rollout-2026-10-10T21-58-03-${session}.jsonl`,
    cwd,
    hook_event_name: "UserPromptSubmit",
    model: "gpt-5-codex",
    permission_mode: "default",
    prompt: "Look for every caller of chargeCard and list them.",
    ...more,
  }),
  stop: (session, cwd) => ({
    session_id: session,
    turn_id: "019a7c10-0000-7000-8000-000000000003",
    transcript_path: `/home/u/.codex/sessions/2026/10/10/rollout-2026-10-10T21-58-03-${session}.jsonl`,
    cwd,
    hook_event_name: "Stop",
    model: "gpt-5-codex",
    permission_mode: "default",
    stop_hook_active: false,
    last_assistant_message: "Done.",
  }),
  inSubagent: { agent_id: "019a7c11-0000-7000-8000-0000000000a1", agent_type: "explorer" },
};
