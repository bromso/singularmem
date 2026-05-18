# MCP Server — Write + Utility Tools (Sub-Project 4b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four tools (`memory_ingest`, `memory_get`, `memory_list`, `memory_revisions`) + a `--read-only` flag to the existing `singularmem-mcp` crate so MCP clients have full read+write parity with the `singularmem` CLI.

**Architecture:** Extends 4a's `singularmem-mcp` crate. Each new tool gets its own file under `src/tools/`. `Config` gains a `read_only: bool` field; `--read-only` (with `SINGULARMEM_READ_ONLY` env var) excludes `memory_ingest` from `tools/list` AND rejects direct calls. `memory_ingest` auto-wires Tantivy + USearch hooks the same way the root binary does (~30 lines of intentional duplication for v0).

**Tech Stack:** Rust 1.80, rmcp 1.7.0 (pinned in 4a), tokio (workspace), clap, serde_json, tracing. Reuses everything from 4a unchanged.

**Spec:** `docs/superpowers/specs/2026-05-18-mcp-server-4b-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-mcp/src/tools/get.rs` — `memory_get` handler + 3 unit tests.
- `crates/singularmem-mcp/src/tools/list.rs` — `memory_list` handler + 4 unit tests.
- `crates/singularmem-mcp/src/tools/revisions.rs` — `memory_revisions` handler + 4 unit tests.
- `crates/singularmem-mcp/src/tools/ingest.rs` — `memory_ingest` handler + auto-wiring helper + 6 unit tests.
- `crates/singularmem-mcp/src/tools/util.rs` — `open_store_for_reading()` helper shared by all read handlers.
- `crates/singularmem-mcp/tests/full_write_read_cycle.rs` — end-to-end ingest→get→list→revisions→retrieve integration test.
- `crates/singularmem-mcp/tests/read_only_mode.rs` — `--read-only` filtering + rejection integration test.

**Modified:**
- `crates/singularmem-mcp/src/error.rs` — adds `Error::ReadOnly` and `Error::InvalidId(String)` variants.
- `crates/singularmem-mcp/src/config.rs` — `Config` gains `read_only: bool` field; `Config::new` gains a fourth parameter; one new unit test.
- `crates/singularmem-mcp/src/main.rs` — adds `--read-only` clap arg with `SINGULARMEM_READ_ONLY` env binding; passes through to `Config::new`.
- `crates/singularmem-mcp/src/server.rs` — `list_tools` extends to 5 descriptors (4 when read-only); `call_tool` dispatches all 5 with read-only rejection on `memory_ingest`.
- `crates/singularmem-mcp/src/tools/mod.rs` — re-exports the four new handler functions and `Args`/`Output` types.
- `crates/singularmem-mcp/src/tools/retrieve.rs` — refactored to use the new `open_store_for_reading()` helper so it honours `--read-only`.
- `crates/singularmem-mcp/src/lib.rs` — re-exports updated for new types.
- `crates/singularmem-mcp/tests/mcp_handshake.rs` — `tools/list` assertion updated from 1 tool to 5.
- `crates/singularmem-mcp/README.md` — status banner, "Available tools", config table, troubleshooting, "What's coming next".
- `docs/mcp-server.md` — extended layering diagram, expanded tools list, new "Read-only mode" subsection, promoted roadmap.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"`).

---

## Task 1: Foundation — Error variants, `Config.read_only`, `--read-only` flag, shared helper

**Why first:** Every subsequent task needs `Config.read_only`, the error variants, and the `open_store_for_reading` helper. Land them first so Tasks 2–5 can compose against a stable foundation.

**Files:**
- Modify: `crates/singularmem-mcp/src/error.rs`
- Modify: `crates/singularmem-mcp/src/config.rs`
- Modify: `crates/singularmem-mcp/src/main.rs`
- Create: `crates/singularmem-mcp/src/tools/util.rs`
- Modify: `crates/singularmem-mcp/src/tools/mod.rs`
- Modify: `crates/singularmem-mcp/src/tools/retrieve.rs` (small refactor to use the new helper)

- [ ] **Step 1: Add two new `Error` variants**

Open `crates/singularmem-mcp/src/error.rs`. Add two variants to the `pub enum Error` block (place them logically — `ReadOnly` near top, `InvalidId` near the existing `Io` variant):

```rust
    /// Server is launched in read-only mode and the request would write.
    #[error("server is read-only; memory_ingest is disabled")]
    ReadOnly,

    /// Could not parse an ItemId argument.
    #[error("invalid item ID: {0}")]
    InvalidId(String),
```

- [ ] **Step 2: Run clippy/test to confirm the variants compile cleanly**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings. The new variants are unused for now (Tasks 2-5 use them); rustc's dead-code lint doesn't fire on enum variants by default.

- [ ] **Step 3: Add `read_only: bool` to `Config` and update `Config::new`**

Open `crates/singularmem-mcp/src/config.rs`. Update the struct:

```rust
pub struct Config {
    /// Path to the `SQLite` store backing the server.
    pub store_path: PathBuf,
    /// Default adapter name when the client doesn't specify one.
    /// Must be the `name()` of one of `known_adapters`.
    pub default_adapter: String,
    /// Registered adapters available to clients. Mirrors the root
    /// binary's `known_adapters()` registry.
    pub known_adapters: Vec<Box<dyn Adapter>>,
    /// When true, the server omits `memory_ingest` from `tools/list`
    /// and rejects direct calls to it. Read tools open the store with
    /// SQLite read-only mode as a third safety layer.
    pub read_only: bool,
}
```

Update the manual `Debug` impl to include the new field:

```rust
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let adapter_names: Vec<&str> = self.known_adapters.iter().map(|a| a.name()).collect();
        f.debug_struct("Config")
            .field("store_path", &self.store_path)
            .field("default_adapter", &self.default_adapter)
            .field("known_adapters", &adapter_names)
            .field("read_only", &self.read_only)
            .finish()
    }
}
```

Update `Config::new` signature and body:

```rust
impl Config {
    /// Build a config from CLI args. Adapter registry is hard-coded
    /// to the four constitutional Principle II providers.
    #[must_use]
    pub fn new(
        store_path: PathBuf,
        default_adapter: String,
        read_only: bool,
    ) -> Self {
        Self {
            store_path,
            default_adapter,
            known_adapters: vec![
                Box::new(singularmem_retrieve::PlainAdapter),
                Box::new(singularmem_adapter_claude::ClaudeAdapter),
                Box::new(singularmem_adapter_openai::OpenAiAdapter),
                Box::new(singularmem_adapter_gemini::GeminiAdapter),
            ],
            read_only,
        }
    }
}
```

- [ ] **Step 4: Update the existing 4a Config unit tests to use the new signature**

In `crates/singularmem-mcp/src/config.rs`, the existing `#[cfg(test)] mod tests` block has three tests that call `Config::new(path, adapter)`. Update each to pass `false` as the third argument:

```rust
    #[test]
    fn config_new_registers_four_adapters() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "plain".to_string(), false);
        // ... rest unchanged
    }

    #[test]
    fn config_new_preserves_store_path() {
        let cfg = Config::new(PathBuf::from("/tmp/custom.db"), "claude".to_string(), false);
        // ... rest unchanged
    }

    #[test]
    fn config_new_preserves_default_adapter() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "openai".to_string(), false);
        // ... rest unchanged
    }
```

- [ ] **Step 5: Add the new `read_only` unit test**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-mcp/src/config.rs`:

```rust
    #[test]
    fn config_new_preserves_read_only_flag() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "plain".to_string(), true);
        assert!(cfg.read_only);

        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "plain".to_string(), false);
        assert!(!cfg.read_only);
    }
```

- [ ] **Step 6: Run the config tests**

Run: `cargo test -p singularmem-mcp --lib config::tests`
Expected: PASS for all 4 tests (3 existing updated + 1 new).

- [ ] **Step 7: Add `--read-only` clap arg in main.rs**

Open `crates/singularmem-mcp/src/main.rs`. Find the `Args` struct (after the other `--store` / `--default-adapter` / `--log-level` fields). Add:

```rust
    /// Open the store in read-only mode. When set, `memory_ingest` is
    /// omitted from `tools/list` AND direct calls are rejected. Read
    /// tools open the store with SQLite read-only mode.
    #[arg(long, env = "SINGULARMEM_READ_ONLY", default_value_t = false)]
    read_only: bool,
```

Then find the `Config::new(...)` call in `main()`. Update it to pass `args.read_only`:

```rust
    let config = singularmem_mcp::Config::new(
        store_path,
        args.default_adapter.as_str().to_string(),
        args.read_only,
    );
