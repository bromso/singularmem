//! MCP server: initialize handshake over stdio transport.
//!
//! This module implements the rmcp `ServerHandler` trait for the
//! `singularmem-mcp` server. Task 2 ships only the handshake and an
//! empty tool registry; tool registration lands in Task 4.

use rmcp::{
    model::{Implementation, ServerCapabilities, ServerInfo},
    transport::stdio,
    ServerHandler, ServiceExt,
};

use crate::Result;

/// Minimal handler for the Task 2 handshake.
///
/// Declares the `tools` capability so clients know to call `tools/list`
/// (which returns an empty list until Task 4 fills it in).
#[derive(Debug, Clone)]
struct SingularmemServer;

impl ServerHandler for SingularmemServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("singularmem-mcp", env!("CARGO_PKG_VERSION")),
        )
    }
}

/// Launch the MCP server on stdio. Blocks until the client closes the
/// connection or a fatal error occurs.
///
/// # Errors
///
/// Returns [`crate::Error::Io`] if the stdio transport fails to
/// initialise.
pub async fn serve() -> Result<()> {
    let service = SingularmemServer
        .serve(stdio())
        .await
        .map_err(std::io::Error::other)?;

    service.waiting().await.map_err(std::io::Error::other)?;

    Ok(())
}
