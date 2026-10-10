# Provider drift

XenoNet reads data that other apps write: Claude Code's transcripts, the Agent SDK's session list, Codex's rollouts and app-server answers, and more. Each has a schema in `schemas/vendor/`, with the range of versions it was made for (see [Schemas](../design/schemas.md#vendor-data)). The apps change often. The provider drift check installs their newest releases every day, runs them the way XenoNet reads them, and reports where their output and published types differ from XenoNet's schemas.

It runs as the **Provider drift** workflow (`.github/workflows/provider-drift.yml`), daily and on demand from the Actions tab. It needs no secrets.

## What it checks

| Provider | Versions | How |
| --- | --- | --- |
| Claude Agent SDK | npm `latest`, and the version in XenoNet's `package.json` | Compares `SDKSessionInfo` and `SessionMessage` in its `sdk.d.ts` with XenoNet's schemas, and checks it still exports what `core/` imports. Has the SDK's own Claude Code run one turn, then calls `listSessions`, `getSessionMessages`, `getSessionInfo` and `renameSession`, and reads the answers through XenoNet's readers. |
| Claude Code | npm `latest` and `stable`, and the VM lab's version | Runs `claude -p` for one turn, with `--bare`, and reads the transcript it writes the way XenoNet's tailer does. Checks that XenoNet finds the prompt, the tool call, its result, the tokens and the turn's end. |
| Codex CLI | npm `latest`, and the VM lab's version | Compares the app-server protocol's JSON Schemas (`codex app-server generate-json-schema`) with XenoNet's, and checks the methods XenoNet calls and the params it sends. Runs `codex exec` for one turn and reads the rollout. Then starts `codex app-server` and calls `initialize`, `thread/list`, `thread/items/list` and `thread/name/set` as XenoNet does, and opens the state database XenoNet opens. |
| Claude desktop | The newest in Anthropic's APT repository | The version only. Its session files and plan usage samples are written by the signed-in app as you use it, so a runner has nothing to read. |
| Grok Bot desktop | The latest on Cursor's update server, which the app asks itself | The version only. XenoNet reads none of Grok Bot's data, only the processes its app runs and its version. |
| Muse desktop | The latest in the app's Sparkle update feed | The version only. XenoNet reads none of Muse's data, only whether its app runs and two keys of its `Info.plist`. |
| GitHub CLI | The latest GitHub release | The version only. `hosts.yml` is written by signing in. |
| Hermes Agent | The latest GitHub release | The version, checked against the ranges of the schemas whose data carries it. Its files are written by a running Hermes with a model behind it. |
| Anthropic's plan usage API, Codex's sign-in claims | None | Not inspected: both need a signed-in account. |
| ChatGPT and Codex desktop apps | None | Not inspected: they bundle the `codex` binary, so the Codex CLI's check covers the formats. No public feed for their versions is known yet. |

Each CLI runs one turn against a stand-in model API on `127.0.0.1` (`scripts/provider-drift/fake-api.mjs`), which asks for one harmless tool call (reading a made-up file, or updating a plan) and then replies in text. The model requests go to the stand-in, not to Anthropic or OpenAI, no command runs, and the CLIs' telemetry and update checks are turned off where they have a setting for it.

Each CLI also runs in a made-up home (`scripts/provider-drift/sandbox.mjs`): `HOME`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and the app data folders all point inside a temporary folder, and the CLIs get none of the runner's keys. Claude Code runs with `--bare`, so it doesn't look for `CLAUDE.md` files in the folders above the project, which on Windows include your home folder. So you can run the check on your own machine, and it leaves your `~/.claude`, `~/.codex` and desktop apps' folders alone.

## The report

Every run writes `report.md` and `report.json`. The workflow shows `report.md` as the run's summary and keeps both as the `provider-drift` artifact for 30 days. For each provider and version, the report lists:

