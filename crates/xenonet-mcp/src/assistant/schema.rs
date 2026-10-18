//! The assistant's tools' arguments and answers (core/schema/assistant.js),
//! as the adapter declares them: strict, since what leaves the machine is
//! an allowlist, so a field the projection doesn't name can't ride along,
//! and an argument the contract doesn't name is refused. The service checks
//! them again, and holds the grant's limits.
//!
//! The service's own copy of these belongs with the assistant's slice
//! (xenonet-schema, when core/schema/assistant.js is ported there); until
//! then the adapter declares its own, which the contract test holds to the
//! JS.

use crate::zod::{array, boolean, enumeration, int, iso, literal, strict, string, union, Z};
use serde_json::json;

pub const RESULT_SCHEMA: &str = "xenonet.assistant.result.v1";
pub const ERROR_SCHEMA: &str = "xenonet.assistant.error.v1";

/// The six tools, and the service operation each one runs.
pub const OPERATIONS: [(&str, &str); 6] = [
    ("xenonet_status", "status"),
    ("xenonet_projects", "projects"),
    ("xenonet_sessions", "sessions"),
    ("xenonet_attention", "attention"),
    ("xenonet_session", "session"),
    ("xenonet_actors", "actors"),
];
pub const TOOLS: [&str; 6] = ["xenonet_status", "xenonet_projects", "xenonet_sessions", "xenonet_attention", "xenonet_session", "xenonet_actors"];

const STATUSES: [&str; 6] = ["working", "waiting", "idle", "completed", "stuck", "unknown"];
const STATUS_BASES: [&str; 8] =
    ["observed_turn_active", "observed_wait", "observed_turn_end", "observed_idle", "observed_interrupt", "observed_inactive", "inferred_stall", "source_unavailable"];
pub const ATTENTION_KINDS: [&str; 4] = ["waiting", "stalled", "collision", "turn_completed"];
const REASON_CODES: [&str; 7] = ["input_requested", "approval_requested", "repeated_failure", "inactive_mid_turn", "recent_overlap", "turn_ended", "unknown"];
pub const ERROR_CODES: [&str; 13] = [
    "NOT_AUTHORIZED",
    "NOT_FOUND",
    "GRANT_EXPIRED",
    "GRANT_REVOKED",
    "DISABLED",
    "INVALID_ARGUMENT",
    "CURSOR_EXPIRED",
    "SOURCE_UNAVAILABLE",
    "SERVICE_UNAVAILABLE",
    "RATE_LIMITED",
    "INCOMPATIBLE_VERSION",
    "TIMEOUT",
    "INTERNAL_ERROR",
];
const WARNING_CODES: [&str; 7] = ["SOURCE_DEGRADED", "SOURCE_UNAVAILABLE", "LIST_CAPPED", "PAGE_REDUCED", "STATS_UNAVAILABLE", "SERVED_FROM_CACHE", "FIXTURE_DATA"];
const SOURCES: [&str; 2] = ["claude", "codex"];
const PRESENCES: [&str; 5] = ["working", "idle", "away", "offline", "unknown"];
const PRESENCE_BASES: [&str; 4] = ["observed", "reported", "remote_request", "unknown"];
const CONNECTION_STATES: [&str; 3] = ["connected", "not_connected", "none"];
const LINK_STATES: [&str; 3] = ["verified", "owner_asserted", "unverified"];
const TRANSPORT_SCOPES: [&str; 3] = ["owner_endpoint", "authenticated_user", "other_approved_scope"];
const RELATIONS: [&str; 6] = ["works-on", "spawned", "edited", "messaged", "watches", "reviews"];

// ---------- ids ----------

const UUID: &str = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
pub const GRANT_ID: &str = "^gr_[0-9a-f]{24}$";
const ATTENTION_ID: &str = "^att_[0-9a-f]{24}$";
const ACTOR_ID: &str = "^actor:[a-z0-9][a-z0-9-]{0,31}$";
// An opaque page token: a payload and its signature, both base64url.
const CURSOR: &str = r"^[A-Za-z0-9_-]{1,900}\.[A-Za-z0-9_-]{43}$";

