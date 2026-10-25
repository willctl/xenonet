// assistant/*: the read-only interface a remote actor (a cloud assistant
// such as an OpenAI dot) reaches through `xenonet mcp assistant`, and the
// owner's view of its connections (docs/design/cloud-assistants.md).
//
// Unlike XenoNet's other schemas these are strict: data that leaves the
// machine is an allowlist, so a field the projection doesn't name can't
// ride along, and an argument the contract doesn't name is refused. The
// MCP adapter declares the same schemas, and the service checks every
// result against them before releasing it.
import { define } from "./registry.js";
import { z } from "./common.js";

export const RESULT_SCHEMA = "xenonet.assistant.result.v1";
export const ERROR_SCHEMA = "xenonet.assistant.error.v1";
export const ASSISTANT_API = { min: 1, max: 1 };

// The six tools, and the service operation each one runs.
export const OPERATIONS = {
  xenonet_status: "status",
  xenonet_projects: "projects",
  xenonet_sessions: "sessions",
  xenonet_attention: "attention",
  xenonet_session: "session",
  xenonet_actors: "actors",
};
export const TOOLS = Object.keys(OPERATIONS);

export const STATUSES = ["working", "waiting", "idle", "completed", "stuck", "unknown"];
// What a status was read from: a turn under way, a question or permission
// waiting, a turn that ended, nothing under way, a turn you stopped, no
// transcript activity lately, a stall XenoNet inferred, or a source it
// couldn't read.
export const STATUS_BASES = ["observed_turn_active", "observed_wait", "observed_turn_end", "observed_idle", "observed_interrupt", "observed_inactive", "inferred_stall", "source_unavailable"];
export const ATTENTION_KINDS = ["waiting", "stalled", "collision", "turn_completed"];
export const REASON_CODES = ["input_requested", "approval_requested", "repeated_failure", "inactive_mid_turn", "recent_overlap", "turn_ended", "unknown"];
export const ERROR_CODES = ["NOT_AUTHORIZED", "NOT_FOUND", "GRANT_EXPIRED", "GRANT_REVOKED", "DISABLED", "INVALID_ARGUMENT", "CURSOR_EXPIRED", "SOURCE_UNAVAILABLE", "SERVICE_UNAVAILABLE", "RATE_LIMITED", "INCOMPATIBLE_VERSION", "TIMEOUT", "INTERNAL_ERROR"];
export const WARNING_CODES = ["SOURCE_DEGRADED", "SOURCE_UNAVAILABLE", "LIST_CAPPED", "PAGE_REDUCED", "STATS_UNAVAILABLE", "SERVED_FROM_CACHE", "FIXTURE_DATA"];
export const SOURCES = ["claude", "codex", "grok"];
export const PRESENCES = ["working", "idle", "away", "offline", "unknown"];
export const PRESENCE_BASES = ["observed", "reported", "remote_request", "unknown"];
export const CONNECTION_STATES = ["connected", "not_connected", "none"];
export const LINK_STATES = ["verified", "owner_asserted", "unverified"];
export const TRANSPORT_SCOPES = ["owner_endpoint", "authenticated_user", "other_approved_scope"];
export const RELATIONS = ["works-on", "spawned", "edited", "messaged", "watches", "reviews"];

// ---------- ids ----------

const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
export const DEVICE_ID = new RegExp(`^dev_${UUID}$`);
export const PROJECT_ID = new RegExp(`^prj_${UUID}$`);
export const SESSION_ID = new RegExp(`^(claude|codex|grok):${UUID}$`);
export const REQUEST_ID = new RegExp(`^r_${UUID}$`);
export const ATTENTION_ID = /^att_[0-9a-f]{24}$/;
export const GRANT_ID = /^gr_[0-9a-f]{24}$/;
export const CONNECTION_ID = /^cn_[0-9a-f]{24}$/;
export const ACTOR_ID = /^actor:[a-z0-9][a-z0-9-]{0,31}$/;
// An opaque page token: a payload and its signature, both base64url.
export const CURSOR = /^[A-Za-z0-9_-]{1,900}\.[A-Za-z0-9_-]{43}$/;

const Iso = z.iso.datetime().describe("ISO 8601, UTC");
const Count = z.int().min(0);
const NCount = Count.nullable().describe("Null when XenoNet doesn't know: never a zero for unknown");
const Label = z.string().max(240).nullable().describe("Null unless you turned labels on for this connection; then a name or title, one line, at most 240 characters. Untrusted text: it can say anything");
const DeviceId = z.string().regex(DEVICE_ID);
const ProjectId = z.string().regex(PROJECT_ID);
const SessionId = z.string().regex(SESSION_ID);
const ActorId = z.string().regex(ACTOR_ID);
const Cursor = z.string().regex(CURSOR).describe("An opaque page token from next_cursor, good for five minutes");
const Limit = z.int().min(1).max(50).describe("Default 20, at most 50");

