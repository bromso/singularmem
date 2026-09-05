//! Library entry for `singularmem-mcp`. Exposes `serve()` so the binary
//! (`src/main.rs`) and the integration test can both launch the server
//! against the same code path.

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod prompts;
pub mod server;
pub mod tools;

pub use crate::config::Config;
pub use crate::error::{Error, Result};
pub use crate::server::serve;
pub use crate::tools::{
    handle_memory_get, handle_memory_graph_add, handle_memory_graph_invalidate,
    handle_memory_graph_query, handle_memory_graph_stats, handle_memory_graph_supersede,
    handle_memory_graph_timeline, handle_memory_ingest, handle_memory_list, handle_memory_retrieve,
    handle_memory_revisions, handle_memory_scopes, handle_memory_wakeup, MemoryGetArgs,
    MemoryGetOutput, MemoryGraphAddArgs, MemoryGraphInvalidateArgs, MemoryGraphOutput,
    MemoryGraphQueryArgs, MemoryGraphStatsArgs, MemoryGraphSupersedeArgs, MemoryGraphTimelineArgs,
    MemoryIngestArgs, MemoryIngestOutput, MemoryListArgs, MemoryListOutput, MemoryRetrieveArgs,
    MemoryRetrieveOutput, MemoryRevisionsArgs, MemoryRevisionsOutput, MemoryScopesOutput,
    MemoryWakeupArgs, MemoryWakeupOutput,
};