```

- [ ] **Step 8: Create `src/tools/util.rs` with the shared helper**

Create `crates/singularmem-mcp/src/tools/util.rs`:

```rust
//! Shared helpers used by tool handlers.

use singularmem_core::{Store, StoreOptions};

use crate::{Config, Result};

/// Open the store for read-side handlers, honouring `config.read_only`.
/// When read-only, SQLite is opened with `read_only=true` as a third
/// safety layer (in addition to the dispatch-level + list-level guards).
///
/// # Errors
///
/// Returns whatever error `Store::open` / `Store::open_with_options`
/// raises (e.g., I/O, malformed SQLite file).
pub(crate) fn open_store_for_reading(config: &Config) -> Result<Store> {
    if config.read_only {
        Ok(Store::open_with_options(
            &config.store_path,
            StoreOptions { read_only: true },
        )?)
    } else {
        Ok(Store::open(&config.store_path)?)
    }
}
```

- [ ] **Step 9: Wire `util` into `tools/mod.rs`**

Open `crates/singularmem-mcp/src/tools/mod.rs`. Add the `util` module declaration alongside the existing `retrieve` declaration. The current content is something like:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub mod retrieve;

pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
```

Update to:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;
pub mod retrieve;

pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
```

(util is `pub(crate)` because the helper is an internal implementation detail; external consumers don't need it.)

- [ ] **Step 10: Refactor `handle_memory_retrieve` to use the helper**

Open `crates/singularmem-mcp/src/tools/retrieve.rs`. Find the line that opens the store (currently `let store = Store::open(&config.store_path)?;`). Replace it with:

```rust
    let store = crate::tools::util::open_store_for_reading(config)?;
```

This is a 1-line refactor. The existing 7 tests should still pass because they all create configs with `read_only: false` (which is the default after Step 4's updates).

- [ ] **Step 11: Run all crate tests**

Run: `cargo test -p singularmem-mcp`
Expected: PASS for all 11 existing tests (10 from 4a + 1 new) plus the `error::tests` (1 from 4a; 2 if Task 2 of 4a's plan added one) — exact count depends on prior state but everything should be green.

- [ ] **Step 12: Run clippy and fmt**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 13: Smoke-test that the server still works**

Run:

```bash
{ echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'; sleep 0.5; } | cargo run --quiet -p singularmem-mcp 2>/dev/null | head -1 | grep -c '"name":"singularmem-mcp"'
```

Expected: `1` — server still handshakes correctly after the Config + retrieve refactor.

- [ ] **Step 14: Commit**

```bash
git add crates/singularmem-mcp/
git commit -s -m "feat(mcp): foundation for 4b — Error variants, Config.read_only, --read-only flag, shared helper

Adds Error::ReadOnly + Error::InvalidId variants for upcoming 4b
write/utility tools. Config gains a read_only: bool field with
Config::new updated accordingly. New --read-only clap arg (with
SINGULARMEM_READ_ONLY env var) feeds Config from main.rs.

New shared helper tools::util::open_store_for_reading(config)
opens the store with SQLite read-only mode when config.read_only
is set, providing the third defense layer (in addition to the
list_tools + call_tool filters added in Task 3).

memory_retrieve refactored to use the helper — its existing
behaviour is preserved (all 4a tests still pass) and it now
honours --read-only mode.

One new config unit test (config_new_preserves_read_only_flag);
existing 3 config tests updated to use new Config::new signature.
Smoke test confirms initialize handshake still works."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: Three utility read tools — `memory_get`, `memory_list`, `memory_revisions`

**Why next:** These three tools share the same shape — open store via the helper, call a `Store::*` method, format the output as text. Bundling them in one task keeps the per-tool boilerplate efficient. Total: 3 handlers + 11 unit tests + 3 server.rs registrations.

**Files:**
- Create: `crates/singularmem-mcp/src/tools/get.rs`
- Create: `crates/singularmem-mcp/src/tools/list.rs`
- Create: `crates/singularmem-mcp/src/tools/revisions.rs`
- Modify: `crates/singularmem-mcp/src/tools/mod.rs` (declare + re-export the three new modules)
- Modify: `crates/singularmem-mcp/src/server.rs` (register 3 new tools in `list_tools` + `call_tool`)
- Modify: `crates/singularmem-mcp/src/lib.rs` (re-export new public types)

### Step 1 — Step 4: Create `memory_get`

- [ ] **Step 1: Write `src/tools/get.rs` (handler + 3 tests)**

Create `crates/singularmem-mcp/src/tools/get.rs`:

```rust
//! `memory_get` tool — fetch a single memory by ID.

use std::str::FromStr;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::ItemId;

use crate::tools::util::open_store_for_reading;
use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_get` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGetArgs {
    /// ULID of the memory to fetch (26 characters, Crockford base32).
    pub id: String,
}

/// Handler output: a single text block with the memory's content +
/// metadata. The MCP transport layer wraps this in a `CallToolResult`.
#[derive(Debug, Clone)]
pub struct MemoryGetOutput {
    /// Formatted text block per the spec.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_get`. Wired into
/// `ServerHandler::list_tools` in `src/server.rs`.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "ULID of the memory to fetch (26 characters, Crockford base32)."
            }
        },
        "required": ["id"]
    });
    Tool::new(
        "memory_get",
        "Fetch a single memory by ID. Returns the memory's content and metadata as text.",
        std::sync::Arc::new(schema.as_object().unwrap().clone()),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_get`.
///
/// # Errors
///
/// - [`Error::InvalidId`] when `args.id` doesn't parse as a ULID.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::NotFound`]
///   when no item with that ID exists.
/// - [`Error::Core`] for other store I/O failures.
pub fn handle_memory_get(args: MemoryGetArgs, config: &Config) -> Result<MemoryGetOutput> {
    let id = ItemId::from_str(&args.id).map_err(|e| Error::InvalidId(e.to_string()))?;
    let store = open_store_for_reading(config)?;
    let item = store.get(id)?;

    let mut text = String::new();
    text.push_str(&format!("Memory {}\n", item.id));
    text.push_str(&format!("Created: {}\n", item.created_at));
    if let Some(source) = &item.source {
        text.push_str(&format!("Source: {source}\n"));
    }
    if !item.tags.is_empty() {
        text.push_str(&format!("Tags: {}\n", item.tags.join(", ")));
    }
    text.push('\n');
    text.push_str(&item.content);

    Ok(MemoryGetOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    fn seeded(default_adapter: &str, read_only: bool) -> (TempDir, Config, ItemId) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let mut item = NewItem::text("the quick brown fox jumps over the lazy dog");
        item.tags = vec!["fox".to_string(), "animals".to_string()];
        item.source = Some("test-source".to_string());
        let stored = store.ingest(item).unwrap();
        drop(store);
        let config = Config::new(store_path, default_adapter.to_string(), read_only);
        (dir, config, stored.id)
    }

    #[test]
    fn get_returns_full_item() {
        let (_dir, config, id) = seeded("plain", false);
        let args = MemoryGetArgs {
            id: id.to_string(),
        };
        let out = handle_memory_get(args, &config).expect("ok");
        assert!(out.text.contains(&format!("Memory {id}")), "missing ID header: {}", out.text);
        assert!(out.text.contains("the quick brown fox"), "missing content: {}", out.text);
        assert!(out.text.contains("Source: test-source"), "missing source: {}", out.text);
        assert!(out.text.contains("Tags: fox, animals"), "missing tags: {}", out.text);
    }

    #[test]
    fn get_invalid_ulid_returns_error() {
        let (_dir, config, _) = seeded("plain", false);
        let args = MemoryGetArgs {
            id: "not-a-ulid".to_string(),
        };
        let r = handle_memory_get(args, &config);
        assert!(
            matches!(r, Err(Error::InvalidId(_))),
            "expected InvalidId, got {r:?}"
        );
    }

    #[test]
    fn get_not_found_returns_error() {
        let (_dir, config, _) = seeded("plain", false);
        // Valid ULID format but doesn't exist in the store.
        let args = MemoryGetArgs {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        };
        let r = handle_memory_get(args, &config);
        assert!(
            matches!(r, Err(Error::Core(singularmem_core::Error::NotFound { .. }))),
            "expected Core(NotFound), got {r:?}"
        );
    }
}
```

- [ ] **Step 2: Run the get tests to verify they pass**

Run: `cargo test -p singularmem-mcp --lib tools::get::tests`
Expected: PASS for all 3 tests (`get_returns_full_item`, `get_invalid_ulid_returns_error`, `get_not_found_returns_error`).

### Step 3 — Step 4: Create `memory_list`

- [ ] **Step 3: Write `src/tools/list.rs` (handler + 4 tests)**

Create `crates/singularmem-mcp/src/tools/list.rs`:

```rust
//! `memory_list` tool — enumerate memories, optionally filtered by tag.

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use crate::tools::util::open_store_for_reading;
use crate::{Config, Result};