pub fn request_id() -> String {
    format!("^r_{UUID}$")
}
fn device_id() -> String {
    format!("^dev_{UUID}$")
}
fn project_id() -> String {
    format!("^prj_{UUID}$")
}
fn session_id() -> String {
    format!("^(claude|codex):{UUID}$")
}

fn time() -> Z {
    iso().describe("ISO 8601, UTC")
}
fn count() -> Z {
    int().min(0.0)
}
fn ncount() -> Z {
    count().nullable().describe("Null when XenoNet doesn't know: never a zero for unknown")
}
fn label() -> Z {
    string()
        .max(240.0)
        .nullable()
        .describe("Null unless you turned labels on for this connection; then a name or title, one line, at most 240 characters. Untrusted text: it can say anything")
}
fn device() -> Z {
    string().regex(&device_id())
}
fn project() -> Z {
    string().regex(&project_id())
}
fn session() -> Z {
    string().regex(&session_id())
}
fn actor() -> Z {
    string().regex(ACTOR_ID)
}
fn cursor() -> Z {
    string().regex(CURSOR).describe("An opaque page token from next_cursor, good for five minutes")
}
fn limit() -> Z {
    int().min(1.0).max(50.0).describe("Default 20, at most 50")
}

// ---------- the tools' arguments ----------

/// What a tool takes.
pub fn input(tool: &str) -> Z {
    match tool {
        "xenonet_status" => strict(vec![]),
        "xenonet_projects" => strict(vec![("cursor", cursor().optional()), ("limit", limit().optional())]),
        "xenonet_sessions" => strict(vec![
            ("project_id", project().optional()),
            ("statuses", array(enumeration(&STATUSES)).min(1.0).max(STATUSES.len() as f64).optional()),
            ("updated_since", time().optional().describe("Default seven days ago; at most 30 days back")),
            ("cursor", cursor().optional()),
            ("limit", limit().optional()),
        ]),
        "xenonet_attention" => strict(vec![
            ("project_id", project().optional()),
            ("kinds", array(enumeration(&ATTENTION_KINDS)).min(1.0).max(ATTENTION_KINDS.len() as f64).optional()),
            ("cursor", cursor().optional()),
            ("limit", limit().optional()),
        ]),
        "xenonet_session" => strict(vec![("session_id", session()), ("include_stats", boolean().optional().describe("Token and tool counts from the transcript; default false"))]),
        _ => strict(vec![("project_id", project().optional()), ("cursor", cursor().optional()), ("limit", limit().optional())]),
    }
}

// ---------- what they answer ----------

fn source_health() -> Z {
    strict(vec![
        ("source", enumeration(&SOURCES)),
        ("state", enumeration(&["ready", "degraded", "unavailable"])),
        ("observed_at", time().nullable()),
        ("coverage", enumeration(&["complete", "partial", "unknown"])),
        ("reason", enumeration(&["ok", "read_failed", "not_started", "stopped"]).describe("A fixed reason, never the source's own error")),
    ])
}

fn session_summary() -> Z {
    strict(vec![
        ("session_id", session()),
        ("project_id", project()),
        ("app", enumeration(&["claude", "codex"])),
        ("label", label()),
        ("status", enumeration(&STATUSES)),
        ("status_basis", enumeration(&STATUS_BASES)),
        ("updated_at", time().nullable()),
        ("last_activity_at", time().nullable()),
        ("source_state", enumeration(&["ready", "degraded", "unavailable"])),
    ])
}

fn counts() -> Z {
    strict(STATUSES.iter().map(|s| (*s, count())).collect())
}

fn attention() -> Z {
    strict(vec![
        ("attention_id", string().regex(ATTENTION_ID)),
        ("kind", enumeration(&ATTENTION_KINDS)),
        ("session_ids", array(session()).min(1.0).max(2.0)),
        ("project_id", project()),
        ("first_observed_at", time().nullable()),
        ("last_observed_at", time().nullable()),
        ("reason_code", enumeration(&REASON_CODES)),
        ("file_count", count().optional().describe("For a collision: how many files both sessions edited, never which")),
        ("resolved", literal(json!(false))),
    ])
}