- **Changes to what XenoNet reads**: a field XenoNet reads that's gone or of another type, a version outside a schema's range, an export or method XenoNet calls that's gone, or an expectation that failed (the data fits the schema, but XenoNet doesn't find what the turn put there). These can break XenoNet.
- **Additions**: a field a closed object doesn't list, or a kind of record a schema doesn't list. These can't break a reader, but they show up as drift for people running that version.
- **Notes**: things that are fine, such as a kind of record a newer version stopped writing, which XenoNet still reads from older versions.
- **Couldn't run**: a step of the check failed for that version. A CLI may have changed its flags or output.

An older version's findings are set beside the latest's. A field the older version doesn't have yet is a note, since XenoNet reads it when it's there. A finding the older version shares with the latest is counted once.

## What the workflow does with it

- **The tracking issue.** One open issue, labelled `provider-drift`, holds the latest report. The workflow opens it when there's something to report, comments when the findings change, and closes it with a comment when they're gone. When the findings are the same as last time, it updates the description without a comment.
- **The ranges pull request.** When the latest version is outside a schema's range, and the check read that version without a change to what XenoNet reads, the workflow widens the range. It edits `core/schema/vendor/`, runs `npm run schemas` and `npm test`, and pushes the branch `provider-drift/ranges`. Then it opens or updates a pull request from it. If the tests fail, the pull request is a draft and says why. The range keeps every version it had, so older hosts still read with the same schema. The workflow changes nothing else: what XenoNet should read is a person's to decide.

A run on a few providers, chosen when you start it by hand, leaves the issue and the pull request alone, since it doesn't speak for the rest.

Two repository settings matter:

- The pull request needs **Allow GitHub Actions to create and approve pull requests** (Settings, Actions, General). Without it, the workflow pushes the branch and warns that it couldn't open the pull request.
- Pushes made with the workflow's token don't start other workflows, so the Checks workflow doesn't run on that pull request by itself. Close and reopen it, or push a commit to it, to run them. Don't build on the branch: each run replaces it. Branch from it instead.

## Acting on the report

Start with the provider's release notes for the version, then:

1. **A field or kind XenoNet should read**: declare it in the schema in `core/schema/vendor/`, optional so older data still fits, then read it in the adapter.
2. **A field or kind XenoNet passes over**: list it, a kind with `skipped`, a key of a closed object in `known`.
3. **A field gone or retyped for good**: add a new major of the schema with the new version range, beside the old one, so data from older versions still reads with the schema it was written for. Never remove what older hosts send.
4. Run `npm run schemas`, `npm test`, and the check for that provider, as below.

[Schemas](../design/schemas.md#when-the-report-shows-drift) has more on each.

## Running it yourself

```bash
npm run provider-drift                       # every provider, latest and pinned versions
npm run provider-drift -- --providers openai/codex
npm run provider-drift -- --version openai/codex=0.161.0-alpha.13   # a version of your choosing
npm run provider-drift -- --latest-only     # skip the pinned versions
npm run provider-drift -- --no-inspect      # look up versions and ranges only
npm run provider-drift -- versions          # the latest versions, beside XenoNet's pins
```

It installs each version into a temporary folder and deletes it after. The CLIs are a few hundred megabytes each, so a full run takes a few minutes. The report goes in `.xenonet/provider-drift/`. `--out` and `--work` choose other folders, `--json` prints the report as JSON, and `--exit-code` exits 1 when there's something to look at. `fix --report <report.json> --dry-run` shows the ranges a report would widen.

## Adding a provider

The providers are listed in `scripts/provider-drift/registry.mjs`. Each says the vendor sources it covers, where its versions are published, the versions XenoNet pins, how to inspect a version, and what can't be inspected and why. A unit test fails when a vendor source has no provider.

The registry is also where a release watcher would start. `release` says where to look for a new version, and `inspect` checks one. A watcher would poll the first, keep what it has seen, and queue the second for each new release. `npm run provider-drift -- versions --json` gives what it would poll today.
