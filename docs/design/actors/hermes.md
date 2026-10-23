# Hermes Agent as an actor

The Wave 1 deep-dive on `hermes` (Hermes Agent, by Nous Research), so that
XenoNet can model it as an actor (`contracts.md` section 3). It was checked on
2026-10-05 against Hermes's own docs and against the source installed on the
development Mac, a clean clone of upstream `main` at `38d19b92e8`. Code links
point to that commit. The actors interface doc builds on this one. XenoNet's
observer of Hermes was built from both in Wave 2, observed and attributed
only (`core/actors/hermes.js`, `hermes-files.js`, and the vendor schemas
under `nous/hermes-agent/`), and its edited paths count in agents' edit
warnings, marked as observed; the registered mode, through a XenoNet plugin
for Hermes, isn't built. `interface.md` says what the observer does now.

## Summary

- **What's here.** A source install of Hermes Agent 0.21.5 that tracks
  `main`. The Hermes-Setup bootstrap app put it there, and the desktop app
  was built on this Mac. There's no gateway service and no profiles, and no
  hooks, plugins or MCP servers are configured.
- **What runs.** Only Hermes.app, and it's stuck. It's set to use a saved
  remote gateway that it can't reach, and it retries about 29 times a
  minute.
  - Nothing Hermes runs listens on a port.
  - Its session database holds one empty session, from July.
  - So today XenoNet would see an app that's open and doing nothing here.
- **What XenoNet can see without changing anything**, which is a lot:
  - which Hermes processes run, from a spawn ledger that Hermes keeps for
    itself and from the process table;
  - the gateway's heartbeat file and the cron ticker's heartbeat;
  - session metadata and activity stamps in `state.db`;
  - the background processes it started;
  - a public status endpoint on a running desktop backend.
- **Attributing Claude Code and Codex sessions to Hermes** works in three
  ways:
  - by process ancestry, in the common case (`claude -p` and `codex exec`
    run under Hermes's own process);
  - through Hermes's checkpoint of background processes;
  - by Codex's `originator`, for Hermes's optional Codex runtime.

  Sessions it starts inside a detached tmux session, or by driving an app
  with computer use, lose that link. Only registered mode closes the gap.
- **Recommendation.**
  - In Wave 2, build Hermes as observed and attributed only: no installs and
    no changes to its settings.
  - Add registered mode later, through a small XenoNet plugin for Hermes
    (observer hooks, metadata only), with the owner's OK.
  - Don't use Hermes's API server, its outbound webhooks or
    `hermes mcp serve`.

## How this was checked

- **Sources.**
  - The docs at `hermes-agent.nousresearch.com/docs`, which are also in the
    checkout under `website/docs`.
  - The source code, which is the best record of the file formats.
  - The [release notes][releases].
  - Secondary sources are marked where they're all there is.
- **Read-only checks on this Mac:**
  - app bundles, `command -v` and `--version`;
  - `ps`, `lsof`, `launchctl list` and `codesign -dv`;
  - the names, sizes and times of files;
  - `sqlite3 -readonly` for `state.db`'s schema and row counts;
  - in `config.yaml`, the key names and a few named switches, parsed in
    memory without printing anything else;
  - in `desktop.log`, the last lines (boot status) and the timestamps of
    boot failures.
- **Not read:**
  - Hermes's secrets: `.env`, `auth.json`, `pairing/`;
  - the desktop app's connection and token files;
  - any row of `state.db`, and the content of any other log.
- **Side effects.**
  - `hermes --version` wrote `~/.hermes/source-checks/<install id>.json`
    and compiled 64 Python bytecode files in the checkout.
  - Opening `state.db` read-only updated the timestamp of `state.db-shm`,
    as every reader of a WAL database does.
  - Nothing else changed.

## 1. How it installs and runs here

### What's installed

| Part | Where | Version | Notes |
|---|---|---|---|
| CLI and agent | `~/.hermes/hermes-agent`, a git checkout of `main` | Base version 0.21.5 (release v2026.9.24), shown as `0.21.5+7348.g38d19b9`; commit `38d19b92e8` of 5 Oct | `install-stamp.json` says source `git`, payload `bootstrap`. The launchers are `hermes`, `hermes-acp` and `hermes-agent` in `~/.local/bin`. They call `~/.hermes/hermes-agent/.hermes/bin/hermes`, which runs a managed Python 3.14.7 |
| Desktop app | Built at `~/.hermes/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app` | `com.nousresearch.hermes` 0.0.0 (source builds carry no version), Electron 40.10.2, ad-hoc signed | Rebuilt by today's update. 317 MB |
| Setup app | `/Applications/Hermes.app` | `com.nousresearch.hermes.setup` 0.0.1, from 5 Jun, Developer ID signed (team `T2F6S8MF7C`) | The Hermes-Setup bootstrap installer. It "downloads a source installation and builds the desktop app" ([installation][docs-install]). 12 MB |
| Managed tools | `~/.hermes/tools/` | Python 3.14.7, Node 26.7.0, npm 12.0.2, uv 0.12.3, ripgrep 15.2.0, ffmpeg 9.0.1, agent-browser 0.26.0, Chromium 1208, cua-driver 0.21.0 | 935 MB. cua-driver is the computer-use backend. `~/.local/bin/node`, `npm` and `npx` link to an older Node in `~/.hermes/node`, but Homebrew's come first on PATH |
| All of it | `~/.hermes` | | 5.5 GB, of which the checkout is 3.8 GB |

- **Today's update.** The checkout, the tools and the app build all changed
  between about 12:24 and 12:30 today. Hermes.app updates itself: "Built-in
  updates pull the latest agent and rebuild the app in place" (desktop
  README).
- **The July install.** The first install, in July, was 0.18.0 (its
  update-check cache says so), and it created the `state.db` that's here.
- **The latest release.** That's v0.21.5, tagged v2026.9.24 on 24 Sep
  ([releases]). This install tracks `main` ahead of it, so its formats move
  more often than the releases do.

### How each part runs

