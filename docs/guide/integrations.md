# Agent workflows

The command palette has **Route task**, **Review decisions**, **Launch sandbox**, **Review sandbox changes** and **Review Pi runs**. The tabs share search, status filtering, oldest/newest sorting, pagination and record details. Sending decisions, configuring sources, launching jobs, retrieving changes, exporting patches, forgetting local reviews and deleting sandboxes require XenoNet's own window. The console MCP `integrations` tool can browse cached reviews and select a changed file. It cannot execute these actions.

## Routing decisions

Set the existing OpenRouter decision key in the command palette. It stays in the platform credential store. Type a short task, choose **Preview what will be sent**, review the exact request and candidate descriptions, then send it. Editing the form invalidates its preview. Previews expire after ten minutes and can be used once.

JevRouter ranks the observed Codex and Claude Code apps and human review. Observation establishes a suggested destination, not authority to start a process. Unavailable candidates stay visible and are filtered. Confidence below 75%, invalid probability distributions, provider errors and changed inventory produce a fallback. There is no automatic task execution, background decision polling or new model call in stall detection. Model/effort selection and arbitrary specialist inventories are not included in this first adapter.

XenoNet supplies JevRouter with its existing decision provider, preserving `provider.zdr: true`. It sends only the reviewed task and fixed candidate descriptions and availability, with no transcript, project paths or user/session identifiers. Obvious credential patterns are rejected; review the preview for other sensitive content. The provider receives the task, but the local receipt saves only its SHA-256, probabilities, selection, model, time, cost and your outcome annotation. Provider error bodies are omitted. Outcomes are feedback; marking a decision accepted does not grant permissions.

JevRouter is MIT and pinned to upstream commit `b1cdf80593ee530cedd99728a938f4e74ddd556b`. Its package is not currently published to the npm registry. A 95 KiB archive of the built upstream SDK lives in `vendor/`, with its license, provenance and lockfile integrity hash, for the JS core and its tests. The installed service doesn't carry it: the `xenonet` binary applies JevRouter's ranking and policy rules itself, for the one-stage choice XenoNet asks for. The SDK's CLI setup and raw-context receipt store are not used.

## OpenShell execution