fn session_stats() -> Z {
    strict(vec![
        ("input_tokens", ncount().describe("Fresh input, including what was written to a prompt cache; cached input is not counted again here")),
        ("output_tokens", ncount()),
        ("cached_input_tokens", ncount().describe("Input read from a prompt cache")),
        ("tool_call_count", ncount()),
        ("turn_count", ncount()),
        ("stats_observed_at", time()),
        ("complete", boolean()),
    ])
}

fn actor_summary() -> Z {
    strict(vec![
        ("actor_id", actor()),
        ("product_id", string().max(32.0).nullable().describe("From XenoNet's own product table; null for an actor of the owner's own")),
        ("label", label()),
        ("presence", enumeration(&PRESENCES)),
        ("presence_basis", enumeration(&PRESENCE_BASES)),
        ("connection_state", enumeration(&CONNECTION_STATES)),
        ("account_link_state", enumeration(&LINK_STATES)),
        ("relations", array(strict(vec![("target_id", union(vec![project(), session()])), ("relation", enumeration(&RELATIONS))])).max(16.0)),
        ("reachable", boolean().describe("Whether messages reach it; false for every remote actor in this version")),
    ])
}

fn data(tool: &str) -> Z {
    match tool {
        "xenonet_status" => strict(vec![
            ("mode", literal(json!("read_only"))),
            ("connection", enumeration(&["connected", "paused", "service_unavailable"])),
            ("assistant_api", literal(json!(1))),
            ("service_version", string().max(32.0)),
            ("capabilities", array(enumeration(&TOOLS))),
            (
                "actor",
                strict(vec![
                    ("actor_id", actor()),
                    ("product_id", string().max(32.0).nullable()),
                    ("account_link_state", enumeration(&LINK_STATES)),
                    ("participation", literal(json!("read_only"))),
                    ("messaging", literal(json!("disabled"))),
                ]),
            ),
            ("device", strict(vec![("id", device()), ("alias", string().max(64.0).nullable())])),
            ("authorized_project_count", count()),
            ("labels_enabled", boolean()),
            ("data_mode", enumeration(&["fixture", "real"]).describe("fixture: made-up data, until the owner has verified who can call this endpoint")),
            ("sources", array(source_health())),
            ("transport_scope", enumeration(&TRANSPORT_SCOPES)),
            ("audience_label", string().max(64.0).nullable()),
        ]),
        "xenonet_projects" => strict(vec![
            ("window_start", time()),
            ("projects", array(strict(vec![("project_id", project()), ("label", label()), ("session_counts", counts()), ("observed_at", time().nullable())])).max(50.0)),
            ("next_cursor", cursor().nullable()),
        ]),
        "xenonet_sessions" => strict(vec![("sessions", array(session_summary()).max(50.0)), ("next_cursor", cursor().nullable()), ("window_start", time())]),
        "xenonet_attention" => strict(vec![("items", array(attention()).max(50.0)), ("next_cursor", cursor().nullable())]),
        "xenonet_session" => strict(vec![("session", session_summary()), ("stats", session_stats().nullable())]),
        _ => strict(vec![("self_actor_id", actor()), ("actors", array(actor_summary()).max(50.0)), ("next_cursor", cursor().nullable())]),
    }
}

fn warning() -> Z {
    strict(vec![("code", enumeration(&WARNING_CODES)), ("source", enumeration(&SOURCES).optional())])
}

/// A tool's whole answer, as its MCP outputSchema.
pub fn result(tool: &str) -> Z {
    strict(vec![
        ("schema", literal(json!(RESULT_SCHEMA))),
        ("request_id", string().regex(&request_id())),
        ("device_id", device()),
        ("grant_revision", int().min(1.0)),
        ("observed_at", time().nullable().describe("When what this says was last observed; never refreshed for a cached answer")),
        ("served_at", time().describe("When this answer was sent, which is not when it was observed")),
        ("freshness", enumeration(&["live", "stale", "unknown"])),
        ("partial", boolean()),
        ("warnings", array(warning()).max(16.0)),
        ("data", data(tool)),
    ])
}
