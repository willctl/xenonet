//! The MCP server over `rmcp`: lists the tools and runs them.
//!
//! Standard output carries only the protocol. Nothing here prints.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::IntoTransport;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};

use crate::error::Error;
use crate::tools::{definitions, BrowserTools, Part, ToolDef};

const NAME: &str = "eidolon-browser";
const INSTRUCTIONS: &str = "Tools for the web browser of one identity. Call take_snapshot to see a page and get a uid for each element, then act on a uid. Files go through the shared files folder: upload_file_by_uid takes a file from it, fetch_download puts a download in its downloads folder.";

struct Server {
    tools: Arc<BrowserTools>,
    defs: Vec<ToolDef>,
}

impl Server {
    fn listed(&self) -> Vec<Tool> {
        self.defs.iter().map(|d| Tool::new(Cow::Borrowed(d.name), Cow::Borrowed(d.description), Arc::new(d.schema.clone()))).collect()
    }
}

fn answer(parts: Vec<Part>) -> CallToolResult {
    let content = parts
        .into_iter()
        .map(|p| match p {
            Part::Text(t) => ContentBlock::text(t),
            Part::Image { data, mime } => ContentBlock::image(data, mime),
        })
        .collect();
    CallToolResult::success(content)
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.listed())))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        if !self.defs.iter().any(|d| d.name == request.name) {
            return Err(McpError::invalid_params(format!("There is no tool named {}.", request.name), None));
        }
        let arguments = request.arguments.map_or(serde_json::Value::Null, serde_json::Value::Object);
        // A tool that fails answers with its reason, which the agent reads
        // and acts on; it is not a protocol error.
        let result = match self.tools.call_tool(&request.name, arguments).await {
            Ok(parts) => answer(parts),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
        };
        Ok(result.into())
    }
}

/// Serves the tools on any transport until the client goes. A pair of an
/// async reader and writer is a transport, which is how the tests run it.
pub async fn serve_transport<T, E, A>(tools: BrowserTools, transport: T) -> Result<(), Error>
where
    T: IntoTransport<RoleServer, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let server = Server { tools: Arc::new(tools), defs: definitions() };
    let running = match server.serve(transport).await {
        Ok(r) => r,
        // A client that left before the handshake: nothing to say.
        Err(rmcp::service::ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => return Err(Error::Serve(e.to_string())),
    };
    let _ = running.waiting().await;
    Ok(())
}

/// Serves the tools on standard input and output until the client goes.
pub async fn serve_stdio(tools: BrowserTools) -> Result<(), Error> {
    serve_transport(tools, rmcp::transport::stdio()).await
}
