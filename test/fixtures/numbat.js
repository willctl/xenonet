// Made-up numbat event records (record_type "event"), in the shape of
// numbat's published NDJSON record schema, version 0.5.0
// (https://github.com/perplexityai/numbat, docs/schema). Nothing here is
// anyone's real data: the host, user and paths are invented.
import { ATLAS_DIR, LEDGER_DIR } from "./actor-map.js";

const endpoint = { hostname: "made-up-host", os: "darwin", arch: "arm64", username: "nova", uid: "501" };
let n = 0;
export function event(fields = {}) {
  n++;
  return {
    schema_version: "0.5.0",
    record_type: "event",
    run_id: "run-0001",
    endpoint,
    event_id: `evt-${String(n).padStart(4, "0")}`,
    source_agent: "gemini-cli",
    source_type: "hook",
    event_type: "tool.call",
    confidence: "high",
    evidence: { artifact_type: "hook" },
    ...fields,
  };
}

// A Gemini CLI session in atlas-api that starts, runs a command and edits
// two files (one by a relative path), and a Cursor session that edits a
// file outside every place on the map.
export const STREAM = (at) => {
  const t = (s) => new Date(at - s * 1000).toISOString();
  return [
    event({ event_type: "session.start", session_id: "g-0000001", project_path: ATLAS_DIR, timestamp: t(60) }),
    event({ event_type: "command.exec", session_id: "g-0000001", project_path: ATLAS_DIR, command: "npm test -- --token=made-up-secret", tool_name: "run_shell_command", timestamp: t(50) }),
    event({ event_type: "file.write", session_id: "g-0000001", project_path: ATLAS_DIR, file_path: `${ATLAS_DIR}/src/refunds.js`, tool_name: "write_file", content_preview: "made-up preview", timestamp: t(40) }),
    event({ event_type: "file.write", session_id: "g-0000001", project_path: ATLAS_DIR, file_path: "src/ledger.js", tool_name: "replace", timestamp: t(30) }),
    event({ source_agent: "cursor", event_type: "file.write", session_id: "c-0000001", project_path: "/tmp/elsewhere", file_path: "/tmp/elsewhere/notes.md", timestamp: t(20) }),
    // Left out: agents XenoNet reads itself, other record types, a version
    // XenoNet doesn't know.
    event({ source_agent: "claude-code", event_type: "file.write", session_id: "x", project_path: LEDGER_DIR, file_path: `${LEDGER_DIR}/a.js`, timestamp: t(10) }),
    event({ source_agent: "codex", event_type: "file.write", session_id: "y", project_path: LEDGER_DIR, file_path: `${LEDGER_DIR}/b.js`, timestamp: t(10) }),
    { schema_version: "0.5.0", record_type: "finding", run_id: "run-0001", endpoint, finding_id: "f-1" },
    event({ schema_version: "9.0.0", event_type: "file.write", file_path: `${LEDGER_DIR}/c.js`, timestamp: t(5) }),
  ];
};

export const ndjson = (records) => records.map((r) => JSON.stringify(r)).join("\n") + "\n";
