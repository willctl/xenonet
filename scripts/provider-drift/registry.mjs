// The providers whose data XenoNet reads, and for each: the vendor sources it
// covers (schemas/vendor/**), the ones whose data says the provider's own
// version (`rangeSources`, whose schemas' ranges a new version must be in),
// where its versions are published, the versions XenoNet pins, how to inspect
// a version of it, and what can't be inspected yet and why.
//
// This is what the scheduled drift check (scripts/provider-drift.mjs)
// walks. A release watcher can walk it too: `release` says where to look
// for a new version (releases.mjs answers), and `inspect` checks one, so
// queuing work for a new release is a matter of calling both and keeping
// what was seen. Every vendor source must belong to a provider here; a unit
// test (test/provider-drift.test.js) says so when one doesn't.
import fs from "node:fs";
import path from "node:path";
import { inspectAgentSdk, inspectClaudeCode } from "./inspect-claude.mjs";
import { inspectCodex } from "./inspect-codex.mjs";
import { inspectGeminiCli, inspectCopilotCli, inspectOpenCode } from "./inspect-native.mjs";
import { REPO } from "./common.mjs";

const readJson = (rel) => JSON.parse(fs.readFileSync(path.join(REPO, rel), "utf8"));
// What XenoNet itself ships with, from package.json.
const dependency = (pkg) => () => readJson("package.json").dependencies?.[pkg]?.replace(/^[\^~=]/, "") ?? null;
// What the VM lab installs (scripts/lab/versions.json).
const lab = (key) => () => readJson("scripts/lab/versions.json")[key] ?? null;