Install and configure [OpenShell](https://docs.nvidia.com/openshell/latest/about/installation) in the execution environment first. On Windows, select WSL and enter the installed distribution's exact name. NVIDIA lists WSL support as experimental. **Check gateway** checks the CLI and sandbox-list API; it does not certify kernel enforcement. Qualify the runtime against NVIDIA's support matrix and test allowed and blocked operations before using sensitive projects.

Choose an image containing Pi, Codex or Claude Code, an optional existing provider, and optionally a policy YAML file and project directory. Confirm a project upload explicitly. The preview shows literal executable arguments and the policy file. XenoNet translates Windows paths through that WSL distribution's `wslpath`. It rechecks the policy hash and project path before launch.

Launch creates a uniquely named, detached sandbox running the selected agent in print/exec mode. It uses literal argument arrays without a host shell. A small shell command inside the sandbox closes the agent's stdin. A project upload happens before the agent starts. Directory uploads retain their basename, so XenoNet chooses the agent's working directory accordingly. Credentials belong in an OpenShell provider; XenoNet does not pass credentials as task text or environment flags. Without an explicit policy, OpenShell uses the image policy or its default. Attached providers can add access, so inspect the effective policy after creation.

A successful create command means the sandbox was created, not that the agent completed its task. **Inspect status, policy and logs** retrieves a bounded snapshot. Logs may be incomplete and are displayed on request, not saved into the integration record store. An execution policy denial is a distinct runtime event; it is not inserted into the provider-refusal dataset or used as a stall signal.

If a launch step times out or loses its connection, the record says `creation-unknown` and shows how many steps completed. The sandbox may still exist. Inspect it before retrying. Jobs survive closing the panel or desktop. XenoNet never deletes them automatically. **Review deletion** requires a second click and deletes the sandbox and its files. Retrieve changes you want before deleting. Interactive PTY attachment is not part of this adapter.

## Review sandbox changes

For a Git project, select **Freeze a text-only Git snapshot for change review and patch export** before previewing the launch. The preview lists the selected files, exclusions, byte count and baseline hash. Review this list before launching. Existing uncommitted edits are part of the baseline.

XenoNet stages the selected bytes outside the project. It excludes `.git`, ignored files, dependency/cache directories, links, binary files and common credential filenames. These rules do not recognize every secret. Review the manifest yourself. The first version supports portable filenames, up to 512 regular UTF-8 text files, 256 KiB per file and 4 MiB in total. File permissions are outside the review.

New result files use the repository `.gitignore` rules captured in the baseline, even if the sandbox changes those rules. Existing omitted paths stay excluded. Global and local-only ignore rules are not applied to newly created paths.

The image needs the selected agent, `/bin/sh`, `sleep` and Node at `/usr/local/bin/node`. XenoNet uploads the frozen directory as `/workspace/project`, reads it back and compares it with the manifest before releasing the agent. A mismatch keeps the agent waiting and leaves the sandbox available for inspection or deletion.

OpenShell 0.1.4 refuses commands in a completed sandbox. Review-enabled jobs keep the sandbox running after their agent exits so you can retrieve files. Delete the sandbox when you finish reviewing it. The gateway's phase does not establish task completion, and XenoNet labels execution as unknown.

Open **Review sandbox changes**, select the job and choose **Retrieve text changes**. XenoNet compares a bounded snapshot with its own baseline. It shows changed files and a read-only diff. Renames appear as a deletion and an addition. Binary files, links, invalid paths, unstable reads and exceeded limits make retrieval incomplete and block export. An interrupted retrieval preserves the previous local review.

The collector also limits directory depth and the number of entries it visits. A large generated tree can reach these limits before host-side ignore filtering. Retrieval then reports that it is unavailable and retains the last cached review. Remove unnecessary generated output inside the sandbox before trying again.

**Export reviewed patch** opens a save dialog. It saves the cached revision you reviewed, with a receipt containing its baseline hash, retrieval time, exclusions, source state and patch hash. It does not retrieve newer files behind the button. The patch targets the frozen baseline. If your project changes during the run, the panel says so. Review and apply the patch yourself in the intended checkout.

The baseline and retrieved changes contain source code and remain in XenoNet's local state directory after sandbox deletion. **Forget local review** requires a second click and removes them. It does not delete the sandbox. Metadata retention keeps these jobs reachable until their local review is forgotten. Local storage allows 32 reviews, 24 MiB per serialized review and 128 MiB overall. At a limit, XenoNet retains existing reviews and asks you to forget an old one. A diff too large to retain leaves the previous review intact. Older jobs and jobs launched without a baseline support inspection and deletion.

## Pi records

Reading Native Pi's `~/.pi/agent/sessions` is off by default. **Read Pi sessions** enables metadata reads when this panel refreshes; **Stop reading** disables them. The source is never modified. The reader supports session versions through 3, reads at most 500 session files up to 8 MiB each, skips symlinks, and reports missing, malformed, unsupported or oversized inputs. Partial JSONL tails are tolerated. Metadata follows the latest recorded parent chain, rather than combining abandoned branches.

Pi records show model, provider, tokens, cost, tool-call/result counts, compactions and pending results. `recorded` and `awaiting-result` describe file evidence, not live process health. Pending results do not establish a stall. This panel is a read-only run bridge; Pi is not yet a native map session with Claude/Codex control actions.

Import a pi-harness JSON run export under 1 MiB through the file picker. XenoNet projects metadata and counts; it omits prompts, trace content, arguments, results and artifact contents. Reimporting the same run updates its saved metadata. Markdown and debug-bundle formats are not accepted. No pi-harness implementation is embedded: its AGPL code remains separate from XenoNet. Native Pi is MIT.

## Storage and verification

Integration records and the Pi reading preference use XenoNet's platform state directory under `integrations/`. The latest 1,000 completed records are retained, plus all undeleted sandbox jobs so retention cannot orphan them. At most 64 undeleted jobs can be launched. Deletion stays pending until a gateway listing confirms absence. Preview inputs stay in service memory only. Nothing creates a plaintext `.env` file.

Unit and API tests cover provider payloads, malformed scores, outages, inventory changes, privacy, expiry, owner gates, literal subprocess arguments, timeouts, launch uncertainty, policy changes, status/deletion, bounded Pi reads, branch handling, imports, persistence and querying. Native UI tests use synthetic providers and a simulated gateway; they do not prove that a separately installed gateway enforces its policy. No live paid decision is needed to run the suite.

The free result check uses a deterministic fixture agent and no provider. On Linux or WSL2, build its image with `docker build -t xenonet-review-fixture:1 -f scripts/openshell/Dockerfile.review-fixture scripts/openshell`, then run `node scripts/openshell/verify-results.mjs`. It checks the real upload destination, exact bytes, dirty-source preservation, ignored and sensitive-file exclusions, a patch applied to a disposable copy, and deletion of its own sandbox. The opt-in test uses `XENONET_REVIEW_REAL=1 node --test test/real/sandbox-results.test.js`.
