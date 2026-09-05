//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;

pub mod get;
pub mod graph;
pub mod ingest;
pub mod list;
pub mod retrieve;
pub mod revisions;
pub mod scopes;
pub mod wakeup;

pub use crate::tools::get::{handle_memory_get, MemoryGetArgs, MemoryGetOutput};
pub use crate::tools::graph::{
    handle_memory_graph_add, handle_memory_graph_entities, handle_memory_graph_history,
    handle_memory_graph_invalidate, handle_memory_graph_query, handle_memory_graph_stats,
    handle_memory_graph_supersede, handle_memory_graph_timeline, MemoryGraphAddArgs,
    MemoryGraphEntitiesArgs, MemoryGraphHistoryArgs, MemoryGraphInvalidateArgs, MemoryGraphOutput,
    MemoryGraphQueryArgs, MemoryGraphStatsArgs, MemoryGraphSupersedeArgs, MemoryGraphTimelineArgs,
};
pub use crate::tools::ingest::{handle_memory_ingest, MemoryIngestArgs, MemoryIngestOutput};
pub use crate::tools::list::{handle_memory_list, MemoryListArgs, MemoryListOutput};
pub use crate::tools::retrieve::{
    handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput,
};
pub use crate::tools::revisions::{
    handle_memory_revisions, MemoryRevisionsArgs, MemoryRevisionsOutput,
};
pub use crate::tools::scopes::{handle_memory_scopes, MemoryScopesOutput};
pub use crate::tools::wakeup::{handle_memory_wakeup, MemoryWakeupArgs, MemoryWakeupOutput};