/// JSON-deserialised arguments for the `memory_list` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryListArgs {
    /// AND-filter tags. When present, only items containing every
    /// listed tag are returned.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Maximum number of items to return. Clamped to `[1, 100]`.
    /// Default: 50.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Handler output: a single text block with one line per item.
#[derive(Debug, Clone)]
pub struct MemoryListOutput {
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_list`.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "AND-filter tags."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "default": 50,
                "description": "Maximum number of items to return."
            }
        },
        "required": []
    });
    Tool::new(
        "memory_list",
        "Enumerate memories in the store, optionally filtered by tag (AND-semantics). Returns a compact listing with IDs and content snippets.",
        std::sync::Arc::new(schema.as_object().unwrap().clone()),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_list`.
///
/// # Errors
///
/// Returns [`Error::Core`] for store I/O failures.
pub fn handle_memory_list(args: MemoryListArgs, config: &Config) -> Result<MemoryListOutput> {
    let limit = args.limit.unwrap_or(50).clamp(1, 100);
    let store = open_store_for_reading(config)?;

    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        match &args.tags {
            Some(tags) if !tags.is_empty() => {
                let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
                Box::new(store.list_by_tags(&tag_refs)?)
            }
            _ => Box::new(store.list()?),
        };

    let items: Vec<singularmem_core::Item> = iter
        .take(limit)
        .collect::<singularmem_core::Result<Vec<_>>>()?;

    let count = items.len();
    let mut text = String::new();
    text.push_str(&format!(
        "Found {count} memor{} (limit {limit}):\n\n",
        if count == 1 { "y" } else { "ies" }
    ));
    for item in &items {
        let snippet: String = item.content.chars().take(80).collect();
        let snippet_one_line = snippet.replace('\n', " ");
        text.push_str(&format!("{}: {snippet_one_line}\n", item.id));
    }

    Ok(MemoryListOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    fn seeded(n: usize, with_tags: bool) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        for i in 0..n {
            let mut item = NewItem::text(format!("seed memory number {i}"));
            if with_tags {
                item.tags = if i % 2 == 0 {
                    vec!["even".to_string()]
                } else {
                    vec!["odd".to_string()]
                };
            }
            store.ingest(item).unwrap();
        }
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config)
    }

    #[test]
    fn list_returns_all_when_no_filter() {
        let (_dir, config) = seeded(3, false);
        let args = MemoryListArgs { tags: None, limit: None };
        let out = handle_memory_list(args, &config).expect("ok");
        assert!(out.text.contains("Found 3 memories"), "missing count: {}", out.text);
        assert_eq!(out.text.matches("seed memory number").count(), 3);
    }

    #[test]
    fn list_respects_tag_filter() {
        let (_dir, config) = seeded(6, true);
        let args = MemoryListArgs {
            tags: Some(vec!["even".to_string()]),
            limit: None,
        };
        let out = handle_memory_list(args, &config).expect("ok");
        assert!(out.text.contains("Found 3 memories"), "expected 3 even, got: {}", out.text);
    }

    #[test]
    fn list_caps_limit_at_100() {
        let (_dir, config) = seeded(150, false);
        let args = MemoryListArgs { tags: None, limit: Some(500) };
        let out = handle_memory_list(args, &config).expect("ok");
        assert!(out.text.contains("Found 100"), "expected limit clamp to 100, got: {}", out.text);
    }

    #[test]
    fn list_default_limit_50() {
        let (_dir, config) = seeded(100, false);
        let args = MemoryListArgs { tags: None, limit: None };
        let out = handle_memory_list(args, &config).expect("ok");
        assert!(out.text.contains("Found 50"), "expected default limit 50, got: {}", out.text);
    }
}
```

- [ ] **Step 4: Run the list tests to verify they pass**

Run: `cargo test -p singularmem-mcp --lib tools::list::tests`
Expected: PASS for all 4 tests.

### Step 5 — Step 6: Create `memory_revisions`

- [ ] **Step 5: Write `src/tools/revisions.rs` (handler + 4 tests)**

Create `crates/singularmem-mcp/src/tools/revisions.rs`:

```rust
//! `memory_revisions` tool — walk the supersedes chain newest-first.

use std::str::FromStr;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::ItemId;

use crate::tools::util::open_store_for_reading;
use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_revisions` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRevisionsArgs {
    /// ULID of any item in the chain.
    pub id: String,
}

/// Handler output: a single text block with header + one line per revision.
#[derive(Debug, Clone)]
pub struct MemoryRevisionsOutput {
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_revisions`.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "ULID of any item in the chain."
            }
        },
        "required": ["id"]
    });
    Tool::new(
        "memory_revisions",
        "Walk the supersedes chain for a memory, newest-first. Returns each revision in the chain with ID and content snippet.",
        std::sync::Arc::new(schema.as_object().unwrap().clone()),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_revisions`.
///
/// # Errors
///
/// - [`Error::InvalidId`] when `args.id` doesn't parse as a ULID.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::NotFound`]
///   when no item with that ID exists.
/// - [`Error::Core`] for other store I/O failures.
pub fn handle_memory_revisions(
    args: MemoryRevisionsArgs,
    config: &Config,
) -> Result<MemoryRevisionsOutput> {
    let id = ItemId::from_str(&args.id).map_err(|e| Error::InvalidId(e.to_string()))?;
    let store = open_store_for_reading(config)?;
    let history = store.revision_history(id)?;

    let count = history.len();
    let mut text = String::new();
    text.push_str(&format!(
        "Revisions of {} ({count} item{}, newest first):\n\n",
        args.id,
        if count == 1 { "" } else { "s" }
    ));
    for item in &history {
        let snippet: String = item.content.chars().take(80).collect();
        let snippet_one_line = snippet.replace('\n', " ");
        text.push_str(&format!("{}: {snippet_one_line}\n", item.id));
    }

    Ok(MemoryRevisionsOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    fn seeded_with_chain(depth: usize) -> (TempDir, Config, ItemId) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let mut prev_id: Option<ItemId> = None;
        let mut newest_id: Option<ItemId> = None;
        for i in 0..depth {
            let mut item = NewItem::text(format!("revision {i}"));
            item.supersedes = prev_id;
            let stored = store.ingest(item).unwrap();
            prev_id = Some(stored.id);
            newest_id = Some(stored.id);
        }
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config, newest_id.unwrap())
    }

    #[test]
    fn revisions_walks_chain_newest_first() {
        let (_dir, config, newest) = seeded_with_chain(3);
        let args = MemoryRevisionsArgs { id: newest.to_string() };
        let out = handle_memory_revisions(args, &config).expect("ok");
        assert!(out.text.contains("3 items, newest first"), "missing header count: {}", out.text);
        // 3 revision lines + 1 header line + 1 blank line.
        assert_eq!(out.text.matches("revision ").count(), 3);
        // Newest content "revision 2" should appear before "revision 0" in the output.
        let pos_2 = out.text.find("revision 2").expect("revision 2 present");
        let pos_0 = out.text.find("revision 0").expect("revision 0 present");
        assert!(pos_2 < pos_0, "newest should appear first: {}", out.text);
    }

    #[test]
    fn revisions_for_single_item_returns_one() {
        let (_dir, config, only) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs { id: only.to_string() };
        let out = handle_memory_revisions(args, &config).expect("ok");
        assert!(out.text.contains("(1 item, newest first)"), "expected singular: {}", out.text);
    }

    #[test]
    fn revisions_invalid_ulid_returns_error() {
        let (_dir, config, _) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs { id: "not-a-ulid".to_string() };
        let r = handle_memory_revisions(args, &config);
        assert!(matches!(r, Err(Error::InvalidId(_))), "expected InvalidId, got {r:?}");
    }

    #[test]
    fn revisions_not_found_returns_error() {
        let (_dir, config, _) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        };
        let r = handle_memory_revisions(args, &config);
        assert!(
            matches!(r, Err(Error::Core(singularmem_core::Error::NotFound { .. }))),
            "expected Core(NotFound), got {r:?}"
        );
    }
}
```

- [ ] **Step 6: Run the revisions tests to verify they pass**

Run: `cargo test -p singularmem-mcp --lib tools::revisions::tests`
Expected: PASS for all 4 tests.

### Step 7 — Step 10: Wire the new modules

- [ ] **Step 7: Update `src/tools/mod.rs` to declare and re-export the new modules**

Open `crates/singularmem-mcp/src/tools/mod.rs`. The current content after Task 1:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;
pub mod retrieve;

pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
```

