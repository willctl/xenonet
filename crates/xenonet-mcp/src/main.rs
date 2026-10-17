//! The sessions' MCP server as a binary of its own, for this crate's stdio
//! tests; XenoNet runs it as `xenonet mcp`.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    xenonet_mcp::serve().await;
}