| Part | Started by | Processes, as `ps` shows them | Listens on | In the spawn ledger as |
|---|---|---|---|---|
| Interactive CLI (TUI) | `hermes` in a terminal | The managed Python (`python3 -I -c <launcher>`), then `node …/ui-tui/dist/entry.js`, then `python3 -m tui_gateway.entry`, which runs the agent | nothing | Not in it. The classic CLI (`cli.py`) registers as `cli`, but the TUI's processes don't register |
| One-shot runs | `hermes -z …`, `hermes chat -q …` | The launcher Python | nothing | |
| Desktop, local mode | Hermes.app | Electron `Hermes` and its helpers, then `hermes serve --host 127.0.0.1 --port 0` in Python. Its environment carries `HERMES_PARENT_PID` and `HERMES_SPAWN=v1:-:serve:<app pid>:<start>` ([`backend-command.ts`][src-backend-command], [`parent-process-identity.ts`][src-parent-identity]) | A random port on 127.0.0.1, announced as `HERMES_BACKEND_READY port=<n>` ([CLI reference][docs-cli]) | `serve`, with host, port and profile |
| Desktop, remote or cloud mode | Hermes.app | Electron only. "Home-relative paths use the gateway's home", and tools run on the remote host ([desktop][docs-desktop]) | nothing | |
| Dashboard | `hermes dashboard` | Python | 127.0.0.1:9119 | `dashboard` |
| Gateway, in the foreground | `hermes gateway run` | Python | 127.0.0.1:8642 for the API server, when it's on | `gateway` |
| Gateway, as a service | `hermes gateway install` adds launchd agent `ai.hermes.gateway` | `osascript` (a JXA wrapper that macOS's Local Network privacy accepts), then `sh`, then `python3 -m hermes_cli.stderr_timestamp`, then `python3 … gateway run --external-supervisor` ([`gateway_launchd.py`][src-launchd]) | the same | `gateway` |
| MCP servers it uses | The agent's own process | Each server runs in its own process group ([`mcp_tool_transport.py`][src-mcp-transport]) | | `mcp-helper` |
| Commands it runs | Its `terminal` tool | `bash -c <command>` in a new session, as a child of the agent's process ([`local.py`][src-local-env]) | | |

### What runs right now

- **Hermes.app**, with one main process and three helpers. It opened at
  12:28 while today's update was running, so it still runs the build from
  before the update.
- **It's in remote mode.** `desktop.log` shows every boot failing with
  "Could not reach the remote Hermes gateway while refreshing its WebSocket
  ticket". That's every boot since the app opened: about 1,300 failures by
  13:20, or 29 a minute.
  - The connection signs in with OAuth: the app keeps an Electron partition
    named `hermes-remote-oauth`.
  - Which host it points at is in `connection.json`, which holds a token
    and wasn't read.
  - `desktop.log` grows by about 2.9 MB an hour. It rotates at 10 MB and
    keeps three old files.
- **So nothing runs locally:**
  - there's no `hermes serve`, no gateway, no CLI or TUI, and no MCP server
    of Hermes's;
  - none of its ports (8642, 9119, or a random one) is open;
  - the app's processes have no network sockets at all.
- **No launchd service.** There's no
  `~/Library/LaunchAgents/ai.hermes.gateway.plist`, and `launchctl list`
  shows only the app's own GUI job.
- **No runtime files:**
  - no `gateway_state.json`, `gateway.pid`, `spawn-ledger.json` or
    `processes.json`;
  - no cron jobs (`cron/jobs.json` doesn't exist), and the cron ticker last
    beat on 5 Jul.
- **`state.db`** holds 1 session and 0 messages, from 5 Jul, in an older
  layout (section 2).
- **`config.yaml`:**
  - It has no `hooks`, `plugins`, `mcp_servers`, `gateway` or `monitoring`
    sections.
  - `display.tool_progress` is `all`, the terminal backend is `local`, and
    the model provider is `auto`.
  - A provider sign-in exists (`auth.json`, from July) but wasn't read.
  - `.env` wasn't read either, so whether the API server is switched on is
    unknown. It can't run without the gateway anyway.
- **Codex.** `~/.codex/config.toml` has no block managed by Hermes, so
  Hermes's Codex runtime has never been switched on.
- **On PATH**: tmux 3.6a, `claude` and `codex`, which is everything the
  coding-agent skills need.

## 2. Where it keeps state

Everything is per profile:

- the default profile's state is in `~/.hermes`;
- each named profile has the same layout in `~/.hermes/profiles/<name>/`;
- `HERMES_HOME` points elsewhere when it's set ([session storage][docs-storage]).

There are no named profiles on this Mac.

| Path (under `~/.hermes`) | What | Format, and how it changes | What XenoNet may read |
|---|---|---|---|
| `state.db`, `-wal`, `-shm` | Sessions, messages, usage, routing, backend heartbeats | SQLite in WAL mode. Many processes write to it, in short `BEGIN IMMEDIATE` transactions, with a passive checkpoint every 50 writes. Columns are added and data migrated when a newer Hermes opens it | Session and message metadata, read-only (below) |
| `gateway_state.json` | The gateway's state and heartbeat | JSON, replaced atomically on every change. `updated_at` is re-stamped every 60 s and counts as stale after 120 s ([messaging][docs-messaging], [`status.py`][src-gw-status]) | Everything except `argv`, `hermes_home`, and each platform's `error_message`, `ingress_url` and `listener_base` |
| `gateway.pid`, `gateway.lock` | The running gateway | JSON written once at start (create-exclusive) and removed at stop, plus a lock file | `pid`, `start_time`, `kind` |
| `spawn-ledger.json` (in the default profile only) | Every long-lived Hermes process of this install, with purpose `cli`, `serve`, `dashboard`, `gateway` or `mcp-helper` | JSON list, mode 0600, replaced atomically. Dead entries are pruned when the next one is added, so check each pid and start time ([`process_identity.py`][src-process-identity]) | Everything except `argv`, which can hold a prompt |
| `processes.json` | Background processes started with `terminal(background=true)` that are still running. It's Hermes's checkpoint for recovering them after a crash | JSON list, replaced atomically whenever one starts, finishes or is killed ([`process_registry.py`][src-process-registry], [`process_registry_checkpoint.py`][src-process-checkpoint]) | `pid`, `host_start_time`, `cwd`, `started_at`, `parent_session_id` and `task_id`. Never `command` |
| `cron/jobs.json` | Scheduled jobs | JSON `{"jobs": [...], "updated_at": …}`, replaced atomically ([cron internals][docs-cron]) | `id`, `state`, `enabled`, `schedule.kind`, `next_run_at`, `last_run_at`, `last_status`, `workdir` |
| `cron/ticker_heartbeat`, `cron/ticker_last_success` | The scheduler's liveness | One line each, `<epoch seconds> <pid>` and `<epoch seconds>`, rewritten on every 60 s tick ([`cron/jobs.py`][src-cron-jobs]) | Both |
| `cron/output/` | What jobs produced | Markdown files | Never |
| `logs/` | `agent.log`, `errors.log`, `gateway.log`, `gui.log`, `desktop.log`, `mcp-stderr.log`, `update.log`, and `tool_calls.log` when it's switched on | Appended, and rotated by renaming to `.1`, `.2` and so on | Not needed (below) |
| `hermes-agent/install-stamp.json` | The build stamp | JSON, rewritten on every update ([`source_stamp.py`][src-source-stamp]) | `baseVersion`, `displayVersion`, `source`, `payload` |
| `config.yaml` | Settings | YAML. It can hold inline keys | Only named switches, and only if they're ever needed |
| `skills/` | Installed skills | Folders of Markdown | Folder names |
| `sessions/sessions.json` | A legacy mirror of the gateway's routing | JSON | Never: chat ids |
| `memories/`, `SOUL.md` | Its memory and persona | Markdown | Never |
| `.env`, `auth.json`, `pairing/`, `mcp-tokens/` | Secrets | | Never |
| `response_store.db`, `checkpoints/` | The API server's stored history, and file snapshots for rollback | | Never |
| `~/Library/Application Support/Hermes/` | The desktop app's Electron data | | Only `backend-ownership.json`'s pid and profile, if ever needed, and never its nonce. Never `connection.json`, `connections.json`, `secure-token-storage.json`, `native-oauth-tokens.json` or `desktop-installation.json`, which Hermes itself treats as credential-bearing ([`hardening.ts`][src-hardening]), nor cookies or storage |

### `state.db`

**On this Mac.** These are the tables and columns that `.schema` shows,
with the file opened read-only:

- `sessions` has 41 columns: `id`, `source`, `user_id`, `session_key`,
  `chat_id`, `chat_type`, `thread_id`, `model`, `model_config`,
  `system_prompt`, `parent_session_id`, `started_at`, `ended_at`,
  `end_reason`, the counts and tokens, `cwd`, `git_branch`,
  `git_repo_root`, billing and cost, `title`, `api_call_count`, handoff and
  compression fields, `rewind_count` and `archived`.
- `messages` has 19 columns: `id`, `session_id`, `role`, `content`,
  `tool_call_id`, `tool_calls`, `tool_name`, `timestamp`, `token_count`,
  `finish_reason`, four reasoning columns, `codex_message_items`,
  `platform_message_id`, `observed`, `active` and `compacted`.
- `state_meta`, `compression_locks` and `schema_version`, plus the FTS5
  tables `messages_fts` and `messages_fts_trigram`, kept in step with
  `messages` by triggers.
- The row counts are: sessions 1, messages 0, state_meta 0,
  compression_locks 0, schema_version 1.

**What the current code writes.** The file here was last written on 5 Jul.
It has no `session_model_usage` table, which v20 added, so it's an older
layout. The current schema is version 31 ([session storage][docs-storage],
`SCHEMA_SQL` in [`hermes_state_common.py`][src-schema]). It adds:

- the tables `system_prompts`, `session_model_usage`, `gateway_routing`,
  `gateway_hygiene_state`, `conversation_generations`,
  `gateway_heartbeats`, `session_turn_leases`, `async_delegations`,
  `delivery_obligations` (created when first needed) and
  `messages_fts_cjk`;
- on `sessions`: `created_source`, `display_name`, `origin_json`,
  `system_prompt_hash`, `title_source`, `last_activity_at`,
  `last_activity_description`, `last_activity_provenance`, `profile_name`,
  `transport_profile`, `auto_archived`, `pinned`, `hidden`,
  `last_read_at`, `tool_names` (a hash of the tool schemas, not a list),
  and more compression fields;
- on `messages`: `effect_disposition`, `api_content`, `display_kind`,
  `display_metadata`, `display_identity`, `display_order`, `message_uid`,
  `absorbed_message_uids`, `tool_call_uids` and `tool_call_uid`.

The first time the new code opens this file, it migrates it. XenoNet has to
read both layouts.

Schema 32 also keeps the metadata columns XenoNet reads. Hermes commit
`5b147ab036d6d19131bff8d20701b5001eab844a` declares it in
`hermes_state_common.py`: it records the additive runtime/admission/worker
structures introduced under 31, with no data migration in
`hermes_state_schema.py`. The observer accepts versions through 32, still
checks the available columns, and reports later versions as drift. Tests
cover incremental reads, edit paths and content exclusion under both 31
and 32.

**The columns XenoNet needs:**

| Table | Column | What XenoNet does with it |
|---|---|---|
| `sessions` | `id` | Usually `YYYYMMDD_HHMMSS_<hex>`, with 6 hex characters for CLI, TUI and desktop sessions and 8 for the gateway's. Some ids are derived instead: `cron_<job>_<time>`, `bg_…`, `api_…`, `run_…`, and UUIDs ([`hermes_state_ids.py`][src-ids]) |
| `sessions` | `source` | `cli`, `oneshot`, `desktop`, `tui`, the messaging platforms, `api-server`, `acp`, `cron`, `batch`, `kanban`, `tool`, `subagent` and more ([sessions][docs-sessions]). An open set |
| `sessions` | `parent_session_id` | Compression continuations and delegated subagents |
| `sessions` | `started_at`, `ended_at`, `end_reason` | Epoch seconds, as floats |
| `sessions` | `cwd`, `git_repo_root`, `git_branch` | Where it works, which maps to a project |
| `sessions` | `message_count`, `tool_call_count`, `api_call_count` | Counts |
| `sessions` | `model`, `profile_name` | |
| `sessions` | `last_activity_at`, `last_activity_description` | A durable activity stamp, current schema only, written at most once a minute per session. The description is a machine phrase of at most 120 characters, from fixed templates such as `executing tool: terminal`, `tool completed: patch (2.1s)` and `waiting for stream response (12s, …)` ([`session_activity.py`][src-session-activity], [`tool_executor.py`][src-tool-executor]) |
| `sessions` | `archived`, `hidden` | Hidden from XenoNet, as from Hermes's own pickers |
| `messages` | `session_id`, `role`, `tool_name`, `timestamp`, `active` | Which tools ran and when. Rows with `active = 0` are archived compaction generations |
| `gateway_heartbeats` | `pid`, `started_at`, `last_heartbeat`, `profile` | Each `hermes serve` or TUI backend refreshes its row every 60 s ([`session_reaper.py`][src-reaper]). Never `host`, which is the machine's name |
| `schema_version` | `version` | Which layout to read |

Hermes's own status endpoint counts a session as active when it has no
`ended_at` and has had activity in the last 300 s
([`status.py`][src-status-router]). XenoNet can use the same rule.

**Reading it safely:**

- Open it with `node:sqlite` read-only, query and close, the way
  `listCodexNativeProjects` reads Codex's database. Never hold a read
  transaction open: it keeps Hermes's checkpoints from finishing, and the
  WAL grows.
- Don't open it with `immutable=1`, which would miss whatever is still in
  the WAL.
- A read-only reader still updates `state.db-shm`, the shared WAL index.
  That's normal and changes no data.
- Name the columns in every query, never `*`, so content never reaches
  XenoNet's memory.
- Watch the size and time of `state.db-wal` to know when to look again.
  Hermes writes in bursts.

### Logs

- **The agent logs.** `agent.log`, `errors.log`, `gateway.log` and
  `gui.log` read `<YYYY-MM-DD HH:MM:SS,mmm> <LEVEL>[ [<session id>]]
  <logger>: <message>`, through a formatter that redacts secrets
  ([`hermes_logging.py`][src-logging]).
  - They rotate at 5 MB × 3 (agent), 2 MB × 2 (errors), 5 MB × 3 (gateway)
    and 10 MB × 5 (gui) ([CLI reference][docs-cli]).
- **`desktop.log`**, written by Electron, reads `<time> [hermes] [<area>]
  <message>` and rotates at 10 MB × 3.
- **`tool_calls.log`** exists only for a gateway with
  `display.tool_progress: log`.
  - Each line is `<YYYY-MM-DD HH:MM:SS>  <tool>: "<argument preview>"`,
    with no session id ([messaging][docs-messaging],
    [`run_turn_runner.py`][src-run-turn-runner]).
  - The Phase 0 notes listed it as a standard log. It isn't: it's opt-in
    and gateway-only.
- **XenoNet doesn't need the logs.** `state.db` holds the same facts as data,
  and log messages can carry content. If XenoNet ever reads them, it reads
  only timestamps, levels, the session tag and fixed markers.

## 3. What XenoNet can observe

### Presence

These are the sources, best first:

1. **The spawn ledger.** Its live entries are the ones whose pid and
   `create_time` match the process table.
   - `gateway` means it's always on.
   - `serve` or `dashboard` means a desktop backend or a dashboard, and
     the entry includes its port.
   - `cli` means someone is at the classic Hermes CLI. The TUI doesn't
     register, so the process table finds it.
2. **The process table:**
   - Hermes.app (`com.nousresearch.hermes`);
   - the launcher, `python3 -I -c` with a script that imports
     `hermes_cli.main`, followed by a subcommand such as `serve`,
     `dashboard` or `gateway run`;
   - the TUI's `node …/ui-tui/dist/entry.js`, and its backend,
     `python3 -m tui_gateway.entry`.
3. **The gateway.**
   - `gateway_state.json` gives `gateway_state` (`starting`, `running`,
     `degraded`, `draining`, `stopped` or `startup_failed`).
   - Its `updated_at` is the heartbeat: every 60 s, stale after 120 s.
   - It also gives `active_agents` and each platform's state.
   - `gateway.pid` says which process it is.
4. **A desktop backend** answers `GET http://127.0.0.1:<port>/api/status`
   without a token, by design without content
   ([`status.py`][src-status-router]):
   - its version;
   - whether the gateway is running, its state and the age of its
     heartbeat;
   - its active agents and active sessions;
   - whether the Nous sign-in is still valid.
5. **The cron ticker.** It's the age of `cron/ticker_heartbeat`. Both the
   gateway and the desktop backend tick every 60 s
   ([`web_server.py`][src-web-server]).
6. **Activity stamps:** the rows of `gateway_heartbeats` and
   `sessions.last_activity_at`.

How that maps onto the contract's `presence`:

| Presence | When |
|---|---|
| `working` | A process that hosts the agent is alive, and a session has had activity in the last two minutes, or `active_agents` is above 0 |
| `idle` | Alive, with nothing recent |
| `away` | The gateway's heartbeat is more than 120 s old while its pid lives (what Hermes itself calls a stale heartbeat), or, once registered, two heartbeats were missed |
| `offline` | No Hermes process, or the gateway says it stopped |

Two more notes on presence:

- **Hermes.app stuck in remote mode**, as now: the app runs, but has no
  backend and fails every boot. That's `idle`, with the status "Hermes.app
  can't reach its gateway". Whether that deserves a presence value of its
  own is an open question.
- **`heartbeatMs` is 60000**, the cadence of every Hermes heartbeat above.

### What it's doing

- **Sessions.** New rows in `state.db`, with their `source`, their cwd and
  repo, their counts, and when and why they ended.
- **Tool calls.**
  - Tool results are `messages` rows with `role = 'tool'`, which carry
    `tool_name` and `timestamp`: what ran and when, without the arguments.
  - The names map onto XenoNet's kinds:
    - `terminal`, `process_manage` and `execute_code` are commands;
    - `write_file` and `patch` are edits;
    - `delegate_task` is a subagent;
    - `browser_*` is browsing;
    - `cronjob_manage` is scheduling;
    - `computer_use` is driving apps.
- **The status line.** It comes from `last_activity_description`, matched
  by its template and put into XenoNet's own words, never shown raw.
- **Subagents.**
  - These are sessions with `source = 'subagent'`.
  - Older rows instead carry a marker,
    `json_extract(model_config, '$._delegate_from')`, which returns only
    that marker.
  - They count as the actor's own work, not as actors of their own.
- **Cron.** Only the fields of `cron/jobs.json` that the table above
  allows.

### What it acts on

- **Projects.** `sessions.cwd`, and `git_repo_root` when it's set, map
  onto XenoNet's project nodes, using the same path keys.
- **Files.** The paths that `write_file` and `patch` touch live only in
  the arguments of tool calls (the `messages.tool_calls` JSON), which are
  part of the conversation. There are three ways to go:
  1. Don't: work at the level of projects only (the default).
  2. Let SQLite extract only the `path` argument with `json_extract`, so
     nothing else leaves the database. That's a privacy decision for the
     owner.
  3. Registered mode.
- **Claude Code and Codex sessions** it starts: next.

### Attributing Claude Code and Codex sessions to Hermes

**How its skills launch them** ([claude-code skill][docs-skill-claude],
[codex skill][docs-skill-codex], both installed in
`~/.hermes/skills/autonomous-ai-agents/` and identical to the bundled
ones):

- **Claude Code, print mode** (the preferred way). The call is
  `terminal(command="claude -p '…' --allowedTools … --max-turns N",
  workdir=…, timeout=…)`, sometimes with `--output-format json`,
  `--resume`, `--continue`, `--bare` or `-w <name>`.
- **Claude Code, interactive.** It runs `tmux new-session -d -s <name>`,
  then `tmux send-keys … 'cd <dir> && claude' Enter`, and reads the screen
  back with `tmux capture-pane`.
- **Codex.** It runs `terminal(command="codex exec '…'", workdir=…,
  pty=true)`, often with `background=true`, and often in a git worktree
  under `~/.hermes/cache/scratch/<name>`. It also runs `codex review`.
- **Hermes's optional Codex runtime** runs `codex app-server` over stdio,
  using `~/.codex` ([Codex runtime][docs-codex-runtime]). It sends
  `initialize` with `clientInfo.name` set to `hermes`
  ([`codex_app_server_session.py`][src-codex-session]).

**The process trees.** The agent host is the TUI's `tui_gateway`, a
desktop `hermes serve`, the gateway, or a one-shot CLI:

```
foreground run   agent host (Python) → bash -c "claude -p …" → claude
background run   agent host (Python) → bash -c "codex exec …" → codex   (the background process is in processes.json)
tmux             tmux server (child of launchd) → shell → claude        (no link to Hermes)
Codex runtime    agent host (Python) → codex app-server                 (the thread's originator is "hermes")
```

**What marks them:**

| Signal | Covers | Strength | Notes |
|---|---|---|---|
| A Hermes agent host among the session process's ancestors | Foreground and background runs, while both processes live | Strong | XenoNet already ties sessions to their processes when its hooks are connected: Claude Code's own HTTP hooks pin each session to the process that sends them, and Codex's command hooks run under its process (`core/engine/verify.js`). `claude --bare` loads no hooks ([CLI reference][cc-cli]), so those runs fall back to the signals below |
| An entry in `processes.json`: `pid`, `host_start_time`, `cwd`, `parent_session_id` | Background runs | Strong, and it names the Hermes session | The `claude` or `codex` process descends from the recorded pid. Entries go when the process ends, so XenoNet keeps the link from when it first saw it |
| A Codex `originator` of `hermes` | Hermes's Codex runtime | Strong, once verified | Codex sets the originator from `clientInfo.name` ([app-server docs][codex-app-server], [PR #8988][codex-pr]). The rollouts here say `codex_exec` and `codex-tui`, so `hermes` would stand out. Not seen here, since the runtime was never used: it needs a test thread to verify |
| A cwd under `~/.hermes/cache/scratch/` | The codex skill's worktrees | Medium | A convention, not a guarantee |
| A Hermes tool call in the same cwd, seconds before | Anything its terminal starts | Medium | Needs the session's cwd and the tool timestamps. The `workdir` argument itself is content (above). Not enough on its own to draw a beam |
| Environment variables: `HERMES_HOME`, `HERMES_SESSION_ID`, `HERMES_SESSION_SOURCE` | Anything its terminal starts, and tmux sessions whose server Hermes started | Strong | Terminal children inherit `HERMES_HOME` and the session's `HERMES_SESSION_*` variables ([`local.py`][src-local-env], [`session_context.py`][src-session-context]). But reading another process's environment exposes its secrets and, for gateway sessions, chat and user names, so XenoNet doesn't |
| Claude Code's `entrypoint` | `claude -p` against an interactive run | Weak | It tells a headless run from an interactive one, not who started it. The transcripts here say `claude-desktop` and `cli` |

**What stays unattributed without registered mode:**

- sessions it runs inside tmux: the tmux server doesn't run under Hermes,
  and the only link left, the environment, is off limits;
- sessions started by driving an app's window (computer use);
- runs of `claude --no-session-persistence`, which leave no transcript at
  all.

**The ladder for Wave 2:**

1. A Codex thread whose originator is `hermes`.
2. A session process with a Hermes agent host among its ancestors. When
   the pid is in `processes.json`, that entry also names the Hermes
   session.
3. A cwd under `~/.hermes/cache/scratch/`.
4. Otherwise, no attribution.

**Telling a Hermes process apart.**

- A Hermes process is one of these:
  - a Python under `~/.hermes/tools/python-*/bin/` or
    `~/.hermes/hermes-agent/venv/bin/`, or inside a Hermes.app bundle,
    whose command line names `hermes_cli`, `tui_gateway.entry`,
    `run_agent` or the launcher script;
  - `node` running `…/ui-tui/dist/entry.js` or
    `…/hermes_cli/tui_dist/entry.js`, which is the TUI, and its Python
    child is the host.
- The spawn ledger confirms the long-lived ones by pid and start time.
- No code signature can vouch for them:
  - the managed Python is ad-hoc signed
    (`com.nousresearch.hermes.managed-python`), and so is a Hermes.app
    built from source;
  - whether a bundled release's Python carries the Developer ID signature
    is unverified.

### The local APIs

| API | Where | Auth | What XenoNet uses |
|---|---|---|---|
| `hermes serve` and `hermes dashboard` | 127.0.0.1, on a random port for the desktop (it's in the ledger) or 9119 for the dashboard | `/api/health` and `/api/status` are public ([`public_paths.py`][src-public-paths]). Everything else needs a token made at each start (`X-Hermes-Session-Token`), which only its client holds: the desktop app, or the dashboard's page | The two public routes, for presence and status. Never the token |
| The API server, part of the gateway | 127.0.0.1:8642 by default. It's off unless `API_SERVER_ENABLED=true` and `API_SERVER_KEY` are set and the gateway runs | `/health` is public. Everything else needs the bearer `API_SERVER_KEY` | `/health` only. The key gives "full access to hermes-agent's toolset, including terminal commands" ([API server][docs-api]), and XenoNet shouldn't hold it |
| `hermes mcp serve` | A stdio MCP server | Whoever launches it | Never. It lists and reads conversations, sends messages and answers approvals ([`mcp_serve.py`][src-mcp-serve]) |
| `hermes acp` | A stdio server for editors | | Not relevant |
| The OTLP export | The gateway pushes to a configured collector | | It's content-free gateway and cron health ([monitoring][docs-monitoring]). An option if XenoNet ever takes OTLP |

### Hooks

Hermes has four hook systems ([event hooks][docs-hooks]). None is
configured here.

| System | Where it's declared | Runs in | Consent | Notes |
|---|---|---|---|---|
| Gateway hooks | `~/.hermes/hooks/<name>/HOOK.yaml` and `handler.py` | The gateway only | Placing the files is the consent | Its events are `gateway:startup`, `session:*`, `agent:start`, `agent:step`, `agent:end`, `command:*` and `reaction:*`. `agent:start` and `agent:end` carry the message and the response, cut to 500 characters |
| Plugin hooks | `register(ctx)` in `~/.hermes/plugins/<name>/` | CLI, TUI, desktop backend, gateway. The docs' table says the CLI and the gateway, but plugins load wherever the tool registry does ([`model_tools.py`][src-model-tools]), which is every process that runs the agent | Off until the plugin's name is in `plugins.enabled` ([plugins][docs-plugins]) | The observer contract `hermes.observer.v1` ([observer hooks][docs-observer]). The callbacks run in Hermes's process, fail open, and are bounded by `plugins.hook_callback_timeout` (30 s) |
| Shell hooks | A `hooks:` block in `config.yaml` | CLI, gateway, desktop, TUI, dashboard chat | A prompt on first use of each event and command, recorded in `~/.hermes/shell-hooks-allowlist.json`, or `hooks_auto_accept` | A subprocess per event, which the agent waits for, with a timeout ([`shell_hooks.py`][src-shell-hooks]). Its stdin carries `tool_input`, and for some events the conversation |
| Outbound webhooks | `hooks.outbound` in `config.yaml` | CLI, gateway | None | Signed JSON POSTs from a background thread. The payloads "include tool inputs" |

## 4. How it acts

- **Tools.**
  - The CLI's default toolset is `hermes-cli`
    ([toolsets][docs-toolsets]):
    - files (`read_file`, `write_file`, `patch`, `search_files`);
    - `terminal` and `process_manage`;
    - the web and a browser (agent-browser and Chromium are installed);
    - `execute_code` and `delegate_task`;
    - `cronjob_manage`;
    - memory, skills, session search, todos, vision, image generation,
      speech and `clarify`;
    - `computer_use` (cua-driver is installed).
  - The terminal runs locally as the owner, behind Hermes's own approval
    prompts for dangerous commands.
  - There's "deliberately no agent-callable send_message tool"
    ([`toolsets.py`][src-toolsets]).
- **Claude Code and Codex.** Through its terminal and the skills above.
  The skills for OpenCode and computer use are installed too.
- **Scheduling:**
  - Cron jobs, fired by the gateway's ticker or by the desktop backend's
    ticker every 60 s. With the CLI alone, jobs fire only while a CLI
    session runs, or on `hermes cron tick` ([cron internals][docs-cron]).
  - `/heartbeat`, a recurring prompt in one session. Its state is in
    `state_meta` under `heartbeat:<session id>`, and that's content
    ([heartbeats][docs-heartbeat]).
  - Kanban: a board that spans profiles, with dispatcher workers
    (`source = 'kanban'`).
  - Delegation: `delegate_task` subagents, run inside its own process.
- **Messaging.**
  - The gateway serves Telegram, Discord, Slack, WhatsApp, Signal,
    Matrix, Mattermost, email, SMS, iMessage (through BlueBubbles), Teams
    and more ([messaging][docs-messaging]). None is set up here.
  - It messages people by replying through the gateway, and by delivering
    cron results.
  - `hermes send` sends a one-off message with no agent behind it.
  - `hermes peer` DMs other Hermes gateways' agents.
- **Signing in** (mechanisms only):
  - **Models:**
    - Nous Portal OAuth (`hermes setup --portal`): one subscription for
      300+ models and Nous's Tool Gateway ([Nous Portal][docs-portal]);
    - or provider OAuth (`hermes auth add anthropic` or `openai-codex`,
      with `--type oauth`);
    - or API keys (`hermes auth add <provider> --api-key …`, or in
      `.env`).
    - Credential pools rotate keys. Secrets can come from Bitwarden or
      1Password.
    - Tokens are kept in `auth.json`, keys in `.env`.
  - **The desktop's remote connections** are OAuth or a token, encrypted
    with macOS safeStorage in Electron's data folder.
  - **Messaging**: each platform's bot token or pairing, in `.env` and
    `pairing/`.
  - **MCP servers**: OAuth tokens in `mcp-tokens/`.
- **Cost.**
  - The software is MIT-licensed and free. Models cost per use, or a Nous
    Portal plan pays for them.
  - A secondary source gives the plans as Free at $0, Plus at $20, Super
    at $100 and Ultra at $200 a month ([Hostinger][pricing-secondary]).
    The official page ([Nous Portal plans][pricing-official]) refused our
    fetch.

## 5. Hermes as an actor

- **Ids.** `actor:hermes` is the default profile, and `actor:hermes-<profile>`
  each named profile. Each profile is an isolated Hermes, with its own
  state, gateway and sign-ins.
- **The rest:**
  - `name` is "Hermes", `kind` is `agent`, and `host` is null;
  - `product` is `{"id": "hermes-agent", "vendor": "Nous Research",
    "version": <baseVersion>}`.

### The snapshot

An example, with made-up values:

```json
{
  "schema": "actor/snapshot/v1",
  "id": "actor:hermes",
  "kind": "agent",
  "name": "Hermes",
  "product": { "id": "hermes-agent", "vendor": "Nous Research", "version": "0.21.5" },
  "host": null,
  "via": ["observed", "attributed"],
  "presence": "working",
  "presenceSince": 1791230000000,
  "lastSeen": 1791234567000,
  "heartbeatMs": 60000,
  "status": "Running a command in atlas-api",
  "capabilities": ["run-commands", "edit-repos", "spawn-sessions", "schedule", "browse"],
  "targets": [
    { "id": "project:/users/will/projects/atlas-api", "relation": "works-on", "since": 1791231000000, "lastAt": 1791234500000 },
    { "id": "claude:6c1f0000-0000-4000-8000-000000000001", "relation": "spawned", "since": 1791233000000, "lastAt": 1791234400000 }
  ],
  "recent": [],
  "counts": { "events24h": 42, "spawned24h": 2, "messages24h": 0 },
  "identity": null
}
```

### Where each part comes from

| Field | From | `via` |
|---|---|---|
| `presence`, `presenceSince`, `lastSeen` | The spawn ledger and the process table, `gateway_state.json`, the cron ticker, `/api/status`, activity stamps | observed. Registered heartbeats later |
| `heartbeatMs` | 60000 | observed |
| `status` | `last_activity_description` matched by its template, the gateway's state, the desktop's boot state, the next cron job | observed. The plugin later |
| `capabilities` | What's installed and set up (next list) | observed |
| `targets` with `works-on` | The cwd and repo of recent sessions | observed |
| `targets` with `watches` | Cron jobs' `workdir` | observed |
| `targets` with `spawned` | Ancestry, `processes.json`, the Codex originator, scratch worktrees | attributed |
| `targets` with `edited` | The plugin's file paths, or `json_extract` if the owner allows it | registered, or observed |
| `recent`, `counts` | The events below. `messages24h` counts the assistant's rows in sessions from messaging platforms, without reading them | observed and attributed |
| `identity` | null. The account Hermes acts as is in `auth.json`, which XenoNet never reads. The sessions it starts carry their own identities in XenoNet's graph | none |

**Capabilities:**

- `run-commands`, `edit-repos` and `browse`: its toolset has the terminal,
  file and browser tools.
- `spawn-sessions`: the claude-code and codex skills are installed, and
  `claude`, `codex` and `tmux` are on PATH.
- `schedule`: it has the cron tool, which is live while a ticker runs.
- `message-people`: only while a gateway platform is connected.
- `message-agents`: only with `hermes peer` or A2A set up.
- Computer use isn't in the contract's list; this doc proposes adding
  `use-computer`.

### Events

| `kind` | When | `via` | `detail`, for example |
|---|---|---|---|
| `presence` | A ledger entry appears or goes, `gateway_state` changes, the desktop's boot fails or recovers | observed | "Gateway started", "Desktop backend stopped", "Hermes.app can't reach its gateway" |
| `status` | The status line changes | observed | |
| `spawned` | A Claude Code or Codex session is attributed to it | attributed | "Started a Claude Code session in atlas-api" |
| `ran` | Tool rows for `terminal`, `process_manage` or `execute_code`, gathered up per minute | observed | "Ran 3 commands in atlas-api" |
| `edited` | Tool rows for `write_file` or `patch` (a count), or the plugin's paths | observed, registered | "Edited 2 files in atlas-api" |
| `messaged` | An assistant row in a session from a messaging platform | observed | "Replied on Telegram", with the platform's name only |
| `said` | Its messages to agents, through `hermes peer` or XenoNet's engine | registered | |
| `finished` | A session ends, or a cron job's `last_status` turns `ok` | observed | "Finished a session in atlas-api", "A scheduled job ran" |
| `failed` | A cron job turns `error`, `delivery_failed` or `blocked_config`, the gateway's start fails, or a session ends in an error | observed | "A scheduled job failed" |

### The vendor schemas XenoNet needs

| Schema | Source | Key fields | Notes |
|---|---|---|---|
| `vendor/nous/hermes-agent/state-db` | `state.db`, per profile | `schema_version.version`; the `sessions`, `messages` and `gateway_heartbeats` columns in section 2 | It picks a layout by `schema_version` and `PRAGMA table_info`, not by the app's version, because this install tracks `main`. A missing column is drift (`missing-field`), and an unknown version is `new-version`. The first major covers through v32 and the older layout seen here |
| `vendor/nous/hermes-agent/gateway-state` | `gateway_state.json` and `gateway.pid` | `kind`, `pid`, `start_time`, `updated_at`, `gateway_state`, `exit_reason`, `active_agents`, `active_work`, `served_profiles`, `code_version`, and `platforms.<name>.state`, `needs_attention`, `updated_at` | `gateway_state` is a discriminator |
| `vendor/nous/hermes-agent/spawn-ledger` | `spawn-ledger.json` | `pid`, `create_time`, `purpose`, `install`, `spawner_pid`, `spawner_create`, `registered_at`, `host`, `port`, `profile`, `isolated` | `purpose` is a discriminator. `argv` is skipped |
| `vendor/nous/hermes-agent/processes` | `processes.json` | `pid`, `pid_scope`, `host_start_time`, `cwd`, `started_at`, `parent_session_id`, `task_id` | `command`, `watch_patterns` and `session_key` are skipped |
| `vendor/nous/hermes-agent/cron-jobs` | `cron/jobs.json` | `jobs[].id`, `state`, `enabled`, `schedule.kind`, `next_run_at`, `last_run_at`, `last_status`, `workdir` | `last_status` and `state` are discriminators. `name`, `prompt`, `deliver`, `script`, `skills` and the error texts are skipped |
| `vendor/nous/hermes-agent/cron-ticker` | `cron/ticker_heartbeat`, `cron/ticker_last_success` | epoch seconds, pid | One line, not JSON |
| `vendor/nous/hermes-agent/install-stamp` | `hermes-agent/install-stamp.json` | `schemaVersion`, `baseVersion`, `displayVersion`, `source`, `payload`, `branch` | The vendor version every other schema is tested against |
| `vendor/nous/hermes-agent/serve-status` | `GET /api/health` and `/api/status` on a running `serve` or `dashboard` | `ok`, `version`, `auth_required`, `gateway_running`, `gateway_state`, `gateway_updated_at`, `gateway_heartbeat_stale_s`, `active_agents`, `active_sessions`, `gateway_platforms`, `nous_session_valid` | The paths it returns when no login is required are skipped |
| `vendor/nous/hermes-agent/tool-calls-log` (optional) | `logs/tool_calls.log`, only with `display.tool_progress: log` | The timestamp and the tool's name | Not JSON. The preview after the colon is dropped |

**Registered mode.** The plugin posts `actor/event/v1` itself. The fields of
`hermes.observer.v1` it reads are pinned in its own (Python) code and tests,
not in a XenoNet schema.

### What XenoNet never reads

- **Secrets:**
  - `~/.hermes/.env`, `auth.json`, `pairing/` and `mcp-tokens/`;
  - the desktop's `connection.json`, `connections.json`,
    `secure-token-storage.json`, `native-oauth-tokens.json` and
    `desktop-installation.json`;
  - Electron's cookies and storage;
  - the nonce in `backend-ownership.json`.
- **Content in `state.db`:**
  - in `messages`: `content`, `reasoning`, `reasoning_content`,
    `reasoning_details`, `codex_reasoning_items`, `codex_message_items`,
    `api_content` and `display_metadata`;
  - `tool_calls`, apart from the `path` extraction if the owner allows it;
  - in `sessions`: `system_prompt` and `title`;
  - the `system_prompts` table;
  - the values in `state_meta`;
  - `gateway_routing.entry_json` and `delivery_obligations`.
- **Who's on the other end:** `sessions.user_id`, `chat_id`,
  `display_name` and `origin_json`; `gateway_heartbeats.host`; and
  `sessions/sessions.json`.
- **Everything else that holds content:**
  - `memories/`, `SOUL.md` and `cron/output/`;
  - the prompts and destinations in `cron/jobs.json`;
  - the commands in `processes.json` and their output;
  - `response_store.db` and `checkpoints/`;
  - log messages, and the previews in `tool_calls.log`.
- **Sources XenoNet doesn't use at all:**
  - other processes' environments;
  - any API route that needs a token;
  - `hermes mcp serve`;
  - running `hermes` commands to look. They're slow, they write caches,
    and they may first sync dependencies, which can mean downloads
    ([package management][docs-pm]).

## 6. Registered mode

Could Hermes report to XenoNet itself? Yes, in several ways:

| Way | Runs in | Cost per event | Content | What it changes on the owner's machine | Verdict |
|---|---|---|---|---|---|
| A XenoNet plugin for Hermes (plugin hooks) | CLI, TUI, desktop backend, gateway | Microseconds: a queue and a background thread | The plugin sees arguments in Hermes's own process and sends only metadata | `~/.hermes/plugins/xenonet/`, and `plugins.enabled: [xenonet]` in `config.yaml` | Recommended |
| Shell hooks calling `xenonet hook hermes` | CLI, gateway, desktop, TUI, dashboard chat | A process start per event, which the agent waits for | Its stdin carries `tool_input`, and the conversation for some events | A `hooks:` block, plus a consent entry per event and command | Like XenoNet's hooks for Claude Code and Codex, but slow on every tool call |
| Outbound webhooks to a XenoNet route | CLI, gateway | A background thread | The payloads include tool inputs, which XenoNet would have to drop on arrival | `hooks.outbound`, plus an HMAC secret in `.env` | No |
| XenoNet's MCP server as a Hermes tool | Wherever Hermes runs MCP | | None | `mcp_servers.xenonet` | For coordination (claims, messages), not telemetry |
| The API server | The gateway | | It gives full control | Keys in `.env` | No |
| The OTLP export | The gateway | A background thread | None | `monitoring` in `config.yaml`, plus a download of the `otlp` extra | Perhaps later. Gateway only |

### The XenoNet plugin for Hermes

**Files.** `plugin.yaml` (its name, version and description), and an
`__init__.py` whose `register(ctx)` registers these hooks
([hooks][docs-hooks]):

- `on_session_start`, `on_session_end` and `on_session_finalize`;
- `post_tool_call`, and `pre_tool_call` for `terminal`, to catch a launch;
- `subagent_start` and `subagent_stop`;
- `on_human_input_request` and `on_human_input_resolved`, so XenoNet can
  show that Hermes waits for the person.

It uses only Python's standard library, with a bounded queue that drops
events when it's full, a background sender, and a heartbeat every 60 s.

**What it sends:**

- Hermes's session id and the tool's name;
- for file tools, the path;
- for `terminal`, the `workdir`, and whether the command starts `claude`
  or `codex`. That's decided in Hermes's process, and the command never
  leaves it;
- the duration and status;
- for a wait, its kind only.

**What it never sends:** arguments, results, prompts, user messages, the
text of approvals, chat ids or user names.

**It never steers.** Its hooks return nothing, because return values from
`pre_tool_call` can block or rewrite a call.

**Finding and talking to XenoNet:**

- It finds XenoNet through `XENONET_PORT`, or else tries 47840 and 47842, as
  XenoNet's own client does.
- It registers, sends heartbeats and posts events through the actor API in
  `contracts.md` section 3.
- Where the token lives is for the interface doc to settle: a 0600 file in
  the plugin's folder, or a registration per process. Several Hermes
  processes run at once (the CLI, the desktop backend, the gateway), and
  they should all show as one actor per profile.

**XenoNet's side.** `core/procs.js` has to learn what a Hermes process looks
like (section 3). Without a code signature, the process check is weaker
than it is for Claude Code: a request must come "from under a Hermes agent
host".

**Hermes's own safeguards.** `HERMES_SAFE_MODE=1` skips plugins. A plugin
gets 10 s to load, and each hook callback 30 s.

### XenoNet's MCP server as a Hermes tool

- **The setup.** `hermes mcp add xenonet` would take XenoNet's runtime as the
  command, `cli/xenonet.js mcp` as its arguments, and `XENONET_PORT` (and
  `ELECTRON_RUN_AS_NODE=1`) as its environment. Hermes writes its own
  config, so XenoNet doesn't need to write YAML.
- **It can name only the actor.** Hermes sends no `_meta` with tool calls,
  and one MCP server serves every session in a process
  ([`mcp_tool_handlers.py`][src-mcp-handlers]). So the server can say
  `actor:hermes`, but not which session is calling. `caller()` in
  `mcp/xenonet.js` would need a Hermes branch, which the engine's process
  check backs up.
- **It costs tokens.** The tools' schemas add tokens to every Hermes turn.
  `trust: untrusted` would make Hermes ask for approval before calls that
  write (claims, messages).

### What either one changes

- **The plugin:**
  - adds one folder of a few kilobytes and one line in `config.yaml`;
  - takes effect at the next session or gateway restart;
  - comes out with `hermes plugins disable xenonet` and by removing the
    folder.
- **The MCP server:** one entry in `config.yaml`, which comes out with
  `hermes mcp remove xenonet`.
- **Like XenoNet's Connect flow** for Claude Code and Codex
  (`core/engine/connect.js`):
  - the owner sees a preview and confirms it in XenoNet's window;
  - XenoNet keeps a copy of `config.yaml` from before its first change
    (`config.yaml.before-xenonet`);
  - XenoNet refuses the change from under any agent.

## 7. Install plan

Nothing needs installing for the observed and attributed mode that Wave 2
builds. The rest is optional, and each step is the owner's to approve.

| # | Step | Needed for | Source | What it changes | Size | Cost | Who |
|---|---|---|---|---|---|---|---|
| 1 | Sort out Hermes.app's connection: sign in again to its saved remote gateway, or switch Desktop to local mode (Settings → Connections) | Anything to see on this Mac | [Desktop][docs-desktop] | Desktop's encrypted connection settings | None | None | Owner only (a sign-in) |
| 2 | A model sign-in, if the one from July has lapsed: `hermes setup --portal`, or `hermes auth add …` | Hermes doing anything | [Nous Portal][docs-portal] | `auth.json` or `.env`, and `config.yaml` | None | A Nous Portal plan ($0 to $200 a month, secondary source) or per-use provider prices | Owner only |
| 3 | Always on: `hermes gateway install` | An always-on actor; cron without the app open; messaging | [CLI reference][docs-cli], [`gateway_launchd.py`][src-launchd] | Adds `~/Library/LaunchAgents/ai.hermes.gateway.plist` (runs at login, restarts after crashes, waits 30 s between restarts) and starts `gateway run` now. It writes `gateway.pid`, `gateway.lock` and `gateway_state.json`, and logs to `gateway.log` and `gateway.error.log`. `hermes gateway uninstall` reverses it | No download: the code is here, and source installs carry the `all` extra ([package management][docs-pm]). One Python process | Free; models cost what it uses | Owner's OK |
| 4 | A messaging platform, for example a Telegram bot: `hermes gateway setup` | `message-people` | [Messaging][docs-messaging] | Tokens in `.env`, and `config.yaml` | None | The platforms are free; models cost what it uses | Owner only (bot tokens, pairing) |
| 5 | The XenoNet plugin for Hermes (later than Wave 2's core) | Registered mode | XenoNet, still to be written | `~/.hermes/plugins/xenonet/`, and `plugins.enabled` in `config.yaml` | A few KB | None | Owner's OK, in XenoNet |
| 6 | XenoNet's MCP server in Hermes (optional) | Hermes using XenoNet's claims and messages | XenoNet | `mcp_servers.xenonet` in `config.yaml` | None | A few tool schemas on every turn | Owner's OK |

**Not recommended:**

- **The API server.** It means `API_SERVER_ENABLED` and `API_SERVER_KEY`
  in `.env`, and whoever holds the key controls Hermes.
- **Outbound webhooks.** Their payloads carry tool inputs.
- **`display.tool_progress: log`.** It's gateway-only, and the same facts
  are in `state.db`.
- **The OTLP export.** It needs a download of the `otlp` extra, it's
  gateway-only, and `gateway_state.json` says the same.
- **Hermes's Codex runtime.** It rewrites part of `~/.codex/config.toml`,
  and it's out of XenoNet's scope, except to verify the `originator`.

**Updates** stay the owner's: `hermes update`, or Hermes.app's built-in
updates, which pull the latest agent and rebuild the app in place. This
install tracks `main`, so expect its formats to drift. The lenient readers
and drift records in `contracts.md` section 1 are what cover it.

## Corrections to the Phase 0 notes

- `tool_calls.log` isn't a standard log. It's written only by a gateway
  with `display.tool_progress: log`.
- `gateway_state.json` and `gateway.pid` live in `~/.hermes`, not in
  `logs/`.
- There's no `profiles/` folder here. It appears with the first named
  profile.
- `/Applications/Hermes.app` is the setup app. The Hermes.app that runs
  is a build inside the checkout.
- That app is set to a remote gateway, so it runs no local `hermes serve`
  now.
- `state.db` has more than `sessions`, `messages` and `messages_fts`. The
  current schema creates 17 tables, and the file here is an older layout.

## Open questions

1. **For the owner.** Is Hermes.app meant to use a remote gateway, on
   another machine or Hermes Cloud? If so, Hermes's work happens there, and
   on this Mac XenoNet sees only the app. That's for the hosts workstream, or
   for a cloud actor.
2. **For the owner.** Should Hermes be always on here (steps 3 and 4)?
3. **For the owner, before Wave 2's later steps.** Is the XenoNet plugin for
   Hermes all right to add?
4. **For the owner and the orchestrator.** May XenoNet take file paths from
   the arguments of tool calls with `json_extract`, or only through the
   plugin?
5. **For the interface doc:**
   - a `use-computer` capability;
   - whether an app that runs but can't reach its backend gets a presence
     value of its own;
   - `actor:hermes-<profile>` for named profiles;
   - how several Hermes processes register as one actor.
6. **To verify.** That threads from Hermes's Codex runtime carry
   `originator: "hermes"`. That takes the runtime switched on, which edits
   `~/.codex/config.toml` and so needs the owner's OK, or Codex's source.

[releases]: https://github.com/NousResearch/hermes-agent/releases
[docs-install]: https://hermes-agent.nousresearch.com/docs/getting-started/installation
[docs-desktop]: https://hermes-agent.nousresearch.com/docs/user-guide/desktop
[docs-cli]: https://hermes-agent.nousresearch.com/docs/reference/cli-commands
[docs-storage]: https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage
[docs-sessions]: https://hermes-agent.nousresearch.com/docs/user-guide/sessions
[docs-messaging]: https://hermes-agent.nousresearch.com/docs/user-guide/messaging/
[docs-cron]: https://hermes-agent.nousresearch.com/docs/developer-guide/cron-internals
[docs-heartbeat]: https://hermes-agent.nousresearch.com/docs/user-guide/features/heartbeat
[docs-hooks]: https://hermes-agent.nousresearch.com/docs/user-guide/features/hooks
[docs-observer]: https://hermes-agent.nousresearch.com/docs/developer-guide/observer-hooks
[docs-plugins]: https://hermes-agent.nousresearch.com/docs/user-guide/features/plugins
[docs-api]: https://hermes-agent.nousresearch.com/docs/user-guide/features/api-server
[docs-monitoring]: https://hermes-agent.nousresearch.com/docs/developer-guide/gateway-monitoring
[docs-toolsets]: https://hermes-agent.nousresearch.com/docs/reference/toolsets-reference
[docs-codex-runtime]: https://hermes-agent.nousresearch.com/docs/user-guide/features/codex-app-server-runtime
[docs-skill-claude]: https://hermes-agent.nousresearch.com/docs/user-guide/skills/bundled/autonomous-ai-agents/autonomous-ai-agents-claude-code
[docs-skill-codex]: https://hermes-agent.nousresearch.com/docs/user-guide/skills/bundled/autonomous-ai-agents/autonomous-ai-agents-codex
[docs-portal]: https://hermes-agent.nousresearch.com/docs/integrations/nous-portal
[docs-pm]: https://hermes-agent.nousresearch.com/docs/reference/package-management
[pricing-official]: https://portal.nousresearch.com/manage-subscription
[pricing-secondary]: https://www.hostinger.com/tutorials/hermes-agent-cost
[codex-app-server]: https://learn.chatgpt.com/docs/app-server.md
[codex-pr]: https://github.com/openai/codex/pull/8988
[cc-cli]: https://code.claude.com/docs/en/cli-reference
[src-backend-command]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/apps/desktop/electron/backend-command.ts
[src-parent-identity]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/apps/desktop/electron/parent-process-identity.ts
[src-hardening]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/apps/desktop/electron/hardening.ts
[src-launchd]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/gateway_launchd.py
[src-mcp-transport]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tools/mcp_tool_transport.py
[src-mcp-handlers]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tools/mcp_tool_handlers.py
[src-mcp-serve]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/mcp_serve.py
[src-local-env]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tools/environments/local.py
[src-session-context]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/gateway/session_context.py
[src-gw-status]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/gateway/status.py
[src-process-identity]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/process_identity.py
[src-process-registry]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tools/process_registry.py
[src-process-checkpoint]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tools/process_registry_checkpoint.py
[src-cron-jobs]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/cron/jobs.py
[src-source-stamp]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/source_stamp.py
[src-schema]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_state_common.py
[src-ids]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_state_ids.py
[src-session-activity]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/agent/session_activity.py
[src-tool-executor]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/agent/tool_executor.py
[src-reaper]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/tui_gateway/session_reaper.py
[src-status-router]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/web_routers/status.py
[src-public-paths]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/dashboard_auth/public_paths.py
[src-web-server]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_cli/web_server.py
[src-logging]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/hermes_logging.py
[src-run-turn-runner]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/gateway/run_turn_runner.py
[src-codex-session]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/agent/transports/codex_app_server_session.py
[src-shell-hooks]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/agent/shell_hooks.py
[src-toolsets]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/toolsets.py
[src-model-tools]: https://github.com/NousResearch/hermes-agent/blob/38d19b92e8/model_tools.py
