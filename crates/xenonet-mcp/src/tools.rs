//! What the servers have in common, as the MCP SDK's McpServer does it for
//! the JS servers: the tools they list, the check of a call's arguments
//! against the tool's schema before it runs, and the shape of a tool's
//! answer and of its failure.

use crate::zod::Z;
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock, ListToolsResult, MetaObject, ProtocolVersion, Tool};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;
use serde_json::{json, Map, Value};

/// XenoNet's version, package.json's (build.rs), which the JS servers give
/// as theirs.
pub const VERSION: &str = env!("XENONET_VERSION");

/// One tool, as registerTool takes it.
pub struct Def {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub input: Z,
    pub output: Option<Z>,
    pub annotations: Value,
}

impl Def {
    /// As tools/list shows it.
    pub fn listed(&self) -> Tool {
        let mut t = Map::new();
        t.insert("name".into(), json!(self.name));
        t.insert("title".into(), json!(self.title));
        t.insert("description".into(), json!(self.description));
        t.insert("inputSchema".into(), Value::Object(self.input.input_schema()));
        t.insert("annotations".into(), self.annotations.clone());
        if let Some(out) = &self.output {
            t.insert("outputSchema".into(), Value::Object(out.output_schema()));
        }
        serde_json::from_value(Value::Object(t)).expect("a tool rmcp reads")
    }
}

/// The tool a call names and its arguments as its schema reads them, or
/// how the SDK answers instead: a protocol error for a tool there isn't,
/// and a tool error for arguments that don't fit. (The answer is as big as
/// rmcp's own, which is what a handler returns anyway.)
#[expect(clippy::result_large_err, reason = "the error is rmcp's own answer, which a handler returns as it is")]
pub fn check<'a>(defs: &'a [Def], name: &str, arguments: Option<Map<String, Value>>) -> Result<(&'a Def, Value), Result<CallToolResult, McpError>> {
    let Some(def) = defs.iter().find(|d| d.name == name) else {
        return Err(Err(McpError::invalid_params(format!("Tool {name} not found"), None)));
    };
    match def.input.parse(&Value::Object(arguments.unwrap_or_default())) {
        Ok(args) => Ok((def, args)),
        Err(issues) => Err(Ok(failed(format!("Input validation error: Invalid arguments for tool {name}: {issues}")))),
    }
}

/// A tool's answer: text, with no isError, as the JS servers answer.
pub fn text(text: String) -> CallToolResult {
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.is_error = None;
    r
}

/// A tool's answer with its structured content beside the same as text.
pub fn structured(text: String, value: Value) -> CallToolResult {
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.is_error = None;
    r.structured_content = Some(value);
    r
}

/// A tool that failed, saying why: a tool error the model reads, rather
/// than a protocol one.
pub fn failed(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

// A request of the 2026-07-28 revision, whose results name the server in
// their _meta, as the MCP SDK's do.
fn named(context: &RequestContext<RoleServer>, server: &str) -> Option<MetaObject> {
    let era = context.protocol_version().is_some_and(|v| v.as_str() >= ProtocolVersion::V_2026_07_28.as_str());
    era.then(|| MetaObject([("io.modelcontextprotocol/serverInfo".to_string(), json!({ "name": server, "version": VERSION }))].into_iter().collect()))
}

/// A call's answer as it goes.
pub fn answer(mut r: CallToolResult, context: &RequestContext<RoleServer>, server: &str) -> CallToolResponse {
    if let Some(meta) = named(context, server) {
        r.meta = Some(meta);
    }
    r.into()
}

/// The tools as tools/list gives them.
pub fn list(defs: &[Def], context: &RequestContext<RoleServer>, server: &str) -> ListToolsResult {
    let mut r = ListToolsResult::with_all_items(defs.iter().map(Def::listed).collect());
    r.meta = named(context, server);
    r
}

/// The arguments' values under these keys, as the JS passes them on (one
/// left out stays out).
pub fn pick(args: &Value, keys: &[(&str, &str)]) -> Map<String, Value> {
    keys.iter().filter_map(|(from, to)| args.get(*from).map(|v| (to.to_string(), v.clone()))).collect()
}