Replace with:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;

pub mod get;
pub mod list;
pub mod retrieve;
pub mod revisions;

pub use crate::tools::get::{handle_memory_get, MemoryGetArgs, MemoryGetOutput};
pub use crate::tools::list::{handle_memory_list, MemoryListArgs, MemoryListOutput};
pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
pub use crate::tools::revisions::{handle_memory_revisions, MemoryRevisionsArgs, MemoryRevisionsOutput};
```

- [ ] **Step 8: Update `src/lib.rs` re-exports**

Open `crates/singularmem-mcp/src/lib.rs`. Find the re-export block. Update it to include the new types:

```rust
pub use crate::tools::{
    handle_memory_get, handle_memory_list, handle_memory_retrieve, handle_memory_revisions,
    MemoryGetArgs, MemoryGetOutput, MemoryListArgs, MemoryListOutput, MemoryRetrieveArgs,
    MemoryRetrieveOutput, MemoryRevisionsArgs, MemoryRevisionsOutput,
};
```

(Adjust to match the existing pattern — likely a single multi-line `pub use` block.)

- [ ] **Step 9: Register the three new tools in `src/server.rs`**

Open `crates/singularmem-mcp/src/server.rs`. Find the `list_tools` override. It currently returns a vec containing only `memory_retrieve`'s descriptor. Update to:

```rust
        let tools = vec![
            crate::tools::retrieve::tool_descriptor(),
            crate::tools::get::tool_descriptor(),
            crate::tools::list::tool_descriptor(),
            crate::tools::revisions::tool_descriptor(),
        ];
```

(The 4a code likely uses a different invocation pattern — adapt to whatever shape it has. The key change: 4 descriptors in the vec, in the specified order. `memory_ingest` joins in Task 3.)

Find the `call_tool` override. It currently has a single match arm for `"memory_retrieve"`. Update the match to dispatch all 4 tools:

```rust
        match name {
            "memory_retrieve" => {
                let args: crate::tools::MemoryRetrieveArgs = parse_args(arguments)?;
                let out = crate::tools::handle_memory_retrieve(args, &self.config)
                    .map_err(map_to_mcp_error)?;
                Ok(CallToolResult::text(out.text))
            }
            "memory_get" => {
                let args: crate::tools::MemoryGetArgs = parse_args(arguments)?;
                let out = crate::tools::handle_memory_get(args, &self.config)
                    .map_err(map_to_mcp_error)?;
                Ok(CallToolResult::text(out.text))
            }
            "memory_list" => {
                let args: crate::tools::MemoryListArgs = parse_args(arguments)?;
                let out = crate::tools::handle_memory_list(args, &self.config)
                    .map_err(map_to_mcp_error)?;
                Ok(CallToolResult::text(out.text))
            }
            "memory_revisions" => {
                let args: crate::tools::MemoryRevisionsArgs = parse_args(arguments)?;
                let out = crate::tools::handle_memory_revisions(args, &self.config)
                    .map_err(map_to_mcp_error)?;
                Ok(CallToolResult::text(out.text))
            }
            _ => Err(rmcp::Error::MethodNotFound(format!("unknown tool: {name}"))),
        }
```

Adapt the helper function names (`parse_args`, `map_to_mcp_error`) to whatever 4a actually used. The pattern is: deserialize args, call handler, map errors to MCP, wrap success in `CallToolResult::text`. If 4a inlined these patterns rather than extracting helpers, repeat the inline pattern four times.

**`Error::InvalidId` → MCP error mapping.** Add to `map_to_mcp_error` (or wherever 4a maps errors to rmcp::Error):

```rust
        Error::InvalidId(msg) => rmcp::Error::InvalidParams(format!("invalid item ID: {msg}")),
```

Also map `Error::Core(NotFound { .. })` → `InvalidParams` with "memory not found: \<id\>", if 4a didn't already cover it.

- [ ] **Step 10: Verify all tests still pass (existing + new)**

Run: `cargo test -p singularmem-mcp`
Expected: PASS for all crate tests. New count: ~22 (4a's 11 + 11 new from this task).

- [ ] **Step 11: Run clippy + fmt**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 12: Smoke-test tools/list shows 4 tools now**

Run:

```bash
{ echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'; \
  echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'; \
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'; \
  sleep 0.5; } | cargo run --quiet -p singularmem-mcp 2>/dev/null
```

Expected: at least two JSON responses. The second (id=2) should include `memory_retrieve`, `memory_get`, `memory_list`, and `memory_revisions` in the tools array.

- [ ] **Step 13: Commit**

```bash
git add crates/singularmem-mcp/
git commit -s -m "feat(mcp): memory_get + memory_list + memory_revisions tools + 11 unit tests

Three utility read tools all following the same shape: open store
via tools::util::open_store_for_reading helper, call Store::*
method, format as text content block.

- memory_get: Memory <ULID>\\nCreated: ...\\nSource: ...\\nTags: ...\\n\\n<content>
- memory_list: 'Found N memories (limit L):' header + one '<ULID>: <80-char-snippet>'
  line per item. AND-filter tags via Store::list_by_tags. Limit clamped to [1,100],
  default 50.
- memory_revisions: 'Revisions of <ID> (N items, newest first):' header + one line
  per revision. Walks Store::revision_history.

Eleven unit tests total (3 get + 4 list + 4 revisions). All use
TempDir + Store::ingest + handle_* directly; no subprocess. Server.rs
list_tools now returns 4 descriptors; call_tool dispatches all four.
memory_ingest joins in Task 3."
```

Verify sign-off.

---

## Task 3: `memory_ingest` + auto-wiring helper + 6 unit tests + read-only filtering

**Why third:** Ingest is the meatiest of the four new tools because of the index auto-wiring. It also introduces the read-only filtering logic (skip from `list_tools`, reject from `call_tool`).

**Files:**
- Create: `crates/singularmem-mcp/src/tools/ingest.rs`
- Modify: `crates/singularmem-mcp/src/tools/mod.rs` (declare + re-export ingest)
- Modify: `crates/singularmem-mcp/src/server.rs` (register ingest with read-only conditional + add rejection branch in call_tool)
- Modify: `crates/singularmem-mcp/src/lib.rs` (re-export new types)
- Modify: `crates/singularmem-mcp/Cargo.toml` if any new deps needed (e.g., make sure `singularmem-core`'s `hook` module is accessible — it should already be since 4a depends on it transitively)

- [ ] **Step 1: Write `src/tools/ingest.rs`**

Create `crates/singularmem-mcp/src/tools/ingest.rs`:

```rust
//! `memory_ingest` tool — write a new memory to the store, with index
//! auto-wiring so the new memory is immediately retrievable.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use jiff::Timestamp;
use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::{hook::MultiHook, IndexHook, ItemId, NewItem, Store};
use singularmem_search::{Embedder, EmbedderIndex, FastembedEmbedder, Index};

use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_ingest` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryIngestArgs {
    /// Memory body text. Non-empty, max 1 MiB.
    pub content: String,
    /// Optional tag labels (non-empty strings, max 64 bytes each, deduplicated).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional provenance label. Max 256 bytes.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional ULID of an existing memory this one corrects.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// Optional user-defined JSON object.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Handler output: the new memory's ID and timestamp.
#[derive(Debug, Clone)]
pub struct MemoryIngestOutput {
    pub id: ItemId,
    pub created_at: Timestamp,
    /// Formatted text block per the spec.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_ingest`. Only registered
/// in `list_tools` when `config.read_only == false`.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "Memory body text. Non-empty, max 1 MiB."
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional tag labels (non-empty strings, max 64 bytes each, deduplicated)."
            },
            "source": {
                "type": "string",
                "description": "Optional provenance label. Max 256 bytes."
            },
            "supersedes": {
                "type": "string",
                "description": "Optional ULID of an existing memory this one corrects. Must exist in the store."
            },
            "metadata": {
                "type": "object",
                "description": "Optional user-defined JSON object. Soft warning threshold 64 KiB."
            }
        },
        "required": ["content"]
    })
    .as_object()
    .unwrap()
    .clone();
    Tool::new(
        "memory_ingest",
        "Add a new memory to the user's local Singularmem store. Returns the new memory's ID and timestamp. Memories are private to this user.",
        std::sync::Arc::new(schema),
    )
    .annotate(ToolAnnotations::new().read_only(false))
}

