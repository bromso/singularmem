//! MCP server: initialize handshake + tool dispatch over stdio transport.
//!
//! This module implements the rmcp `ServerHandler` trait for the
//! `singularmem-mcp` server. Tool registration and routing land here in
//! Task 4; the `memory_retrieve` handler lives in `crate::tools::retrieve`.

use std::sync::Arc;

use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    transport::stdio,
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
};
use serde_json::json;

use crate::tools::{
    handle_memory_get, handle_memory_list, handle_memory_retrieve, handle_memory_revisions,
    MemoryGetArgs, MemoryListArgs, MemoryRetrieveArgs, MemoryRevisionsArgs,
};
use crate::{Config, Error, Result};

/// MCP server handler for Singularmem.
///
/// Implements `list_tools` (returning the `memory_retrieve` descriptor) and
/// `call_tool` (dispatching to [`handle_memory_retrieve`]).
#[derive(Debug, Clone)]
struct SingularmemServer {
    config: Arc<Config>,
}

impl SingularmemServer {
    const fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Build the `memory_retrieve` tool descriptor once.
    fn memory_retrieve_tool() -> Tool {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language query describing what kind of memory to retrieve."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of memory blocks to return. Defaults to 10.",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10
                },
                "adapter": {
                    "type": "string",
                    "description": "Which provider-specific format to render memories with.",
                    "enum": ["plain", "claude", "openai", "gemini"]
                }
            },
            "required": ["query"]
        });
        let schema_obj = schema.as_object().expect("schema is object").clone();
        Tool::new(
            "memory_retrieve",
            "Retrieve memories from the user's local Singularmem store that are relevant to a \
             query. Returns formatted context the model can use to ground its response. \
             Memories are private to this user and stored locally.",
            schema_obj,
        )
    }
}

impl ServerHandler for SingularmemServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("singularmem-mcp", env!("CARGO_PKG_VERSION")),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListToolsResult, McpError>>
           + rmcp::service::MaybeSendFuture
           + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(vec![
            Self::memory_retrieve_tool(),
            crate::tools::get::tool_descriptor(),
            crate::tools::list::tool_descriptor(),
            crate::tools::revisions::tool_descriptor(),
        ])))
    }

    #[allow(clippy::too_many_lines)]
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<CallToolResult, McpError>>
           + rmcp::service::MaybeSendFuture
           + '_ {
        let config = Arc::clone(&self.config);
        std::future::ready(match request.name.as_ref() {
            "memory_retrieve" => {
                // Parse arguments.
                let args_value = serde_json::Value::Object(request.arguments.unwrap_or_default());
                let args: MemoryRetrieveArgs = match serde_json::from_value(args_value) {
                    Ok(a) => a,
                    Err(e) => {
                        return std::future::ready(Err(McpError::invalid_params(
                            format!("failed to parse memory_retrieve arguments: {e}"),
                            None,
                        )));
                    }
                };

                match handle_memory_retrieve(&args, &config) {
                    Ok(out) => Ok(CallToolResult::success(vec![Content::text(out.text)])),
                    Err(Error::Retrieve(singularmem_retrieve::Error::EmptyQuery)) => {
                        Err(McpError::invalid_params("query must not be empty", None))
                    }
                    Err(Error::UnknownAdapter(name)) => Err(McpError::invalid_params(
                        format!("unknown adapter '{name}'; known adapters: plain, claude, openai, gemini"),
                        None,
                    )),
                    Err(Error::Search(singularmem_search::Error::NoIndexes)) => {
                        Err(McpError::internal_error(
                            "no memories indexed yet; run `singularmem ingest` first",
                            None,
                        ))
                    }
                    Err(other) => {
                        Err(McpError::internal_error(other.to_string(), None))
                    }
                }
            }
            "memory_get" => {
                let args_value = serde_json::Value::Object(request.arguments.unwrap_or_default());
                let args: MemoryGetArgs = match serde_json::from_value(args_value) {
                    Ok(a) => a,
                    Err(e) => {
                        return std::future::ready(Err(McpError::invalid_params(
                            format!("failed to parse memory_get arguments: {e}"),
                            None,
                        )));
                    }
                };
                match handle_memory_get(&args, &config) {
                    Ok(out) => Ok(CallToolResult::success(vec![Content::text(out.text)])),
                    Err(Error::InvalidId(msg)) => Err(McpError::invalid_params(
                        format!("invalid item ID: {msg}"),
                        None,
                    )),
                    Err(Error::Core(singularmem_core::Error::NotFound { id })) => Err(
                        McpError::invalid_params(format!("memory not found: {id}"), None),
                    ),
                    Err(other) => Err(McpError::internal_error(other.to_string(), None)),
                }
            }
            "memory_list" => {
                let args_value = serde_json::Value::Object(request.arguments.unwrap_or_default());
                let args: MemoryListArgs = match serde_json::from_value(args_value) {
                    Ok(a) => a,
                    Err(e) => {
                        return std::future::ready(Err(McpError::invalid_params(
                            format!("failed to parse memory_list arguments: {e}"),
                            None,
                        )));
                    }
                };
                match handle_memory_list(&args, &config) {
                    Ok(out) => Ok(CallToolResult::success(vec![Content::text(out.text)])),
                    Err(other) => Err(McpError::internal_error(other.to_string(), None)),
                }
            }
            "memory_revisions" => {
                let args_value = serde_json::Value::Object(request.arguments.unwrap_or_default());
                let args: MemoryRevisionsArgs = match serde_json::from_value(args_value) {
                    Ok(a) => a,
                    Err(e) => {
                        return std::future::ready(Err(McpError::invalid_params(
                            format!("failed to parse memory_revisions arguments: {e}"),
                            None,
                        )));
                    }
                };
                match handle_memory_revisions(&args, &config) {
                    Ok(out) => Ok(CallToolResult::success(vec![Content::text(out.text)])),
                    Err(Error::InvalidId(msg)) => Err(McpError::invalid_params(
                        format!("invalid item ID: {msg}"),
                        None,
                    )),
                    Err(Error::Core(singularmem_core::Error::NotFound { id })) => Err(
                        McpError::invalid_params(format!("memory not found: {id}"), None),
                    ),
                    Err(other) => Err(McpError::internal_error(other.to_string(), None)),
                }
            }
            _other => Err(McpError::method_not_found::<
                rmcp::model::CallToolRequestMethod,
            >()),
        })
    }
}

/// Launch the MCP server on stdio. Blocks until the client closes the
/// connection or a fatal error occurs.
///
/// # Errors
///
/// Returns [`crate::Error::Io`] if the stdio transport fails to
/// initialise.
pub async fn serve(config: Config) -> Result<()> {
    let handler = SingularmemServer::new(Arc::new(config));
    let service = handler
        .serve(stdio())
        .await
        .map_err(std::io::Error::other)?;

    service.waiting().await.map_err(std::io::Error::other)?;

    Ok(())
}