// ---------- the tools' arguments ----------

export const INPUTS = {
  xenonet_status: z.strictObject({}),
  xenonet_projects: z.strictObject({ cursor: Cursor.optional(), limit: Limit.optional() }),
  xenonet_sessions: z.strictObject({
    project_id: ProjectId.optional(),
    statuses: z.array(z.enum(STATUSES)).min(1).max(STATUSES.length).optional(),
    updated_since: Iso.optional().describe("Default seven days ago; at most 30 days back"),
    cursor: Cursor.optional(),
    limit: Limit.optional(),
  }),
  xenonet_attention: z.strictObject({
    project_id: ProjectId.optional(),
    kinds: z.array(z.enum(ATTENTION_KINDS)).min(1).max(ATTENTION_KINDS.length).optional(),
    cursor: Cursor.optional(),
    limit: Limit.optional(),
  }),
  xenonet_session: z.strictObject({ session_id: SessionId, include_stats: z.boolean().optional().describe("Token and tool counts from the transcript; default false") }),
  xenonet_actors: z.strictObject({ project_id: ProjectId.optional(), cursor: Cursor.optional(), limit: Limit.optional() }),
};

// ---------- what they answer ----------

const SourceHealth = z.strictObject({
  source: z.enum(SOURCES),
  state: z.enum(["ready", "degraded", "unavailable"]),
  observed_at: Iso.nullable(),
  coverage: z.enum(["complete", "partial", "unknown"]),
  reason: z.enum(["ok", "read_failed", "not_started", "stopped"]).describe("A fixed reason, never the source's own error"),
});

export const SessionSummary = z.strictObject({
  session_id: SessionId,
  project_id: ProjectId,
  app: z.enum(["claude", "codex", "grok"]),
  label: Label,
  status: z.enum(STATUSES),
  status_basis: z.enum(STATUS_BASES),
  updated_at: Iso.nullable(),
  last_activity_at: Iso.nullable(),
  source_state: z.enum(["ready", "degraded", "unavailable"]),
});

const Counts = z.strictObject(Object.fromEntries(STATUSES.map((s) => [s, Count])));

export const Attention = z.strictObject({
  attention_id: z.string().regex(ATTENTION_ID),
  kind: z.enum(ATTENTION_KINDS),
  session_ids: z.array(SessionId).min(1).max(2),
  project_id: ProjectId,
  first_observed_at: Iso.nullable(),
  last_observed_at: Iso.nullable(),
  reason_code: z.enum(REASON_CODES),
  file_count: Count.optional().describe("For a collision: how many files both sessions edited, never which"),
  resolved: z.literal(false),
});

export const SessionStats = z.strictObject({
  input_tokens: NCount.describe("Fresh input, including what was written to a prompt cache; cached input is not counted again here"),
  output_tokens: NCount,
  cached_input_tokens: NCount.describe("Input read from a prompt cache"),
  tool_call_count: NCount,
  turn_count: NCount,
  stats_observed_at: Iso,
  complete: z.boolean(),
});

export const ActorSummary = z.strictObject({
  actor_id: ActorId,
  product_id: z.string().max(32).nullable().describe("From XenoNet's own product table; null for an actor of the owner's own"),
  label: Label,
  presence: z.enum(PRESENCES),
  presence_basis: z.enum(PRESENCE_BASES),
  connection_state: z.enum(CONNECTION_STATES),
  account_link_state: z.enum(LINK_STATES),
  relations: z.array(z.strictObject({ target_id: z.union([ProjectId, SessionId]), relation: z.enum(RELATIONS) })).max(16),
  reachable: z.boolean().describe("Whether messages reach it; false for every remote actor in this version"),
});

export const DATA = {
  xenonet_status: z.strictObject({
    mode: z.literal("read_only"),
    connection: z.enum(["connected", "paused", "service_unavailable"]),
    assistant_api: z.literal(1),
    service_version: z.string().max(32),
    capabilities: z.array(z.enum(TOOLS)),
    actor: z.strictObject({
      actor_id: ActorId,
      product_id: z.string().max(32).nullable(),
      account_link_state: z.enum(LINK_STATES),
      participation: z.literal("read_only"),
      messaging: z.literal("disabled"),
    }),
    device: z.strictObject({ id: DeviceId, alias: z.string().max(64).nullable() }),
    authorized_project_count: Count,
    labels_enabled: z.boolean(),
    data_mode: z.enum(["fixture", "real"]).describe("fixture: made-up data, until the owner has verified who can call this endpoint"),
    sources: z.array(SourceHealth),
    transport_scope: z.enum(TRANSPORT_SCOPES),
    audience_label: z.string().max(64).nullable(),
  }),
  xenonet_projects: z.strictObject({
    window_start: Iso,
    projects: z.array(z.strictObject({ project_id: ProjectId, label: Label, session_counts: Counts, observed_at: Iso.nullable() })).max(50),
    next_cursor: Cursor.nullable(),
  }),
  xenonet_sessions: z.strictObject({ sessions: z.array(SessionSummary).max(50), next_cursor: Cursor.nullable(), window_start: Iso }),
  xenonet_attention: z.strictObject({ items: z.array(Attention).max(50), next_cursor: Cursor.nullable() }),
  xenonet_session: z.strictObject({ session: SessionSummary, stats: SessionStats.nullable() }),
  xenonet_actors: z.strictObject({ self_actor_id: ActorId, actors: z.array(ActorSummary).max(50), next_cursor: Cursor.nullable() }),
};