/// Handle a `tools/call` for `memory_ingest`.
///
/// # Errors
///
/// - [`Error::ReadOnly`] when `config.read_only` is `true`.
/// - [`Error::InvalidId`] when `args.supersedes` doesn't parse as a ULID.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::Validation`]
///   for empty/oversized content, oversized source, invalid tags, etc.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::SupersedesNotFound`]
///   when the supersedes ID doesn't exist in the store.
/// - [`Error::Core`] for other store I/O failures.
/// - [`Error::Search`] when an index hook fails to open (rare; logged).
pub fn handle_memory_ingest(
    args: MemoryIngestArgs,
    config: &Config,
) -> Result<MemoryIngestOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }

    let supersedes = args
        .supersedes
        .as_deref()
        .map(ItemId::from_str)
        .transpose()
        .map_err(|e| Error::InvalidId(e.to_string()))?;

    let mut item = NewItem::text(args.content);
    item.tags = args.tags.unwrap_or_default();
    item.source = args.source;
    item.supersedes = supersedes;
    if let Some(meta) = args.metadata {
        item.metadata = meta;
    }

    let store = open_store_with_hooks(&config.store_path)?;
    let stored = store.ingest(item)?;

    tracing::info!(id = %stored.id, "memory ingested via MCP");

    let text = format!(
        "Ingested memory {} at {}\n",
        stored.id, stored.created_at
    );

    Ok(MemoryIngestOutput {
        id: stored.id,
        created_at: stored.created_at,
        text,
    })
}

/// Open the store with index hooks auto-wired.
///
/// **Intentional duplication with the root binary's `open_store()`**
/// (see `src/main.rs::open_store`). ~30 lines that mirror Tantivy +
/// USearch sidecar detection and embedder selection. A future
/// extraction into a shared "store opener" library is a fine idea
/// once a third consumer (e.g., a TypeScript SDK binding) needs it;
/// for now, YAGNI says keep the duplication.
fn open_store_with_hooks(store_path: &Path) -> Result<Store> {
    let mut hooks: Vec<Box<dyn IndexHook>> = Vec::new();

    let tantivy_path = derive_index_path(store_path);
    if tantivy_path.exists() {
        match Index::open(&tantivy_path) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(
                error = %e,
                "tantivy hook open failed; lexical writes for this ingest will be skipped"
            ),
        }
    }

    let vectors_path = derive_vectors_path(store_path);
    if vectors_path.exists() {
        let embedder: Box<dyn Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(FastembedEmbedder::new()?),
            };
        match EmbedderIndex::open(&vectors_path, embedder) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(
                error = %e,
                "vector hook open failed; semantic writes for this ingest will be skipped"
            ),
        }
    }

    if hooks.is_empty() {
        Ok(Store::open(store_path)?)
    } else {
        Ok(Store::open_with_hook(
            store_path,
            Box::new(MultiHook::new(hooks)),
        )?)
    }
}

fn derive_index_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    PathBuf::from(s)
}