export const PROVIDERS = [
  {
    id: "anthropic/agent-sdk",
    name: "Claude Agent SDK",
    sources: ["anthropic/agent-sdk/session-listing", "anthropic/agent-sdk/session-message"],
    rangeSources: ["anthropic/agent-sdk/session-listing", "anthropic/agent-sdk/session-message"],
    release: { npm: "@anthropic-ai/claude-agent-sdk", tags: ["latest"] },
    pins: [{ label: "XenoNet's dependency", version: dependency("@anthropic-ai/claude-agent-sdk") }],
    inspect: inspectAgentSdk,
    // The SDK's sdk.d.ts against XenoNet's schemas. A message's timestamp is
    // sent, though the types don't declare it.
    types: [
      { source: "anthropic/agent-sdk/session-listing", xenonet: [], provider: "SDKSessionInfo" },
      { source: "anthropic/agent-sdk/session-message", xenonet: [], provider: "SessionMessage", untyped: ["timestamp"] },
    ],
  },
  {
    id: "anthropic/claude-code",
    name: "Claude Code",
    sources: ["anthropic/claude-code/transcript-line", "anthropic/claude-code/config"],
    rangeSources: ["anthropic/claude-code/transcript-line"],
    release: { npm: "@anthropic-ai/claude-code", tags: ["latest", "stable"] },
    pins: [{ label: "the VM lab", version: lab("claude") }],
    inspect: inspectClaudeCode,
  },
  {
    id: "anthropic/claude-desktop",
    name: "Claude desktop",
    sources: ["anthropic/claude-desktop/session", "anthropic/claude-desktop/plan-usage"],
    // Anthropic's APT repository for the Linux app; the macOS and Windows
    // apps share its version numbers.
    release: { apt: { index: "https://downloads.claude.ai/claude-desktop/apt/stable/dists/stable/main/binary-amd64/Packages", pkg: "claude-desktop" } },
    notInspected: "Its session files and plan usage samples are written by the signed-in app as you use it, and their formats aren't published, so a runner has nothing to read. The check records the latest version only.",
  },
  {
    id: "anthropic/oauth",
    name: "Anthropic's plan usage API",
    sources: ["anthropic/oauth/usage"],
    notInspected: "Answering it needs a signed-in Claude account's token, and the check runs with no secrets.",
  },
  {
    id: "openai/codex",
    name: "Codex CLI",
    sources: ["openai/codex/rollout-line", "openai/codex/app-server", "openai/codex/state-db", "openai/codex/auth-claims"],
    rangeSources: ["openai/codex/rollout-line", "openai/codex/app-server"],
    release: { npm: "@openai/codex", tags: ["latest"] },
    pins: [{ label: "the VM lab", version: lab("codex") }],
    inspect: inspectCodex,
    notInspectedSources: { "openai/codex/auth-claims": "The claims are in a signed-in account's ID token, and the check runs with no secrets." },
    // The protocol's JSON Schemas (codex app-server generate-json-schema)
    // against XenoNet's. `bundle` v1 is the combined file the older methods
    // are in.
    types: [
      { source: "openai/codex/app-server", xenonet: ["initialize"], provider: "InitializeResponse", bundle: "v1" },
      { source: "openai/codex/app-server", xenonet: ["thread/list"], provider: "ThreadListResponse" },
      { source: "openai/codex/app-server", xenonet: ["thread/list", "data", "[]"], provider: "Thread" },
      // origin_url is what older app-servers sent.
      { source: "openai/codex/app-server", xenonet: ["thread/list", "data", "[]", "gitInfo"], provider: "GitInfo", legacy: ["origin_url"] },
      { source: "openai/codex/app-server", xenonet: ["thread/items/list"], provider: "ThreadItemsListResponse" },
      { source: "openai/codex/app-server", xenonet: ["thread/items/list", "data", "[]"], provider: "ThreadItemEntry" },
      { source: "openai/codex/app-server", xenonet: ["thread/items/list", "data", "[]", "item"], provider: "ThreadItem" },
      { source: "openai/codex/app-server", xenonet: ["thread/items/list", "data", "[]", "item", "<fileChange>", "changes", "[]"], provider: "FileUpdateChange" },
      { source: "openai/codex/app-server", xenonet: ["thread/items/list", "data", "[]", "item", "<fileChange>", "changes", "[]", "kind"], provider: "PatchChangeKind" },
      // A rollout's response items are the protocol's ResponseItem. XenoNet
      // reads one set of fields from every kind of tool call; "other" is
      // the protocol's catch-all, never written.
      {
        source: "openai/codex/rollout-line",
        xenonet: ["<response_item>", "payload"],
        provider: "ResponseItem",
        ignoreVariants: ["other"],
        shared: ["call_id", "id", "name", "input", "arguments", "action", "output"],
      },
      { source: "openai/codex/rollout-line", xenonet: ["<event_msg>", "payload", "<item_completed>", "item", "<WebSearch>", "action"], provider: "WebSearchAction" },
    ],
    // The params XenoNet sends, which the protocol must still take.
    params: {
      "thread/list": ["archived", "limit", "cursor", "sourceKinds", "useStateDbOnly"],
      "thread/items/list": ["threadId", "limit", "cursor", "sortDirection"],
    },
  },
  {
    id: "openai/codex-desktop",
    name: "ChatGPT and Codex desktop apps",
    sources: [],
    notInspected:
      "They bundle the codex binary XenoNet runs, so their data is Codex's and the Codex CLI's check covers its formats. No public feed for their versions is known to the check, so the bundled Codex version isn't recorded.",
  },
  {
    id: "meta/muse-desktop",
    name: "Muse desktop",
    sources: [],
    // The update feed its Info.plist names (SUFeedURL), which anyone can
    // read.
    release: { sparkle: "https://www.facebook.com/endo/release/appcast.xml?channel=production" },
    notInspected:
      "XenoNet reads none of its data, only whether its app runs, and from its Info.plist its version and bundle id. Its work happens in Meta's cloud, and what it keeps on the Mac is its sign-in and caches. The check records the latest version only.",
  },
  {
    id: "github/gh",
    name: "GitHub CLI",
    sources: ["github/gh/hosts"],
    release: { github: "cli/cli" },
    notInspected: "hosts.yml is only written by signing in to GitHub, and the check runs with no secrets. The check records the latest version only.",
  },
  {
    id: "nous/hermes-agent",
    name: "Hermes Agent",
    sources: [
      "nous/hermes-agent/state-db",
      "nous/hermes-agent/spawn-ledger",
      "nous/hermes-agent/gateway-state",
      "nous/hermes-agent/processes",
      "nous/hermes-agent/cron-jobs",
      "nous/hermes-agent/cron-ticker",
      "nous/hermes-agent/install-stamp",
    ],
    // Its tags are dates (v2026.9.24); the version is in the release's
    // name ("Hermes Agent v0.21.5 (v2026.9.24)").
    release: { github: "NousResearch/hermes-agent", pattern: "v(\\d+\\.\\d+\\.\\d+)" },
    // Every source but state-db and the stamp is read against Hermes's
    // version; those two carry versions of their own format.
    rangeSources: ["nous/hermes-agent/spawn-ledger", "nous/hermes-agent/gateway-state", "nous/hermes-agent/processes", "nous/hermes-agent/cron-jobs", "nous/hermes-agent/cron-ticker"],
    notInspected: "Its files are written by a running Hermes with a model behind it. The check records the latest version, and whether the schemas' ranges take it.",
  },
  // The providers XenoNet's experimental readers read (core/native/*.js).
  // `experimental` marks them in the report. None of them is run: each is
  // checked against what its project publishes (inspect-native.mjs).
  {
    id: "anomaly/opencode",
    name: "OpenCode",
    experimental: true,
    sources: ["anomaly/opencode/db"],
    release: { npm: "opencode-ai", tags: ["latest"] },
    // Its database says the version that wrote each session.
    rangeSources: ["anomaly/opencode/db"],
    // Its tables, from its source at the release's tag.
    inspect: inspectOpenCode,
    notInspected: "No run is made: its database is written by a running OpenCode with a model behind it. Its tables are read from its source at the release's tag instead.",
  },
  {
    id: "google/gemini-cli",
    name: "Gemini CLI",
    experimental: true,
    sources: ["google/gemini-cli/chat-record"],
    release: { npm: "@google/gemini-cli", tags: ["latest"] },
    // The types in @google/gemini-cli-core of the same version
    // (chatRecordingTypes.d.ts). XenoNetChatRecordLine is the check's own
    // name for any line of a record (inspect-native.mjs).
    inspect: inspectGeminiCli,
    types: [
      { source: "google/gemini-cli/chat-record", xenonet: [], provider: "XenoNetChatRecordLine" },
      { source: "google/gemini-cli/chat-record", xenonet: ["toolCalls", "[]"], provider: "ToolCallRecord" },
      { source: "google/gemini-cli/chat-record", xenonet: ["$set"], provider: "ConversationRecord" },
    ],
    notInspected: "No run is made: its chat records are written by a running Gemini CLI with a model behind it, and carry no version. Their types are read from @google/gemini-cli-core of the same version instead.",
  },
  {
    id: "github/copilot-cli",
    name: "GitHub Copilot CLI",
    experimental: true,
    sources: ["github/copilot-cli/session-event"],
    release: { npm: "@github/copilot", tags: ["latest"] },
    // The event types the Copilot SDK publishes (generated/session-events.d.ts),
    // its latest whatever the CLI's version. XenoNetEventData is the check's
    // own name for the data of the events XenoNet reads (inspect-native.mjs).
    inspect: inspectCopilotCli,
    types: [
      { source: "github/copilot-cli/session-event", xenonet: [], provider: "SessionEvent" },
      { source: "github/copilot-cli/session-event", xenonet: ["data"], provider: "XenoNetEventData" },
      { source: "github/copilot-cli/session-event", xenonet: ["data", "context"], provider: "WorkingDirectoryContext" },
      { source: "github/copilot-cli/session-event", xenonet: ["data", "codeChanges"], provider: "ShutdownCodeChanges" },
    ],
    notInspected: "No run is made: its event logs are written by a running Copilot CLI signed in to GitHub, and the check runs with no secrets. The event types are read from the Copilot SDK's latest release instead, which is versioned apart from the CLI.",
  },
  {
    id: "anysphere/cursor",
    name: "Cursor",
    experimental: true,
    sources: ["anysphere/cursor/hook-input"],
    notInspected: "Its hook input is only written by a running Cursor agent with a model behind it, and Cursor publishes no package or source for it to read. The check records nothing; the schema follows Cursor's hooks reference.",
  },
];

export const providerById = (id) => PROVIDERS.find((p) => p.id === id) ?? null;

// The versions to inspect for a provider: its release tags (in the order
// given), then its pins, each version once, with every label it goes by.
export function targetsOf(provider, released, pins = []) {
  const out = [];
  const add = (version, label) => {
    if (!version) return;
    const had = out.find((t) => t.version === version);
    if (had) had.labels.push(label);
    else out.push({ version, labels: [label] });
  };
  for (const tag of provider.release?.tags ?? ["latest"]) add(released?.[tag], tag);
  for (const pin of pins) add(pin.version, pin.label);
  return out;
}

// The pins' versions, read now.
export const pinsOf = (provider) =>
  (provider.pins ?? []).map((p) => {
    let version = null;
    try {
      version = p.version();
    } catch {}
    return { label: p.label, version };
  });