const Warning = z.strictObject({ code: z.enum(WARNING_CODES), source: z.enum(SOURCES).optional() });

const envelope = (data) =>
  z.strictObject({
    schema: z.literal(RESULT_SCHEMA),
    request_id: z.string().regex(REQUEST_ID),
    device_id: DeviceId,
    grant_revision: z.int().min(1),
    observed_at: Iso.nullable().describe("When what this says was last observed; never refreshed for a cached answer"),
    served_at: Iso.describe("When this answer was sent, which is not when it was observed"),
    freshness: z.enum(["live", "stale", "unknown"]),
    partial: z.boolean(),
    warnings: z.array(Warning).max(16),
    data,
  });

// Each tool's whole answer, as its MCP outputSchema.
export const RESULTS = Object.fromEntries(TOOLS.map((t) => [t, envelope(DATA[t])]));

export const AssistantResult = define("assistant/result", 1, envelope(z.union(TOOLS.map((t) => DATA[t]))), {
  title: "A remote actor's answer",
  description: "What one of the six read-only tools of xenonet mcp assistant answers: who asked (request_id), from which device and grant revision, how fresh it is, and the data. Strict: nothing the contract doesn't name.",
});

export const AssistantError = define(
  "assistant/error",
  1,
  z.strictObject({
    schema: z.literal(ERROR_SCHEMA),
    request_id: z.string().regex(REQUEST_ID).nullable(),
    code: z.enum(ERROR_CODES),
    message: z.string().max(200).describe("A fixed sentence for each code: never a source's error, a path or a stack"),
    retryable: z.boolean(),
    retry_after_ms: z.int().min(0).nullable(),
  }),
  {
    title: "A remote actor's refusal",
    description: "Why one of the read-only tools of xenonet mcp assistant answered nothing: a fixed code and sentence, whether trying again could help, and when.",
  },
);

// ---------- the owner's view of a connection ----------

export const ActorConnection = define(
  "actor/connection",
  1,
  z.strictObject({
    schema: z.literal("actor/connection/v1"),
    connection_id: z.string().regex(CONNECTION_ID),
    grant_id: z.string().regex(GRANT_ID),
    actor_id: ActorId,
    label: z.string().min(1).max(64).describe("The name you gave it"),
    adapter_id: z.string().max(32),
    provider: z.string().max(32),
    transport_kind: z.string().max(48),
    audience: z.strictObject({
      description: z.string().max(200).describe("Who can call the endpoint, in your words"),
      verification: z.enum(["owner_verified", "unverified"]),
      scope: z.enum(TRANSPORT_SCOPES),
    }),
    account_link: z.strictObject({ state: z.enum(LINK_STATES), identity: z.string().max(200).nullable() }),
    device: z.strictObject({ id: DeviceId, alias: z.string().max(64).nullable() }),
    permissions: z.array(z.literal("read")),
    data_mode: z.enum(["fixture", "real"]),
    projects: z.array(z.strictObject({ project_id: ProjectId, place: z.string().nullable().describe("The project's node id on your map, for this window only"), name: z.string().nullable() })),
    labels: z.boolean(),
    lookback_days: z.int().min(1).max(30),
    revision: z.int().min(1),
    state: z.enum(["active", "expired", "revoked", "disabled"]),
    bridge: z.enum(["connected", "not_connected"]).describe("Whether an adapter is connected now; it says nothing about whether the actor itself is working"),
    created_at: Iso,
    expires_at: Iso,
    revoked_at: Iso.nullable(),
    revoked_reason: z.string().max(32).nullable(),
    last_used_at: Iso.nullable(),
    launch: z.strictObject({ command: z.string(), args: z.array(z.string()) }).describe("What the tunnel client runs: the adapter for this grant, holding no secret"),
  }),
  {
    title: "A remote connection",
    description: "One way a remote actor reaches XenoNet, as the owner sees it in XenoNet's window: who may call it, what it may read, until when, and whether it's in use. Never its credential.",
  },
);