fn derive_vectors_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_config(read_only: bool) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let config = Config::new(store_path, "plain".to_string(), read_only);
        (dir, config)
    }

    #[test]
    fn ingest_succeeds_returns_id_and_timestamp() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "hello world".to_string(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let out = handle_memory_ingest(args, &config).expect("ok");
        assert!(out.text.contains("Ingested memory "), "missing prefix: {}", out.text);
        assert!(out.text.contains(&out.id.to_string()), "ID not in text: {}", out.text);
    }

    #[test]
    fn ingest_empty_content_returns_validation_error() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: String::new(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "content",
                    ..
                }))
            ),
            "expected Core(Validation{{field: 'content'}}), got {r:?}"
        );
    }

    #[test]
    fn ingest_with_supersedes_links_to_existing() {
        let (_dir, config) = fresh_config(false);

        // First ingest.
        let first = handle_memory_ingest(
            MemoryIngestArgs {
                content: "first version".to_string(),
                tags: None,
                source: None,
                supersedes: None,
                metadata: None,
            },
            &config,
        )
        .expect("first ok");

        // Second ingest supersedes the first.
        let second = handle_memory_ingest(
            MemoryIngestArgs {
                content: "second version".to_string(),
                tags: None,
                source: None,
                supersedes: Some(first.id.to_string()),
                metadata: None,
            },
            &config,
        )
        .expect("second ok");

        // Verify the link by reading the second item back from the store.
        let store = Store::open(&config.store_path).unwrap();
        let item = store.get(second.id).unwrap();
        assert_eq!(item.supersedes, Some(first.id));
    }

    #[test]
    fn ingest_with_unknown_supersedes_returns_error() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "orphan".to_string(),
            tags: None,
            source: None,
            supersedes: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string()),
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::SupersedesNotFound { .. }))
            ),
            "expected Core(SupersedesNotFound), got {r:?}"
        );
    }

    #[test]
    fn ingest_with_tags_and_source_persists_them() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "tagged content".to_string(),
            tags: Some(vec!["foo".to_string(), "bar".to_string()]),
            source: Some("test-source".to_string()),
            supersedes: None,
            metadata: None,
        };
        let out = handle_memory_ingest(args, &config).expect("ok");

        let store = Store::open(&config.store_path).unwrap();
        let item = store.get(out.id).unwrap();
        assert_eq!(item.source, Some("test-source".to_string()));
        // Tags get sorted/deduped by Store::ingest's validation.
        let mut got_tags = item.tags.clone();
        got_tags.sort();
        assert_eq!(got_tags, vec!["bar".to_string(), "foo".to_string()]);
    }

    #[test]
    fn ingest_rejected_in_read_only_mode() {
        let (_dir, config) = fresh_config(true);
        let args = MemoryIngestArgs {
            content: "should be rejected".to_string(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(matches!(r, Err(Error::ReadOnly)), "expected ReadOnly, got {r:?}");
    }
}
```

- [ ] **Step 2: Run the ingest tests to verify they pass**

Run: `cargo test -p singularmem-mcp --lib tools::ingest::tests`
Expected: PASS for all 6 tests.

- [ ] **Step 3: Wire ingest into `src/tools/mod.rs`**

Open `crates/singularmem-mcp/src/tools/mod.rs`. Add the `ingest` module declaration and re-export:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub(crate) mod util;

pub mod get;
pub mod ingest;
pub mod list;
pub mod retrieve;
pub mod revisions;

pub use crate::tools::get::{handle_memory_get, MemoryGetArgs, MemoryGetOutput};
pub use crate::tools::ingest::{handle_memory_ingest, MemoryIngestArgs, MemoryIngestOutput};
pub use crate::tools::list::{handle_memory_list, MemoryListArgs, MemoryListOutput};
pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
pub use crate::tools::revisions::{handle_memory_revisions, MemoryRevisionsArgs, MemoryRevisionsOutput};
```

- [ ] **Step 4: Update `src/lib.rs` re-exports**

Extend the `pub use crate::tools::{...}` block in `crates/singularmem-mcp/src/lib.rs` to include `handle_memory_ingest`, `MemoryIngestArgs`, and `MemoryIngestOutput`. Place them alphabetically with the others.

- [ ] **Step 5: Register ingest in `src/server.rs` with read-only filtering**

Open `crates/singularmem-mcp/src/server.rs`. Update the `list_tools` override so it conditionally includes `memory_ingest`:

```rust
        let mut tools = vec![
            crate::tools::retrieve::tool_descriptor(),
            crate::tools::get::tool_descriptor(),
            crate::tools::list::tool_descriptor(),
            crate::tools::revisions::tool_descriptor(),
        ];
        if !self.config.read_only {
            tools.push(crate::tools::ingest::tool_descriptor());
        }
```

Update the `call_tool` match to dispatch `memory_ingest` AND reject when read-only:

```rust
            "memory_ingest" => {
                if self.config.read_only {
                    return Err(rmcp::Error::InvalidParams(
                        "server is read-only; memory_ingest is disabled".to_string(),
                    ));
                }
                let args: crate::tools::MemoryIngestArgs = parse_args(arguments)?;
                let out = crate::tools::handle_memory_ingest(args, &self.config)
                    .map_err(map_to_mcp_error)?;
                Ok(CallToolResult::text(out.text))
            }
```

**Error mapping for ingest-specific variants**. Update `map_to_mcp_error` (or the inline error-mapping logic) to handle:

```rust
        Error::ReadOnly => rmcp::Error::InvalidParams(
            "server is read-only; memory_ingest is disabled".to_string(),
        ),
        Error::Core(singularmem_core::Error::Validation { field, reason }) => {
            rmcp::Error::InvalidParams(format!("{field}: {reason}"))
        }
        Error::Core(singularmem_core::Error::SupersedesNotFound { id }) => {
            rmcp::Error::InvalidParams(format!("supersedes target {id} not found"))
        }
```

(These two `Error::Core` matches go BEFORE the generic `Error::Core(_)` catch-all that 4a's `map_to_mcp_error` likely already has.)

- [ ] **Step 6: Verify all tests still pass**

Run: `cargo test -p singularmem-mcp`
Expected: PASS for all crate tests. Count after this task: ~28 (4a's 11 + 11 from Task 2 + 6 new).

- [ ] **Step 7: Run clippy + fmt**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 8: Smoke-test that ingest works end-to-end**

```bash
{ echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'; \
  echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'; \
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_ingest","arguments":{"content":"smoke ingest"}}}'; \
  sleep 0.5; } | SINGULARMEM_STORE=/tmp/sm-4b-smoke.db cargo run --quiet -p singularmem-mcp 2>/dev/null
```

Expected: a response to id=2 with `"text"` containing `"Ingested memory "` followed by a ULID. Clean up: `rm /tmp/sm-4b-smoke.db`.

- [ ] **Step 9: Commit**

```bash
git add crates/singularmem-mcp/
git commit -s -m "feat(mcp): memory_ingest + auto-wiring + read-only filtering

Fifth tool: memory_ingest takes content (required) + tags/source/
supersedes/metadata (optional) and writes through Store::ingest with
hooks auto-wired the same way the root binary's open_store() does.
Tantivy + USearch sidecars are auto-detected via .tantivy / .vectors
suffix probing; MockEmbedder vs FastembedEmbedder selected by
SINGULARMEM_TEST_EMBEDDER env var (same convention as root binary).

~30 lines of intentional duplication with the root binary's
open_store(). YAGNI for v0; a future shared 'store opener' helper
can extract once a third consumer arrives (e.g., TS SDK binding).

Read-only filtering: list_tools omits memory_ingest when
config.read_only; call_tool rejects with InvalidParams 'server is
read-only; memory_ingest is disabled' even if the client somehow
knows the tool's name. Two layers of defense.

Six unit tests cover: success path, empty-content validation error,
supersedes linking, unknown-supersedes error, tags/source
persistence, read-only rejection. Smoke test confirms end-to-end
ingest via JSON-RPC works."
```

Verify sign-off.

---

## Task 4: Integration tests — mcp_handshake update + full_write_read_cycle + read_only_mode

**Files:**
- Modify: `crates/singularmem-mcp/tests/mcp_handshake.rs` (update `tools/list` count assertion from 1 to 5)
- Create: `crates/singularmem-mcp/tests/full_write_read_cycle.rs`
- Create: `crates/singularmem-mcp/tests/read_only_mode.rs`

- [ ] **Step 1: Update `tests/mcp_handshake.rs` to assert 5 tools**

Open `crates/singularmem-mcp/tests/mcp_handshake.rs`. Find the assertion that checks the number of tools returned by `tools/list`. The 4a test asserts 1; update to assert 5.

(The exact line depends on 4a's implementation. If 4a asserted on tool names rather than count, also extend that assertion to include the four new names.)

- [ ] **Step 2: Write `tests/full_write_read_cycle.rs`**

Create `crates/singularmem-mcp/tests/full_write_read_cycle.rs`:

```rust
//! End-to-end integration test exercising the full ingest →
//! get → list → revisions → retrieve flow through MCP. Verifies
//! that memory_ingest's auto-wiring populates the indexes so
//! memory_retrieve can find newly ingested memories without an
//! external `reindex` step.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

/// Path to the singularmem CLI binary (used to seed sidecar
/// directories via `reindex --with-embeddings` before the MCP
/// server starts — otherwise the first ingest creates only
/// SQLite rows, not Tantivy/USearch sidecars).
fn singularmem_bin() -> std::path::PathBuf {
    cargo_bin("singularmem")
}

fn mcp_bin() -> std::path::PathBuf {
    cargo_bin("singularmem-mcp")
}

/// Pre-create the .tantivy / .vectors sidecar directories by running
/// `singularmem reindex --with-embeddings` against an (empty) store.
/// This makes the MCP server's memory_ingest auto-wire the hooks on
/// its first invocation.
fn prime_sidecars(store: &Path) {
    let status = Command::new(singularmem_bin())
        .args(["--store", store.to_str().unwrap(), "reindex", "--with-embeddings"])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("singularmem reindex");
    assert!(status.success(), "reindex failed");
}

#[test]
fn full_write_read_cycle_end_to_end() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");

    // Pre-create sidecars so the MCP server's first ingest auto-wires
    // the Tantivy + USearch hooks.
    prime_sidecars(&store);

    // Spawn the MCP server.
    let mut child = Command::new(mcp_bin())
        .env("SINGULARMEM_STORE", store.to_str().unwrap())
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn singularmem-mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let stderr_handle = thread::spawn(move || {
        let mut sink = String::new();
        let mut r = BufReader::new(stderr);
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => sink.push_str(&line),
                Err(_) => break,
            }
        }
        sink
    });

    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    };
    let recv = |reader: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read");
        assert!(bytes > 0, "EOF");
        serde_json::from_str(line.trim()).expect("parse")
    };

    // Initialize.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let _ = recv(&mut reader);
    send(&mut stdin, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // Step 1: Ingest first memory.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_ingest","arguments":{"content":"the quick brown fox jumps over the lazy dog","tags":["fox","animals"],"source":"test"}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.starts_with("Ingested memory "), "ingest response: {text}");
    let first_id = text
        .strip_prefix("Ingested memory ")
        .expect("strip prefix")
        .split(' ')
        .next()
        .expect("split")
        .to_string();
    assert_eq!(first_id.len(), 26, "expected 26-char ULID: {first_id}");

    // Step 2: Get the first memory.
    let get_req = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"memory_get","arguments":{{"id":"{first_id}"}}}}}}"#
    );
    send(&mut stdin, &get_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("the quick brown fox"), "get response: {text}");

    // Step 3: Ingest second memory superseding the first.
    let supersedes_req = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"memory_ingest","arguments":{{"content":"revised fox content","supersedes":"{first_id}"}}}}}}"#
    );
    send(&mut stdin, &supersedes_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let second_id = text
        .strip_prefix("Ingested memory ")
        .expect("strip prefix")
        .split(' ')
        .next()
        .expect("split")
        .to_string();

    // Step 4: Revisions chain should show 2 items.
    let rev_req = format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"memory_revisions","arguments":{{"id":"{second_id}"}}}}}}"#
    );
    send(&mut stdin, &rev_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("2 items, newest first"), "revisions response: {text}");

    // Step 5: List should show 2 items.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"memory_list","arguments":{}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("Found 2"), "list response: {text}");

    // Step 6: Retrieve should find the new memory (proving hook auto-wiring).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"memory_retrieve","arguments":{"query":"fox"}}}"#,
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("fox"), "retrieve response: {text}");

    // Cleanup.
    drop(stdin);
    let _ = child.wait();
    let stderr_text = stderr_handle.join().expect("stderr thread");
    assert!(!stderr_text.contains("panic"), "panic in stderr: {stderr_text}");
}
```

- [ ] **Step 3: Write `tests/read_only_mode.rs`**

Create `crates/singularmem-mcp/tests/read_only_mode.rs`:

```rust
//! Verifies `--read-only` semantics:
//! - `tools/list` excludes `memory_ingest`.
//! - Direct `tools/call memory_ingest` is rejected with InvalidParams.
//! - Read tools (`memory_get`) still work.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

fn singularmem_bin() -> std::path::PathBuf {
    cargo_bin("singularmem")
}

fn mcp_bin() -> std::path::PathBuf {
    cargo_bin("singularmem-mcp")
}

/// Seed one memory + tag + reindex via the CLI so the read-only MCP
/// server has something to read.
fn seed_via_cli(store: &Path) -> String {
    let output = Command::new(singularmem_bin())
        .args([
            "--store",
            store.to_str().unwrap(),
            "ingest",
            "--content",
            "seeded memory for read-only test",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .output()
        .expect("singularmem ingest");
    assert!(output.status.success(), "ingest failed");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let id = stdout.trim();
    assert_eq!(id.len(), 26, "expected ULID: {id:?}");

    let status = Command::new(singularmem_bin())
        .args([
            "--store",
            store.to_str().unwrap(),
            "reindex",
            "--with-embeddings",
        ])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("reindex");
    assert!(status.success(), "reindex failed");
    id.to_string()
}

#[test]
fn read_only_mode_excludes_ingest_and_rejects_direct_calls() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");
    let seeded_id = seed_via_cli(&store);

    // Spawn the MCP server with --read-only.
    let mut child = Command::new(mcp_bin())
        .args(["--read-only"])
        .env("SINGULARMEM_STORE", store.to_str().unwrap())
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn singularmem-mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let stderr_handle = thread::spawn(move || {
        let mut sink = String::new();
        let mut r = BufReader::new(stderr);
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => sink.push_str(&line),
                Err(_) => break,
            }
        }
        sink
    });

    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").expect("write");
        stdin.flush().expect("flush");
    };
    let recv = |reader: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read");
        assert!(bytes > 0, "EOF");
        serde_json::from_str(line.trim()).expect("parse")
    };

    // Initialize.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let _ = recv(&mut reader);
    send(&mut stdin, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // tools/list should return 4 tools (no memory_ingest).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let resp = recv(&mut reader);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 4, "expected 4 tools in read-only mode, got: {tools:?}");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(!names.contains(&"memory_ingest"), "memory_ingest should be omitted: {names:?}");

    // tools/call memory_ingest should be rejected.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"memory_ingest","arguments":{"content":"should be rejected"}}}"#,
    );
    let resp = recv(&mut reader);
    let err = &resp["error"];
    assert!(err.is_object(), "expected error response: {resp}");
    let msg = err["message"].as_str().expect("error message");
    assert!(
        msg.contains("read-only"),
        "expected 'read-only' in error message: {msg}"
    );

    // tools/call memory_get should still work.
    let get_req = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"memory_get","arguments":{{"id":"{seeded_id}"}}}}}}"#
    );
    send(&mut stdin, &get_req);
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("get response text");
    assert!(text.contains("seeded memory"), "expected seed content: {text}");

    drop(stdin);
    let _ = child.wait();
    let stderr_text = stderr_handle.join().expect("stderr thread");
    assert!(!stderr_text.contains("panic"), "panic in stderr: {stderr_text}");
}
```

- [ ] **Step 4: Run all integration tests**

Run: `cargo test -p singularmem-mcp --test mcp_handshake --test full_write_read_cycle --test read_only_mode`
Expected: PASS for all three integration tests.

If `tests/full_write_read_cycle.rs` fails on the retrieve step, the most likely cause is that hooks didn't auto-wire on ingest. Check that `prime_sidecars` ran successfully (the `.tantivy/` and `.vectors/` directories should exist under the tempdir before the MCP server starts). Without those, `open_store_with_hooks` finds no sidecars to wire and the ingested memory is SQLite-only.

If `tests/read_only_mode.rs` fails on the tools count assertion, check that `list_tools` actually applies the `if !self.config.read_only` filter from Task 3.

- [ ] **Step 5: Run the full crate test suite**

Run: `cargo test -p singularmem-mcp`
Expected: all tests pass (~28 unit + 3 integration = ~31 total).

- [ ] **Step 6: Run workspace tests + clippy + fmt**

Run: `cargo test --workspace 2>&1 | grep "test result" | tail -5`
Expected: all green.

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/singularmem-mcp/tests/
git commit -s -m "test(mcp): integration tests for write+read parity and read-only mode

Three integration tests against subprocess instances of the MCP
server. Each spawns singularmem-mcp with SINGULARMEM_TEST_EMBEDDER
=mock so no network/model-download path runs.

- mcp_handshake: updated tools/list assertion from 1 tool to 5
  (memory_retrieve + the four new ones).
- full_write_read_cycle: pre-primes sidecars via the singularmem
  CLI, then exercises ingest → get → ingest-with-supersedes →
  revisions → list → retrieve through JSON-RPC. Proves the auto-
  wiring in memory_ingest populates indexes so memory_retrieve
  finds newly ingested memories.
- read_only_mode: seeds the store via CLI, then spawns the server
  with --read-only. Asserts tools/list returns 4 tools (no
  memory_ingest), tools/call memory_ingest is rejected with
  'read-only' in the error message, and tools/call memory_get
  still works."
```

Verify sign-off.

---

## Task 5: Documentation — README + `docs/mcp-server.md`

**Files:**
- Modify: `crates/singularmem-mcp/README.md`
- Modify: `docs/mcp-server.md`

- [ ] **Step 1: Update `crates/singularmem-mcp/README.md`**

Open `crates/singularmem-mcp/README.md`. Three sections need editing per the spec's §5:

**Status banner (top of file):** replace the existing "Status: sub-project 4a — read-only foundation" with:

```markdown
**Status:** sub-project 4b — read + write tools shipped. The server's
tool surface matches the `singularmem` CLI's operations: retrieve,
ingest, get, list, revisions. Run with `--read-only` to disable
ingest for shared-memory deployments.
```

**Available tools section:** replace the existing single-tool block (just `memory_retrieve`) with five subsections in this order:

````markdown
## Available tools

### `memory_retrieve`

Retrieves memories relevant to a query and returns them formatted for
the configured (or client-specified) adapter.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | string | yes | — | Natural-language query for the search. |
| `limit` | integer | no | 10 | Maximum number of blocks to return. Clamped to `[1, 50]`. |
| `adapter` | enum string | no | server default | One of `plain`, `claude`, `openai`, `gemini`. |

(Existing example call + response from 4a — keep as-is.)

### `memory_get`

Fetches a single memory by ID. Returns the memory's content and
metadata as text.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `id` | string | yes | — | ULID of the memory to fetch (26 characters, Crockford base32). |

**Example response:**

```
Memory 01ARZ3NDEKTSV4RRFFQ69G5FAV
Created: 2026-05-18T14:30:00Z
Source: claude-conversation:abc-123
Tags: fox, animals

the quick brown fox jumps over the lazy dog
```

### `memory_list`

Enumerates memories in the store, optionally filtered by tag (AND-
semantics). Returns a compact listing with IDs and content snippets.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `tags` | string[] | no | (none) | AND-filter tags. |
| `limit` | integer | no | 50 | Maximum number of items to return. Clamped to `[1, 100]`. |

**Example response:**

```
Found 3 memories (limit 50):

01ARZ3NDEKTSV4RRFFQ69G5FAV: the quick brown fox jumps over the lazy dog
01BX5ZZKBKACTAV9WEVGEMMVRZ: lazy dogs sleep all day
01CW8BZ7FQRJM4HCVCV9ABCDEF: another memory with longer content trunc...
```

### `memory_revisions`

Walks the supersedes chain for a memory, newest-first. Returns each
revision in the chain with ID and content snippet.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `id` | string | yes | — | ULID of any item in the chain. |

**Example response:**

```
Revisions of 01CW8BZ7FQRJM4HCVCV9ABCDEF (3 items, newest first):

01CW8BZ7FQRJM4HCVCV9ABCDEF: latest content here
01BX5ZZKBKACTAV9WEVGEMMVRZ: revised content
01ARZ3NDEKTSV4RRFFQ69G5FAV: original content
```

### `memory_ingest`

Adds a new memory to the user's local Singularmem store. **Disabled
when the server is launched with `--read-only`.** Returns the new
memory's ID and timestamp.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `content` | string | yes | — | Memory body text. Non-empty, max 1 MiB. |
| `tags` | string[] | no | `[]` | Optional tag labels (non-empty strings, max 64 bytes each, deduplicated). |
| `source` | string | no | (none) | Optional provenance label. Max 256 bytes. |
| `supersedes` | string | no | (none) | Optional ULID of an existing memory this one corrects. Must exist in the store. |
| `metadata` | object | no | `{}` | Optional user-defined JSON object. Soft warning threshold 64 KiB. |

**Example response:**

```
Ingested memory 01ARZ3NDEKTSV4RRFFQ69G5FAV at 2026-05-18T14:30:00Z
```
````

**Configuration section:** extend the existing table to include `--read-only`:

```markdown
| Flag | Env var | Default |
|---|---|---|
| `--store <PATH>` | `SINGULARMEM_STORE` | `~/.local/share/singularmem/store.db` (XDG) |
| `--default-adapter <NAME>` | `SINGULARMEM_DEFAULT_ADAPTER` | `plain` |
| `--log-level <LEVEL>` | `RUST_LOG` | `info` |
| `--read-only` | `SINGULARMEM_READ_ONLY` | `false` |
```

**Troubleshooting section:** add one entry near the bottom:

```markdown
- **"server is read-only; memory_ingest is disabled"** — The server
  was launched with `--read-only` or `SINGULARMEM_READ_ONLY=true`.
  Either drop the flag/env var to enable writes, or use the
  `singularmem` CLI for ingest (the CLI bypasses MCP's read-only
  mode since it talks directly to the store).
```

**What's coming next section:** replace the existing 4a "What's coming in 4b" forward-pointer with:

```markdown
## What's coming next

The MCP server's tool surface is now complete for v0. Future MCP work
will likely live in separate sub-projects:

- **HTTP / SSE transport** (in addition to stdio) for remote MCP
  deployments.
- **MCP resources** — read-only URIs for individual memories
  (`singularmem://memory/<id>`).
- **MCP prompts** — pre-baked prompts that incorporate retrieved
  memory for one-click "ask Singularmem about X" workflows.
```

- [ ] **Step 2: Update `docs/mcp-server.md`**

Open `docs/mcp-server.md`. Four edits:

**Layering diagram:** extend the existing diagram to show all five tool branches. Replace the `└── memory_retrieve handler` section with:

```
      └── Tool handlers:
              ├── memory_retrieve (read; uses Retriever + adapter)
              ├── memory_get      (read; Store::get)
              ├── memory_list     (read; Store::list / list_by_tags)
              ├── memory_revisions (read; Store::revision_history)
              └── memory_ingest   (write; Store::ingest + auto-wired hooks)
```

**Available tools section:** replace the single `memory_retrieve` bullet with:

```markdown
## Available tools (4b)

- **`memory_retrieve`** — semantic + lexical hybrid retrieval against
  the local store, returning adapter-formatted prompt-ready blocks.
- **`memory_get`** — fetch a single memory by ULID with full metadata.
- **`memory_list`** — enumerate memories, optionally filtered by
  tag (AND-semantics).
- **`memory_revisions`** — walk the supersedes chain newest-first.
- **`memory_ingest`** — add a new memory. Auto-wires Tantivy +
  USearch hooks so the new memory is immediately retrievable.
  Disabled when the server is launched with `--read-only`.

See `crates/singularmem-mcp/README.md` for the full input schemas
and example calls.
```

**New section before Roadmap:** add a "Read-only mode" section:

```markdown
## Read-only mode

Launch with `--read-only` (or `SINGULARMEM_READ_ONLY=true`) to
exclude `memory_ingest` from the tool surface. Use cases:

- Shared knowledge-base deployments where only specific authors
  ingest via the CLI; the MCP server is read-only for everyone
  else.
- Demos / sandboxes where you want the LLM to read sample memories
  without modifying them.
- Defense-in-depth: even if an LLM ignores instructions and tries
  to write, the server rejects the call.

The `Store` is also opened with SQLite's read-only flag in this
mode, so accidental writes from any code path fail with a SQLite
error rather than silently mutating data.
```

**Roadmap:** update to remove "memory_ingest" from "next" (it shipped in 4b). Promote HTTP transport, MCP resources, and MCP prompts as the remaining items. The existing "Roadmap" section already lists these as "Later"; just reframe to:

```markdown
## Roadmap

The v0 tool surface is complete with 4b. Future MCP work:

- **HTTP / SSE transport** (in addition to stdio).
- **MCP resources** — read-only URIs for individual memories
  (`singularmem://memory/<id>`).
- **MCP prompts** — pre-baked prompts that incorporate retrieved
  memory.
```

- [ ] **Step 3: Verify the docs are well-formed Markdown (sanity check)**

Run: `cargo fmt --check` and `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: clean (Markdown isn't checked by fmt/clippy but the surrounding code should still pass).

Optional: open the rendered docs in a browser to eyeball the structure.

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-mcp/README.md docs/mcp-server.md
git commit -s -m "docs(mcp): update README + project docs for 4b's five-tool surface

README:
- Status banner reframed for 4b (read + write parity).
- Available tools section expanded from 1 tool to 5 with full input
  schemas, example responses, and field tables.
- Configuration table gains a --read-only row.
- New troubleshooting entry for 'server is read-only' error.
- 'What's coming next' rewritten as forward-pointer to HTTP/
  resources/prompts (4 sub-project complete).

docs/mcp-server.md:
- Layering diagram extended with five tool branches.
- Available tools section rewritten to list all five.
- New 'Read-only mode' subsection with use-case rationale.
- Roadmap reframed: v0 tool surface complete; HTTP/resources/
  prompts remain."
```

Verify sign-off.

---

## Task 6: Final workspace gate

Verification-only checkpoint. No source changes unless something below fails.

- [ ] **Step 1: Workspace fmt check**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 3: Workspace test**

Run: `cargo test --workspace`
Expected: all tests pass. The known pre-existing flake
`singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order`
may intermittently fail; re-run once.

- [ ] **Step 4: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean. The four new tool handler files (`ingest.rs`, `get.rs`, `list.rs`, `revisions.rs`) all have module-level doc-comments + per-function `# Errors` sections.

- [ ] **Step 5: Final stdio-purity smoke test**

```bash
{ echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'; \
  echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'; \
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'; \
  sleep 0.5; } | cargo run --quiet -p singularmem-mcp 2>/dev/null
```

Expected: two JSON responses. The second response's `result.tools` array contains 5 entries.

- [ ] **Step 6: Cargo.lock status**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications (likely from new dev-deps in test files like `serde_json` or workspace `tokio` changes), they should already be staged in earlier task commits. Confirm:

```bash
git diff Cargo.lock
```

If clean, skip. If there's residual churn, commit it as a chore.

- [ ] **Step 7: Final repository status**

Run: `git status`
Expected: clean working tree (untracked `.agents/`, `.claude/`,
`skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -10`
Expected: the new commits from Tasks 1-5 sit on top of `a833222`
(v0.9.0 version-bump from 4a's wrap-up) and `e18add8` (4b design
spec).

---

## Self-review

**1. Spec coverage check** (each acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. Four new source files under `src/tools/` | 2 (get/list/revisions), 3 (ingest) |
| 2. `src/tools/mod.rs` re-exports | 2 (3 utility reads), 3 (ingest) |
| 3. `src/error.rs` gains `Error::ReadOnly` + `Error::InvalidId` | 1 |
| 4. `Config.read_only` + updated `Config::new` + new unit test | 1 |
| 5. `src/main.rs` gains `--read-only` flag | 1 |
| 6. `server.rs::list_tools` returns 5 / 4 conditionally | 2 (4 tools), 3 (5th conditionally) |
| 7. `server.rs::call_tool` dispatches all 5 + read-only rejection | 2 (4 tools), 3 (5th + rejection) |
| 8. `handle_memory_ingest` uses `open_store_with_hooks` (duplication intentional) | 3 |
| 9. Read handlers use `Store::open_with_options` via shared helper | 1 (helper + retrieve refactor), 2 (new handlers) |
| 10. Output formats per Interfaces; char-boundary truncation | 2 (list/revisions both use `.chars().take(80)`) |
| 11. Error mapping table | 2 (Core(NotFound), InvalidId), 3 (Validation, SupersedesNotFound, ReadOnly) |
| 12. 18 new unit tests | 1 (1 config) + 2 (11 from get/list/revisions) + 3 (6 ingest) |
| 13. `tests/mcp_handshake.rs` updated | 4 |
| 14. New `tests/full_write_read_cycle.rs` | 4 |
| 15. New `tests/read_only_mode.rs` | 4 |
| 16. No new perf budget | (no task — verified by absence) |
| 17. README updates | 5 |
| 18. `docs/mcp-server.md` updates | 5 |
| 19. fmt/clippy/doc gates clean | 6 |
| 20. `docs/formats/store-v1.md` unchanged | (no task — verified by absence) |
| 21. Tag v0.10.0 on merge | (out of plan scope — maintainer's merge ritual) |
| 22. Project memory updated post-merge | (out of plan scope — same merge ritual) |

All 22 criteria covered.

**2. Placeholder scan:** no TBDs, no "implement later". Task 3's `open_store_with_hooks` is the duplication-with-root-binary section; the plan explicitly calls it out as YAGNI-intentional rather than as something to fix.

Task 2's server.rs registration code says "Adapt the helper function names (`parse_args`, `map_to_mcp_error`) to whatever 4a actually used." This is acceptable adaptive guidance because 4a's commit shape is known but might vary in minor naming; the implementer adjusts to match. Not a placeholder.

**3. Type consistency:**
- `Config { store_path, default_adapter, known_adapters, read_only }` consistent across Tasks 1 (definition) and 2/3 (consumption).
- `Error::ReadOnly` (unit variant) + `Error::InvalidId(String)` consistent across Tasks 1 (creation) and 2/3 (consumption).
- Tool descriptors return `Tool` from rmcp 1.7.0; matches 4a's pattern.
- `*Args` / `*Output` types consistent across handler definitions and `tools/mod.rs` re-exports.
- `tools::util::open_store_for_reading(config)` signature consistent across Tasks 1 (definition) and 2 (consumption in get/list/revisions).
- `handle_memory_ingest`'s `open_store_with_hooks` is private to `ingest.rs` (different from the shared `open_store_for_reading`); the plan makes the distinction explicit.

Plan ready for execution.
