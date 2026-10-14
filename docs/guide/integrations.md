# Agent workflows

The command palette has **Route task**, **Review decisions**, **Launch sandbox** and **Review Pi runs**. The tabs share search, status filtering, oldest/newest sorting, pagination and record details. Sending decisions, configuring sources, launching jobs and deleting sandboxes require XenoNet's own window. The console MCP `integrations` tool can browse; it cannot execute these actions.

## Routing decisions

Set the existing OpenRouter decision key in the command palette. It stays in the platform credential store. Type a short task, choose **Preview what will be sent**, review the exact request and candidate descriptions, then send it. Editing the form invalidates its preview. Previews expire after ten minutes and can be used once.

JevRouter ranks the observed Codex and Claude Code apps and human review. Observation establishes a suggested destination, not authority to start a process. Unavailable candidates stay visible and are filtered. Confidence below 75%, invalid probability distributions, provider errors and changed inventory produce a fallback. There is no automatic task execution, background decision polling or new model call in stall detection. Model/effort selection and arbitrary specialist inventories are not included in this first adapter.

XenoNet supplies JevRouter with its existing decision provider, preserving `provider.zdr: true`. It sends only the reviewed task and fixed candidate descriptions and availability, with no transcript, project paths or user/session identifiers. Obvious credential patterns are rejected; review the preview for other sensitive content. The provider receives the task, but the local receipt saves only its SHA-256, probabilities, selection, model, time, cost and your outcome annotation. Provider error bodies are omitted. Outcomes are feedback; marking a decision accepted does not grant permissions.

JevRouter is MIT and pinned to upstream commit `b1cdf80593ee530cedd99728a938f4e74ddd556b`. Its package is not currently published to the npm registry. A 95 KiB archive of the built upstream SDK lives in `vendor/`, with its license, provenance and lockfile integrity hash. This makes script-free production installs reproducible. Component builds also verify that the packaged integration adapters import successfully. The SDK supplies ranking and policy evaluation; its CLI setup and raw-context receipt store are not used.

## OpenShell execution

Install and configure [OpenShell](https://docs.nvidia.com/openshell/latest/about/installation) in the execution environment first. On Windows, select WSL and enter the installed distribution's exact name. NVIDIA lists WSL support as experimental. **Check gateway** checks the CLI and sandbox-list API; it does not certify kernel enforcement. Qualify the runtime against NVIDIA's support matrix and test allowed and blocked operations before using sensitive projects.

Choose an image containing Pi, Codex or Claude Code, an optional existing provider, and optionally a policy YAML file and project directory. Confirm a project upload explicitly. The preview shows literal executable arguments and the policy file. XenoNet translates Windows paths through that WSL distribution's `wslpath`. It rechecks the policy hash and project path before launch.

Launch creates a uniquely named, detached sandbox running the selected agent in print/exec mode. It uses argument arrays without a host shell. Credentials belong in an OpenShell provider; XenoNet does not pass credentials as task text or environment flags. Without an explicit policy, OpenShell uses the image policy or its default. Attached providers can add access, so inspect the effective policy after creation.

A successful create command means the sandbox was created, not that the agent completed its task. **Inspect status, policy and logs** retrieves a bounded snapshot. Logs may be incomplete and are displayed on request, not saved into the integration record store. An execution policy denial is a distinct runtime event; it is not inserted into the provider-refusal dataset or used as a stall signal.

If creation times out or loses its connection, the record says `creation-unknown`. The sandbox may still exist: inspect it before retrying. Jobs survive closing the panel or desktop. XenoNet never deletes them automatically. **Review deletion** requires a second click and deletes the sandbox and its files. Export/review desired changes with OpenShell before deleting; XenoNet does not copy changes into the source project. Interactive PTY attachment and automatic artifact download are not part of this adapter.

## Pi records

Reading Native Pi's `~/.pi/agent/sessions` is off by default. **Read Pi sessions** enables metadata reads when this panel refreshes; **Stop reading** disables them. The source is never modified. The reader supports session versions through 3, reads at most 500 session files up to 8 MiB each, skips symlinks, and reports missing, malformed, unsupported or oversized inputs. Partial JSONL tails are tolerated. Metadata follows the latest recorded parent chain, rather than combining abandoned branches.

Pi records show model, provider, tokens, cost, tool-call/result counts, compactions and pending results. `recorded` and `awaiting-result` describe file evidence, not live process health. Pending results do not establish a stall. This panel is a read-only run bridge; Pi is not yet a native map session with Claude/Codex control actions.

Import a pi-harness JSON run export under 1 MiB through the file picker. XenoNet projects metadata and counts; it omits prompts, trace content, arguments, results and artifact contents. Reimporting the same run updates its saved metadata. Markdown and debug-bundle formats are not accepted. No pi-harness implementation is embedded: its AGPL code remains separate from XenoNet. Native Pi is MIT.

## Storage and verification

Integration records and the Pi reading preference use XenoNet's platform state directory under `integrations/`. The latest 1,000 completed records are retained, plus all undeleted sandbox jobs so retention cannot orphan them. At most 64 undeleted jobs can be launched. Deletion stays pending until a gateway listing confirms absence. Preview inputs stay in service memory only. Nothing creates a plaintext `.env` file.

Unit and API tests cover provider payloads, malformed scores, outages, inventory changes, privacy, expiry, owner gates, literal subprocess arguments, timeouts, launch uncertainty, policy changes, status/deletion, bounded Pi reads, branch handling, imports, persistence and querying. Native UI tests use synthetic providers and a simulated gateway; they do not prove that a separately installed gateway enforces its policy. No live paid decision is needed to run the suite.
