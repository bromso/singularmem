//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;

pub mod get;
pub mod list;
pub mod retrieve;
pub mod revisions;

pub use crate::tools::get::{handle_memory_get, MemoryGetArgs, MemoryGetOutput};
pub use crate::tools::list::{handle_memory_list, MemoryListArgs, MemoryListOutput};
pub use crate::tools::retrieve::{
    handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput,
};
pub use crate::tools::revisions::{
    handle_memory_revisions, MemoryRevisionsArgs, MemoryRevisionsOutput,
};
