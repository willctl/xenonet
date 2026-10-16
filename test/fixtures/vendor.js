// Made-up vendor records, one source at a time: ones that fit XenoNet's
// schemas for them (`fits`), and odd ones, each with the drift it should
// show (`drift`: [kind, path]). Nothing here comes from anyone's data.
const T = "2026-10-05T12:00:00.000Z";
const SESSION = "0190d3e0-0000-7000-8000-000000000001";
const HERMES_SESSION = "20261005_120000_a1b2c3";

const usage = { input_tokens: 12, cache_creation_input_tokens: 300, cache_read_input_tokens: 40000, cache_creation: { ephemeral_5m_input_tokens: 200, ephemeral_1h_input_tokens: 100 }, output_tokens: 150, service_tier: "standard" };
const assistant = (content, extra = {}) => ({ type: "assistant", timestamp: T, version: "2.1.280", sessionId: SESSION, uuid: "u-2", isSidechain: false, message: { id: "msg_01", type: "message", role: "assistant", model: "claude-opus-5-5", stop_reason: "tool_use", stop_sequence: null, usage, content }, ...extra });
const user = (content, extra = {}) => ({ type: "user", timestamp: T, version: "2.1.280", sessionId: SESSION, uuid: "u-1", isSidechain: false, cwd: "/Users/ada/atlas", message: { role: "user", content }, ...extra });

export const VENDOR = {
  "anthropic/claude-code/transcript-line": {
    fits: [
      user("Fix the tokenizer"),
      user([{ type: "text", text: "Look" }, { type: "image", source: { type: "base64" } }], { isMeta: false }),
      user([{ type: "tool_result", tool_use_id: "tu_1", content: "ok", is_error: false }]),
      user([{ type: "tool_result", tool_use_id: "tu_2", content: [{ type: "text", text: "ok" }], cache_control: { type: "ephemeral" } }, { type: "text", text: "and this", cache_control: { type: "ephemeral" } }]),
      assistant([], { message: { id: "msg_02", type: "message", role: "assistant", model: "claude-opus-5-5", stop_reason: "end_turn", stop_sequence: null, stop_details: null, diagnostics: null, usage: { ...usage, inference_geo: "us", iterations: null, output_tokens_details: null }, content: [{ type: "text", text: "See the docs.", citations: null }, { type: "tool_use", id: "tu_3", name: "Read", input: {}, toolset_name: null }] } }),
      user("This session is being continued", { isCompactSummary: true }),
      assistant([{ type: "text", text: "Done." }], { effort: "high" }),
      assistant([{ type: "thinking", thinking: "…", signature: "s" }, { type: "tool_use", id: "tu_1", name: "Bash", input: { command: "npm test" }, caller: { type: "direct" } }]),
      { type: "summary", summary: "An earlier session", leafUuid: "u-9" },
      { type: "system", subtype: "compact_boundary", timestamp: T },
    ],
    odd: [
      { record: { ...user("Hi"), timestamp: undefined }, drift: [["missing-field", "timestamp"]] },
      { record: user({ text: "an object" }), drift: [["type-change", "message.content"]], strip: [["message", "content"]] },
      { record: assistant([{ type: "text", text: 5 }]), drift: [["type-change", "message.content[].text"]], strip: [["message", "content", 0, "text"]] },
      { record: { type: "progress", timestamp: T, data: {} }, drift: [["new-variant", "type"]] },
      { record: assistant([{ type: "text", text: "x", footnotes: [{ type: "web", url: "https://example.com" }] }]), drift: [["new-field", "message.content[].footnotes"]] },
      { record: user([{ type: "tool_result", tool_use_id: "tu_1", content: "ok", seen_by: "x" }]), drift: [["new-field", "message.content[].seen_by"]] },
      { record: user([{ type: "text", text: "Look", tone: "calm" }]), drift: [["new-field", "message.content[].tone"]] },
      { record: assistant([{ type: "text", text: "x" }], { message: { id: "m", model: "m", stop_reason: null, content: [], usage: { ...usage, output_tokens_breakdown: { a: 1 } } } }), drift: [["new-field", "message.usage.output_tokens_breakdown"]] },
      { record: { ...assistant([]), message: { model: "m", content: [], usage, stop_reason: "end_turn" } }, drift: [["missing-field", "message.id"]] },
      { record: { type: 5 }, drift: [["type-change", "type"]], strip: [["type"]] },
      { record: null, drift: [["type-change", null]], drop: true },
      { record: [], drift: [["type-change", null]], drop: true },
    ],
  },
  "anthropic/claude-code/config": {
    fits: [
      { numStartups: 3, oauthAccount: { accountUuid: "4f1c0000-0000-4000-8000-000000000001", organizationUuid: "4f1c0000-0000-4000-8000-000000000002", emailAddress: "ada@example.com", organizationType: "claude_max", organizationName: null, displayName: "Ada" }, mcpServers: { xenonet: { command: "/usr/local/bin/node", args: ["mcp/xenonet.js"], env: { XENONET_PORT: "47840" } } } },
      { projects: {} },
    ],
    odd: [{ record: { oauthAccount: { accountUuid: 7, organizationUuid: "o" } }, drift: [["type-change", "oauthAccount.accountUuid"]], strip: [["oauthAccount", "accountUuid"]] }],
  },
  "anthropic/claude-desktop/session": {
    fits: [
      { sessionId: "local_0190d3e0-0000-7000-9000-000000000001", cliSessionId: SESSION, title: "Fix the tokenizer", originCwd: "/Users/ada/atlas", createdAt: 1791230000000, lastActivityAt: 1791234567000, isArchived: false, model: "claude-opus-5-5", effort: "high", permissionMode: "default", somethingElse: [1] },
      { sessionId: "local_2", cliSessionId: SESSION, createdAt: 1, lastActivityAt: 2, spawnedFrom: { sessionId: "local_1" }, worktreePath: "/Users/ada/atlas-wt", branch: "fix" },
    ],
    odd: [{ record: { sessionId: "local_3", createdAt: "yesterday", lastActivityAt: 2 }, drift: [["missing-field", "cliSessionId"], ["type-change", "createdAt"]], strip: [["createdAt"]] }],
  },
  "anthropic/claude-desktop/plan-usage": {
    fits: [{ version: 2, samples: [{ t: 1791230000000, org: "o", u: { fh: 12, sd: 40 } }, { t: 1791230060000 }] }],
    odd: [{ record: { version: 2, samples: [{ t: 1, u: { fh: 1, sd: 2, op: 3 } }] }, drift: [["new-field", "samples[].u.op"]] }],
  },
  "anthropic/agent-sdk/session-listing": {
    fits: [
      { sessionId: SESSION, summary: "Fix the tokenizer", lastModified: 1791234567000, fileSize: 1200, customTitle: undefined, firstPrompt: "Fix it", gitBranch: "main", cwd: "/Users/ada/atlas", tag: undefined, createdAt: 1791230000000 },
      { sessionId: SESSION, summary: "s", lastModified: 1 },
    ],
    odd: [{ record: { sessionId: SESSION, summary: "s", lastModified: 1, pinned: true }, drift: [["new-field", "pinned"]] }],
  },
  "anthropic/agent-sdk/session-message": {
    fits: [
      { type: "user", uuid: "u-1", session_id: SESSION, parent_tool_use_id: null, parent_agent_id: null, timestamp: T, message: { role: "user", content: "Fix it" } },
      { type: "assistant", uuid: "u-2", session_id: SESSION, parent_tool_use_id: null, parent_agent_id: null, message: { content: [{ type: "text", text: "Done" }, { type: "tool_use", name: "Bash", input: {} }, { type: "tool_result", is_error: true, content: "1 failed" }] } },
      { type: "assistant", uuid: "u-5", message: { content: [{ type: "thinking", thinking: "…", signature: "s" }, { type: "text", text: "Done", citations: null }, { type: "tool_use", id: "tu_1", name: "Bash", input: {}, caller: { type: "direct" } }, { type: "tool_result", tool_use_id: "tu_1", content: "ok", cache_control: { type: "ephemeral" } }] } },
    ],
    odd: [
      { record: { type: "user", uuid: "u-3", message: {}, isReplay: true }, drift: [["new-field", "isReplay"]] },
      { record: { type: "assistant", uuid: "u-4", message: { content: [{ type: "text", text: "Done", tone: "calm" }] } }, drift: [["new-field", "message.content[].tone"]] },
    ],
  },
  "anthropic/oauth/usage": {
    fits: [{ five_hour: { utilization: 10, resets_at: "2026-10-05T14:00:00+00:00" }, seven_day: { utilization: 6.5, resets_at: null }, seven_day_opus: null }, { five_hour: null }],
    odd: [{ record: { five_hour: { utilization: "10%", resets_at: null } }, drift: [["type-change", "five_hour.utilization"]], strip: [["five_hour", "utilization"]] }],
  },
  "openai/codex/rollout-line": {
    fits: [
      { timestamp: T, type: "session_meta", payload: { id: "t1", cwd: "/Users/ada/ledger", cli_version: "0.150.0", originator: "codex_cli_rs", model_provider: "openai" } },
      { timestamp: T, type: "turn_context", payload: { model: "gpt-5-codex", effort: "high", cwd: "/Users/ada/ledger" } },
      { timestamp: T, type: "event_msg", payload: { type: "task_started", model_context_window: 258400 } },
      { timestamp: T, type: "event_msg", payload: { type: "token_count", info: { total_token_usage: { input_tokens: 9000, cached_input_tokens: 8000, output_tokens: 300, reasoning_output_tokens: 100, total_tokens: 9300 }, last_token_usage: { total_tokens: 3000 }, model_context_window: 258400 }, rate_limits: { limit_id: "x", primary: { used_percent: 12, window_minutes: 300, resets_at: 1791240000 }, secondary: null } } },
      { timestamp: T, type: "event_msg", payload: { type: "token_count", info: null, rate_limits: null } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "CommandExecution", id: "i1", command: ["/bin/zsh", "-lc", "make"], exit_code: 0, status: "completed" } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", thread_id: "t1", turn_id: "u1", started_at_ms: 1791230000000, completed_at_ms: 1791230001000, item: { type: "CommandExecution", id: "i3", command: ["/bin/zsh", "-lc", "make test"], exit_code: 1, status: "failed", aggregated_output: "1 failed" } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "Reasoning", id: "i4", summary_text: ["Checking the tests"], raw_content: [] } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "ContextCompaction", id: "i5" } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "McpToolCall", id: "i6", server: "wiki", tool: "read", arguments: { page: "ledger" }, status: "completed", result: { content: [{ type: "text", text: "…" }] } } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "FileChange", changes: { "/Users/ada/ledger/a.py": { type: "update", unified_diff: "@@" } }, status: "completed" } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "FileChange", changes: { "/Users/ada/ledger/b.py": { type: "update", unified_diff: "@@", move_path: "/Users/ada/ledger/c.py" }, "/Users/ada/ledger/d.py": { type: "add", content: "x = 1\n" } } } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "WebSearch", id: "i7", query: "rounding", action: { type: "search", query: "rounding", queries: ["rounding", "banker's rounding"] } } } },
      { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "SubAgentActivity", id: "i2", kind: "completed", agent_thread_id: "t2", agent_path: "explorer" } } },
      { timestamp: T, type: "event_msg", payload: { type: "user_message", message: "Why is it slow?" } },
      { timestamp: T, type: "event_msg", payload: { type: "thread_settings_applied", thread_id: "t1", thread_settings: { model: "gpt-5.6-sol", model_provider_id: "openai", service_tier: "priority", cwd: "/Users/ada/ledger", reasoning_effort: "high" } } },
      { timestamp: T, type: "event_msg", payload: { type: "thread_settings_applied", thread_id: "t1", thread_settings: { model: "gpt-5.6-sol", model_provider_id: "openai", cwd: "/Users/ada/ledger" } } },
      { timestamp: T, type: "response_item", payload: { type: "function_call", call_id: "c1", name: "shell", arguments: "{}" } },
      { timestamp: T, type: "response_item", payload: { type: "function_call", id: "fc_1", call_id: "c2", name: "read", namespace: "files", arguments: "{}" } },
      { timestamp: T, type: "response_item", payload: { type: "custom_tool_call", status: "completed", call_id: "c3", name: "apply_patch", input: "*** Begin Patch" } },
      { timestamp: T, type: "response_item", payload: { type: "custom_tool_call_output", call_id: "c3", name: "apply_patch", output: "Done" } },
      { timestamp: T, type: "response_item", payload: { type: "function_call_output", call_id: "c2", output: { content: "ok" } } },
      { timestamp: T, type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: "Why?" }] } },
      { timestamp: T, type: "response_item", payload: { type: "reasoning", summary: [] } },
      // Newer kinds of response item XenoNet passes over.
      ...["agent_message", "tool_search_call", "tool_search_output", "image_generation_call", "compaction", "configuration_update", "compaction_trigger", "context_compaction"].map((type) => ({ timestamp: T, type: "response_item", payload: { type, id: "r1" } })),
      { timestamp: T, type: "compacted", payload: { message: "earlier turns" } },
    ],
    odd: [
      { record: { timestamp: T, type: "session_meta", payload: { id: "t1" } }, drift: [["missing-field", "payload.cli_version"]] },
      { record: { timestamp: T, type: "event_msg", payload: { type: "token_count", info: null, rate_limits: { primary: { used_percent: 12, window_minutes: 300, resets_at: "soon" } } } }, drift: [["type-change", "payload.rate_limits.primary.resets_at"]], strip: [["payload", "rate_limits", "primary", "resets_at"]] },
      { record: { timestamp: T, type: "event_msg", payload: { type: "token_count", info: null, rate_limits: { primary: { used_percent: 1, resets_in_minutes: 4 } } } }, drift: [["new-field", "payload.rate_limits.primary.resets_in_minutes"]] },
      { record: { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "Hologram" } } }, drift: [["new-variant", "payload.item.type"]] },
      { record: { timestamp: T, type: "world_view", payload: {} }, drift: [["new-variant", "type"]] },
      { record: { timestamp: T, type: "turn_context", payload: { model: "m" }, made_up: 7 }, drift: [["new-field", "made_up"]] },
      { record: { timestamp: T, type: "event_msg", payload: { type: "item_completed", item: { type: "FileChange", changes: { "/Users/ada/ledger/a.py": { type: "update", unified_diff: "@@", mode: 420 } } } } }, drift: [["new-field", "payload.item.changes.*.mode"]] },
      { record: { timestamp: T, type: "response_item", payload: { type: "function_call", call_id: "c1", name: "shell", arguments: "{}", weight: 2 } }, drift: [["new-field", "payload.weight"]] },
    ],
  },
  "openai/codex/app-server": {
    fits: [
      { initialize: { userAgent: "xenonet/0.150.0 (Mac OS 26.0.0; arm64) unknown (xenonet; 0.4.0)", codexHome: "/Users/ada/.codex" } },
      {
        "thread/list": {
          data: [
            { id: "t1", name: null, preview: "Why is it slow?", cwd: "/Users/ada/ledger", createdAt: 1791230000, updatedAt: 1791234567, recencyAt: 1791234567, status: { type: "notLoaded" }, model: "gpt-5-codex", modelProvider: "openai", reasoningEffort: "high", gitInfo: { sha: "abc", branch: "main", originUrl: "git@github.com:ada/ledger.git" }, source: "vscode", originator: "codex_vscode", projectId: null, cliVersion: "0.150.0" },
            { id: "t2", source: { subAgent: { thread_spawn: { parent_thread_id: "t1", depth: 1, agent_nickname: "Scout" } } }, status: "idle", gitInfo: null },
            { id: "t3", source: { subAgent: "review" }, createdAt: "2026-10-05T12:00:00Z" },
          ],
          nextCursor: null,
        },
      },
      { "thread/items/list": { data: [{ item: { type: "agentMessage", id: "i1", text: "Done" }, startedAtMs: 1, completedAtMs: 2 }, { item: { type: "fileChange", changes: [{ path: "a.py", kind: { type: "update" } }], status: "completed" } }], nextCursor: "c" } },
      { "thread/items/list": { data: [{ item: { type: "fileChange", id: "i2", changes: [{ path: "b.py", kind: { type: "update", move_path: "c.py" }, diff: "@@" }, { path: "d.py", kind: { type: "add" }, diff: "+x" }], status: "completed" } }], nextCursor: null } },
    ],
    odd: [
      { record: { "thread/list": { data: [{ id: "t1", gitInfo: { branch: "main", originUrl: "x", commit: "y" } }] } }, drift: [["new-field", '["thread/list"].data[].gitInfo.commit']] },
      { record: { "thread/list": { data: [{ id: 42 }] } }, drift: [["type-change", '["thread/list"].data[].id']], strip: [["thread/list", "data", 0, "id"]] },
      { record: { "thread/items/list": { data: [{ item: { type: "hologram", id: "i" } }] } }, drift: [["new-variant", '["thread/items/list"].data[].item.type']] },
      { record: { "thread/items/list": { data: [{ item: { type: "fileChange", changes: [{ path: "a.py", kind: { type: "update", mode: 420 }, diff: "@@", stat: 1 }] } }] } }, drift: [["new-field", '["thread/items/list"].data[].item.changes[].kind.mode'], ["new-field", '["thread/items/list"].data[].item.changes[].stat']] },
    ],
  },
  "openai/codex/state-db": {
    fits: [{ projects: [{ id: "p1", name: "Ledger" }], project_roots: [{ project_id: "p1", path: "/Users/ada/ledger" }] }],
    odd: [{ record: { projects: [{ id: "p1" }], project_roots: [] }, drift: [["missing-field", "projects[].name"]] }],
  },
  "openai/codex/auth-claims": {
    fits: [{ iss: "https://auth.example.com", email: "ada@example.com", auth_time: 1791230000, "https://api.openai.com/auth": { chatgpt_account_id: "acct-1", chatgpt_plan_type: "plus" } }, {}],
    odd: [{ record: { "https://api.openai.com/auth": { chatgpt_plan_type: 3 } }, drift: [["type-change", '["https://api.openai.com/auth"].chatgpt_plan_type']], strip: [["https://api.openai.com/auth", "chatgpt_plan_type"]] }],
  },
  "github/gh/hosts": {
    fits: [{ "github.com": { git_protocol: "", users: { "ada-dev": null, orbitworks: null }, user: "" } }, { "ghe.example.com": { user: "", oauth_token: "" } }],
    odd: [{ record: { "github.com": { users: ["ada-dev"] } }, drift: [["type-change", "*.users"]], strip: [["github.com", "users"]] }],
  },
  // Hermes Agent's, as its observer reads them (core/actors/hermes-files.js).
  "nous/hermes-agent/state-db": {
    fits: [
      {
        schema_version: [{ version: 31 }],
        sessions: [{ id: HERMES_SESSION, source: "tui", parent_session_id: null, started_at: 1791230000.5, ended_at: null, end_reason: null, cwd: "/Users/ada/atlas", git_repo_root: "/Users/ada/atlas", last_activity_at: 1791230100.25, last_activity_description: "executing tool: terminal", archived: 0, hidden: 0 }],
        messages: [
          { id: 41, session_id: HERMES_SESSION, role: "assistant", tool_name: null, timestamp: 1791230098.5, active: 1, calls: 1 },
          { id: 42, session_id: HERMES_SESSION, role: "tool", tool_name: "terminal", timestamp: 1791230099.1, active: 1 },
        ],
        tool_paths: [{ id: 40, session_id: HERMES_SESSION, timestamp: 1791230098, tool: "write_file", path: "src/app.ts" }, { id: 40, session_id: HERMES_SESSION, timestamp: 1791230098, tool: "patch", path: null }],
        gateway_heartbeats: [{ pid: 4242, started_at: 1791229000.1, last_heartbeat: 1791230090.4, profile: "default" }],
      },
      // The older layout: no activity stamps, no active column, no heartbeats.
      { schema_version: [{ version: 12 }], sessions: [{ id: "20260705_000500_d4e5f6", source: "cli", started_at: 1783200000, ended_at: 1783200300, end_reason: "cli_close", cwd: "/Users/ada" }], messages: [], tool_paths: [], gateway_heartbeats: [] },
    ],
    odd: [
      { record: { schema_version: [{ version: 31 }], sessions: [{ id: "s1", started_at: "yesterday" }], messages: [], tool_paths: [], gateway_heartbeats: [] }, drift: [["type-change", "sessions[].started_at"]], strip: [["sessions", 0, "started_at"]] },
      { record: { schema_version: [{ version: 31 }], sessions: [], messages: [{ id: 1, session_id: "s1", timestamp: 1 }], tool_paths: [], gateway_heartbeats: [] }, drift: [["missing-field", "messages[].role"]] },
      { record: { sessions: [], messages: [], tool_paths: [], gateway_heartbeats: [] }, drift: [["missing-field", "schema_version"]] },
    ],
  },
  "nous/hermes-agent/spawn-ledger": {
    fits: [
      [
        { purpose: "serve", pid: 4242, create_time: 1791229000.12, install: "a1b2c3d4e5f6", spawner_pid: 4100, spawner_create: 1791228990.5, registered_at: 1791229001, host: "127.0.0.1", port: 51234, profile: "default", isolated: false, argv: "hermes serve --host 127.0.0.1 --port 0", hermes_home: "/Users/ada/.hermes" },
        { purpose: "gateway", pid: 4300, create_time: null, spawner_pid: null, profile: "" },
      ],
      [],
    ],
    odd: [
      { record: [{ purpose: "watchdog", pid: 1 }], drift: [["new-variant", "[].purpose"]] },
      { record: [{ purpose: "serve", pid: "4242" }], drift: [["type-change", "[].pid"]], strip: [[0, "pid"]] },
    ],
  },
  "nous/hermes-agent/gateway-state": {
    fits: [
      { gateway_state: "running", kind: "hermes-gateway", pid: 4300, start_time: 179122900012, updated_at: "2026-10-05T12:00:00.000000+00:00", exit_reason: null, active_agents: 1, served_profiles: ["default"], code_version: "0.21.5", platforms: { telegram: { state: "connected", needs_attention: false, updated_at: "2026-10-05T12:00:00+00:00", error_message: null } }, argv: ["hermes", "gateway", "run"], hermes_home: "/Users/ada/.hermes" },
      { gateway_state: "stopped", pid: 4300, updated_at: 1791230000.5 },
    ],
    odd: [
      { record: { gateway_state: "hibernating", pid: 1 }, drift: [["new-variant", "gateway_state"]] },
      { record: { gateway_state: "running", active_agents: "two" }, drift: [["type-change", "active_agents"]], strip: [["active_agents"]] },
      { record: { pid: 4300 }, drift: [["missing-field", "gateway_state"]] },
    ],
  },
  "nous/hermes-agent/processes": {
    fits: [[{ session_id: "proc_1", command: "codex exec 'tidy the imports'", pid: 5100, pid_scope: "host", host_start_time: 179123000000, cwd: "/Users/ada/atlas", started_at: 1791230000.2, task_id: "t1", owner_task_id: "t1", session_key: "k1", parent_session_id: HERMES_SESSION, watch_patterns: [] }], []],
    odd: [
      { record: [{ pid: "5100" }], drift: [["type-change", "[].pid"]], strip: [[0, "pid"]] },
      { record: [{ cwd: "/Users/ada/atlas" }], drift: [["missing-field", "[].pid"]] },
    ],
  },
  "nous/hermes-agent/cron-jobs": {
    fits: [
      {
        jobs: [
          { id: "j1", name: "Nightly triage", prompt: "Triage the new issues", last_status: "ok", state: "scheduled", enabled: true, schedule: { kind: "cron", expr: "0 3 * * *" }, next_run_at: "2026-10-06T03:00:00+00:00", last_run_at: "2026-10-05T03:00:00+00:00", workdir: "/Users/ada/atlas" },
          { id: "j2", last_status: null, state: "paused", enabled: false, schedule: { kind: "once" }, next_run_at: null, last_run_at: null, workdir: null },
        ],
        updated_at: "2026-10-05T03:00:01+00:00",
      },
      { jobs: [] },
    ],
    odd: [
      { record: { jobs: [{ id: "j3", last_status: "timeout" }] }, drift: [["new-variant", "jobs[].last_status"]] },
      { record: { jobs: [{ id: "j4", last_status: "ok", enabled: "yes" }] }, drift: [["type-change", "jobs[].enabled"]], strip: [["jobs", 0, "enabled"]] },
      { record: { jobs: [{ id: "j5" }] }, drift: [["missing-field", "jobs[].last_status"]] },
    ],
  },
  "nous/hermes-agent/cron-ticker": {
    fits: [{ heartbeat: { at: 1791230000.5, pid: 4300 }, last_success: { at: 1791230000.5 } }, { heartbeat: null, last_success: null }],
    odd: [{ record: { heartbeat: { at: "now", pid: 4300 } }, drift: [["type-change", "heartbeat.at"]], strip: [["heartbeat", "at"]] }],
  },
  "anomaly/opencode/db": {
    fits: [
      {
        session: [{ id: "ses_eb95b1836ffexD4vYDGuLeJDCy", parent_id: null, directory: "/Users/ada/atlas", title: "Overwrite notes", version: "1.19.3", time_created: 1792186574794, time_updated: 1792186579752, time_archived: null }],
        edits: [{ session_id: "ses_eb95b1836ffexD4vYDGuLeJDCy", time_created: 1792186578050, tool: "write", path: "/Users/ada/atlas/notes.txt" }],
      },
      { session: [], edits: [] },
    ],
    odd: [
      { record: { session: [{ id: "s1", directory: "/a", version: "1.19.3", time_created: "today", time_updated: 1 }], edits: [] }, drift: [["type-change", "session[].time_created"]], strip: [["session", 0, "time_created"]] },
      { record: { session: [], edits: [{ session_id: "s1", time_created: 1, path: "/a" }] }, drift: [["missing-field", "edits[].tool"]] },
    ],
  },
  "google/gemini-cli/chat-record": {
    fits: [
      { sessionId: "8f1d2c3b-0000-4000-8000-000000000001", projectHash: "ab12", startTime: "2026-10-16T12:00:00.000Z", lastUpdated: "2026-10-16T12:00:00.000Z", kind: "main" },
      { id: "m2", timestamp: "2026-10-16T12:00:05.000Z", type: "gemini", content: "", toolCalls: [{ id: "c1", name: "write_file", args: { file_path: "/Users/ada/atlas/notes.txt", content: "x" }, status: "success", timestamp: "2026-10-16T12:00:06.000Z" }] },
      { $set: { lastUpdated: "2026-10-16T12:00:07.000Z" } },
      { $rewindTo: "m2" },
    ],
    odd: [
      { record: { id: "m3", type: "gemini", toolCalls: [{ id: "c2", name: 5 }] }, drift: [["type-change", "toolCalls[].name"]], strip: [["toolCalls", 0, "name"]] },
    ],
  },
  "github/copilot-cli/session-event": {
    fits: [
      { id: "e1", timestamp: "2026-10-16T12:00:00.000Z", parentId: null, type: "session.start", data: { sessionId: "0f3c2b1a-0000-4000-8000-000000000001", version: 1, producer: "copilot-agent", copilotVersion: "1.0.98", startTime: "2026-10-16T12:00:00.000Z", context: { cwd: "/Users/ada/atlas", branch: "main" } } },
      { id: "e2", timestamp: "2026-10-16T12:00:05.000Z", parentId: "e1", type: "tool.execution_start", data: { toolCallId: "t1", toolName: "str_replace_editor", arguments: { command: "str_replace", path: "/Users/ada/atlas/notes.txt" } } },
      { id: "e3", timestamp: "2026-10-16T12:00:09.000Z", parentId: "e2", type: "assistant.message", data: { content: "Done" } },
    ],
    odd: [
      { record: { id: "e4", timestamp: "2026-10-16T12:00:10.000Z", type: "session.start", data: { sessionId: "x", version: "1" } }, drift: [["type-change", "data.version"]], strip: [["data", "version"]] },
      { record: { timestamp: "2026-10-16T12:00:10.000Z", type: "session.idle" }, drift: [["missing-field", "id"]] },
    ],
  },
  "nous/hermes-agent/install-stamp": {
    fits: [{ schemaVersion: 2, commit: "0a1b2c3d4e", commitDate: 1791200000, branch: "main", builtAt: "2026-10-05T12:00:00+00:00", dirty: false, source: "git", distribution: null, updateMechanism: "self", baseVersion: "0.21.5", displayVersion: "0.21.5+12.g0a1b2c3", distance: 12, payload: "bootstrap", tag: null }],
    odd: [
      { record: { baseVersion: 21 }, drift: [["type-change", "baseVersion"]], strip: [["baseVersion"]] },
      { record: { displayVersion: "0.21.5" }, drift: [["missing-field", "baseVersion"]] },
    ],
  },
};
