# MCP Server — Foundation + Read Tool (Sub-Project 4a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `singularmem-mcp` binary that speaks JSON-RPC over stdio (via the official `rmcp` SDK), handles the MCP initialize handshake, and exposes one read tool (`memory_retrieve`) backed by the existing `Retriever` + adapter stack from sub-projects 3a-3d.

**Architecture:** New crate `crates/singularmem-mcp/` produces a `singularmem-mcp` binary. The binary is a thin shell over `singularmem-retrieve::Retriever` + the four adapter crates; it depends on the official `rmcp` Rust SDK + `tokio` (new workspace dep). stdio transport only. All `tracing` output to stderr; stdout is reserved for JSON-RPC. Per-request store opens (no long-lived caching).

**Tech Stack:** Rust 1.80, official `rmcp` SDK (version pinned during Task 2), `tokio` (new workspace dep), `clap`, `serde_json`, `tracing`. Reuses `singularmem-retrieve` v0.5.0 surface from sub-project 3a.

**Spec:** `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-mcp/Cargo.toml` — new crate manifest.
- `crates/singularmem-mcp/src/main.rs` — clap CLI + tokio runtime + server launch.
- `crates/singularmem-mcp/src/lib.rs` — library re-exports + `serve()` entry.
- `crates/singularmem-mcp/src/server.rs` — initialize handshake + tool registration.
- `crates/singularmem-mcp/src/config.rs` — `Config` struct + `from_args`.
- `crates/singularmem-mcp/src/error.rs` — `Error` + `Result` types.
- `crates/singularmem-mcp/src/tools/mod.rs` — tool module re-exports.
- `crates/singularmem-mcp/src/tools/retrieve.rs` — `memory_retrieve` descriptor + handler + 7 unit tests.
- `crates/singularmem-mcp/tests/mcp_handshake.rs` — one black-box integration test.
- `crates/singularmem-mcp/README.md` — quick-start + MCP client configs + tool reference + troubleshooting.
- `docs/mcp-server.md` — project-level positioning + layering diagram.

**Modified:**
- `Cargo.toml` (workspace root) — add `tokio` to `[workspace.dependencies]` with the features the new crate needs.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — server is read-only).

---

## Task 1: Crate scaffold + clap CLI binary (no rmcp yet)

**Why first:** Establish the new crate with a working binary that parses CLI args and prints `--help`. Task 2 adds `rmcp` + `tokio` + the server loop; this task keeps the build green with the minimum new surface.

**Files:**
- Create: `crates/singularmem-mcp/Cargo.toml`
- Create: `crates/singularmem-mcp/src/main.rs`

- [ ] **Step 1: Create the Cargo.toml**

Create `crates/singularmem-mcp/Cargo.toml`:

```toml
[package]
name = "singularmem-mcp"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Model Context Protocol (MCP) server for Singularmem (stdio transport, read-only foundation)."

[lints]
workspace = true

[dependencies]
clap = { version = "4.5", features = ["derive", "wrap_help", "env"] }
dirs = "5"

[[bin]]
name = "singularmem-mcp"
path = "src/main.rs"
```

No rmcp / tokio / retrieve deps yet — Task 2 adds rmcp + tokio; Task 3 adds the retrieve crate. We start with just the CLI surface.

The workspace members glob `members = ["crates/*"]` auto-picks up the new crate; no `[workspace]` edit needed.

- [ ] **Step 2: Create the main.rs skeleton**

Create `crates/singularmem-mcp/src/main.rs`:

```rust
//! Singularmem MCP server — speaks Model Context Protocol over stdio so
//! MCP-compatible clients (Claude Code, Cursor, custom agents) can
//! retrieve memories from the user's local Singularmem store.
//!
//! This binary is a thin shell over `singularmem-retrieve::Retriever`
//! plus the four provider adapters from sub-projects 3b/3c/3d. The
//! constitution names the MCP server as an open-core deliverable
//! (Open / Closed Split, line 245).
//!
//! Sub-project 4a ships the foundation + one read tool
//! (`memory_retrieve`). Sub-project 4b will add `memory_ingest` and
//! utility tools.
//!
//! See `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// `singularmem-mcp` — MCP server over stdio.
#[derive(Parser, Debug)]
#[command(
    name = "singularmem-mcp",
    version,
    about = "MCP server exposing Singularmem retrieval over stdio."
)]
struct Args {
    /// Path to the `SQLite` store. Defaults to the per-user XDG data dir
    /// (same convention as `singularmem`).
    #[arg(long, env = "SINGULARMEM_STORE", value_name = "PATH")]
    store: Option<PathBuf>,

    /// Default adapter when clients don't specify one.
    #[arg(
        long,
        env = "SINGULARMEM_DEFAULT_ADAPTER",
        value_enum,
        default_value_t = AdapterChoice::Plain,
    )]
    default_adapter: AdapterChoice,

    /// `tracing` log level for stderr.
    #[arg(long, env = "RUST_LOG", value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
}

/// Adapter choices recognised at startup. Mirrors the registered
/// adapters in the root binary's `known_adapters()` function.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum AdapterChoice {
    Plain,
    Claude,
    Openai,
    Gemini,
}

impl AdapterChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Claude => "claude",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Default store path: XDG data dir + `singularmem/store.db`. Same
/// convention as the root binary's `default_store_path()`.
fn default_store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("singularmem")
        .join("store.db")
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let _store = args.store.unwrap_or_else(default_store_path);
    let _adapter = args.default_adapter.as_str();
    let _level = args.log_level.as_str();
    // Task 2 wires the rmcp server loop. For now the binary just parses
    // args and exits successfully so --help works and the workspace builds.
    std::process::ExitCode::SUCCESS
}
```

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The new crate compiles.

- [ ] **Step 4: Verify --help output**

Run: `cargo run --bin singularmem-mcp -- --help`
Expected output (approximate):

```
MCP server exposing Singularmem retrieval over stdio.

Usage: singularmem-mcp [OPTIONS]

Options:
      --store <PATH>
          Path to the SQLite store. ...
      --default-adapter <DEFAULT_ADAPTER>
          Default adapter when clients don't specify one
          [default: plain]
          Possible values: plain, claude, openai, gemini
      --log-level <LOG_LEVEL>
          tracing log level for stderr [default: info]
          Possible values: trace, debug, info, warn, error
  -h, --help
          Print help
  -V, --version
          Print version
```

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Watch for `clippy::doc_markdown` on `MCP`/`stdio`/`SQLite`/`XDG`/`tracing` in doc-comments — backtick-wrap if flagged. The seed code uses backticks where needed.

- [ ] **Step 6: Run fmt check**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add Cargo.lock crates/singularmem-mcp/
git commit -s -m "feat(mcp): new crate scaffold + clap CLI binary

Creates singularmem-mcp crate with a working CLI binary that parses
--store / --default-adapter / --log-level flags (all with env-var
equivalents) and prints helpful --help output. Builds clean with
zero new transitive deps beyond clap and dirs.

Task 2 will add the rmcp + tokio deps and wire the actual MCP
server loop. The foundation lands here so subsequent tasks can
focus on protocol concerns against a stable CLI surface."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: Add `rmcp` + `tokio` deps + initialize handshake

**Why next:** The rmcp SDK is the foundation everything else builds on. Get the initialize handshake working in isolation; the integration test in Task 5 will exercise it end-to-end. The single biggest implementation risk in this sub-project is rmcp API drift, so confine it to one task.

**Files:**
- Modify: `Cargo.toml` (workspace root — add `tokio` to `[workspace.dependencies]`)
- Modify: `crates/singularmem-mcp/Cargo.toml` (add rmcp + tokio + other server deps)
- Modify: `crates/singularmem-mcp/src/main.rs` (convert to `#[tokio::main]`, call `serve()`)
- Create: `crates/singularmem-mcp/src/lib.rs` (public `serve()` function)
- Create: `crates/singularmem-mcp/src/server.rs` (initialize handshake + empty tool registry)
- Create: `crates/singularmem-mcp/src/error.rs` (`Error` + `Result`)

- [ ] **Step 1: Pin the rmcp version**

Check crates.io for the latest stable `rmcp` version. As of writing this plan (mid-2026), expect a `0.x` series with active development.

Run: `cargo search rmcp --limit 1`
Note the latest version reported.

Pin the version exactly in the next step (e.g., `rmcp = "=0.X.Y"`), matching how `tantivy = "=0.22.1"` and `fastembed = "=4.4.0"` are pinned in `singularmem-search/Cargo.toml`. Exact pins protect against silent API drift during 0.x development.

If `rmcp` itself is unavailable or has materially changed since the brainstorm, **STOP and report BLOCKED** with the version found and a brief note about the API shape. The spec assumes the `rmcp` API stays roughly consistent; significant changes warrant a spec amendment, not a heroic in-task rewrite.

- [ ] **Step 2: Add `tokio` to workspace dependencies**

Modify the workspace root `Cargo.toml`. Find the `[workspace.dependencies]` block (line 13). The current contents end with `bincode = "1.3"`. Add `tokio` after `bincode`:

```toml
[workspace.dependencies]
rusqlite = { version = "=0.32.1", features = ["bundled"] }
ulid = { version = "1.1", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
jiff = { version = "0.1", features = ["serde"] }
tracing = "0.1"
tantivy = "=0.22.1"
fastembed = { version = "=4.4.0", default-features = false, features = ["ort-download-binaries", "online"] }
usearch = "=2.15.3"
bincode = "1.3"
tokio = "1"
```

Do NOT add features to the workspace-level `tokio` line. Each consuming crate selects features explicitly so the dependency surface stays minimal per-crate.

- [ ] **Step 3: Update `crates/singularmem-mcp/Cargo.toml` with server deps**

Replace the existing `[dependencies]` section:

```toml
[dependencies]
clap = { version = "4.5", features = ["derive", "wrap_help", "env"] }
dirs = "5"
```

with the full server dependency list (replace `X.Y.Z` with the version pinned in Step 1):

```toml
[dependencies]
singularmem-retrieve = { path = "../singularmem-retrieve" }
singularmem-adapter-claude = { path = "../singularmem-adapter-claude" }
singularmem-adapter-openai = { path = "../singularmem-adapter-openai" }
singularmem-adapter-gemini = { path = "../singularmem-adapter-gemini" }
rmcp = { version = "=X.Y.Z", features = ["server", "transport-io"] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "io-std"] }
clap = { version = "4.5", features = ["derive", "wrap_help", "env"] }
dirs = "5"
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

If `rmcp` has different feature names than `"server"` and `"transport-io"`, check its docs and adjust. The intent is: server-side capabilities + stdin/stdout transport.

- [ ] **Step 4: Create `src/error.rs`**

Create `crates/singularmem-mcp/src/error.rs`:

```rust
//! Error type for the MCP server.

use std::path::PathBuf;

/// Alias for `std::result::Result<T, Error>` used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the MCP server.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Underlying retrieve-crate failure.
    #[error("{0}")]
    Retrieve(#[from] singularmem_retrieve::Error),

    /// Underlying search-crate failure (bubbled through retrieve).
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),

    /// Underlying core-crate failure (bubbled through retrieve).
    #[error("{0}")]
    Core(#[from] singularmem_core::Error),

    /// Client requested an adapter name not in the registry.
    #[error("unknown adapter '{0}'; known adapters: plain, claude, openai, gemini")]
    UnknownAdapter(String),

    /// I/O error during transport setup or store I/O.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Store path is invalid (e.g., parent dir doesn't exist).
    #[error("invalid store path {path}: {reason}")]
    InvalidStorePath {
        /// The path that was attempted.
        path: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
}
```

The crate doesn't yet directly depend on `singularmem-search` or `singularmem-core` (only via `singularmem-retrieve`). Add them to `Cargo.toml` `[dependencies]` since the error types are referenced by `From` impls:

```toml
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }
```

(Add these two lines after the existing `singularmem-retrieve` line in Step 3's dep list.)

- [ ] **Step 5: Create `src/lib.rs`**

Create `crates/singularmem-mcp/src/lib.rs`:

```rust
//! Library entry for `singularmem-mcp`. Exposes `serve()` so the binary
//! (`src/main.rs`) and the integration test can both launch the server
//! against the same code path.

#![forbid(unsafe_code)]

pub mod error;
pub mod server;

pub use crate::error::{Error, Result};
pub use crate::server::serve;
```

- [ ] **Step 6: Create `src/server.rs` with the initialize handshake**

Create `crates/singularmem-mcp/src/server.rs`:

```rust
//! MCP server: initialize handshake + tool registration over stdio.

use crate::Result;

/// Launch the MCP server on stdio. Blocks until the client closes the
/// connection or a fatal error occurs.
///
/// # Errors
///
/// Returns the underlying [`rmcp`] transport error if stdio setup fails.
///
/// # Implementation notes
///
/// The exact rmcp API depends on the pinned version. The conceptual flow:
///
/// 1. Construct a server instance with `serverInfo { name: "singularmem-mcp",
///    version: <CARGO_PKG_VERSION> }` and `capabilities { tools: {} }`.
/// 2. Register an empty tool registry (Task 4 will add `memory_retrieve`).
/// 3. Run the server loop reading from stdin / writing to stdout.
/// 4. All `tracing` output must be configured to write to stderr only;
///    stdout is reserved for JSON-RPC framing.
pub async fn serve() -> Result<()> {
    // Reference rmcp's docs for the current server-construction pattern.
    // Typical shape (subject to API drift):
    //
    //     let server = rmcp::Server::new()
    //         .with_info(rmcp::ServerInfo {
    //             name: "singularmem-mcp".to_string(),
    //             version: env!("CARGO_PKG_VERSION").to_string(),
    //         })
    //         .with_capabilities(rmcp::Capabilities::tools_only());
    //     let transport = rmcp::transport::stdio();
    //     server.serve(transport).await?;
    //
    // Update the call shape to match the pinned rmcp version. The
    // important invariants:
    //   - serverInfo.name == "singularmem-mcp"
    //   - serverInfo.version == env!("CARGO_PKG_VERSION")
    //   - capabilities declares tools (no resources/prompts in 4a)
    //   - transport is stdio
    //   - the loop blocks until client closes the connection
    todo!("Task 2: pin the rmcp version + wire up the actual server loop")
}
```

The `todo!()` placeholder is acceptable here ONLY because the implementer's first job (Step 1 of this task) is to look at the pinned rmcp version and replace the placeholder with the version-correct call shape. If rmcp's API genuinely matches the sketch above, follow the sketch; if it doesn't, adapt.

- [ ] **Step 7: Update `src/main.rs` to call `serve()`**

Replace the existing `fn main()` body in `crates/singularmem-mcp/src/main.rs` with:

```rust
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();

    // Configure tracing to write to stderr only. stdout is owned by
    // rmcp for JSON-RPC framing — any println! or stray stdout write
    // would corrupt the protocol stream.
    let filter = match args.log_level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let _store = args.store.unwrap_or_else(default_store_path);
    let _adapter = args.default_adapter.as_str();

    // Task 3 will build a Config from the args + pass it to serve().
    // For now Task 2 just exercises the rmcp handshake with no tools.
    match singularmem_mcp::serve().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "MCP server exited with error");
            std::process::ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 8: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build (assuming the rmcp version-correct code was written; if `todo!()` is still present, the build will succeed but running the binary will panic).

If the build fails due to rmcp API mismatch:
1. Check rmcp's docs.rs page (e.g., `https://docs.rs/rmcp/X.Y.Z`).
2. Adjust the `serve()` body to match the actual API.
3. Re-build.
4. If the API is materially different from this plan's sketch (e.g., builder pattern replaced with trait-based registration), the basic structure should still translate — set up info, register tools, run loop on stdin/stdout.

- [ ] **Step 9: Smoke-test the binary manually**

Run a quick smoke test: send an `initialize` JSON-RPC message and verify the response.

Run:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"0"}}}' | cargo run --quiet --bin singularmem-mcp 2>/dev/null | head -1
```

Expected: a JSON-RPC response containing `"name":"singularmem-mcp"`.

If this doesn't work, debug rmcp setup before continuing. The integration test in Task 5 exercises this same flow; getting it working manually first saves debugging time.

- [ ] **Step 10: Verify clippy clean**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Watch for `clippy::doc_markdown`, `clippy::missing_errors_doc` on `serve()`, `clippy::unused_async` if the rmcp call ends up synchronous.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock crates/singularmem-mcp/Cargo.toml crates/singularmem-mcp/src/
git commit -s -m "feat(mcp): rmcp + tokio deps + initialize handshake

Pins rmcp at exact version (matching the tantivy/fastembed/usearch
pinning pattern in singularmem-search). Adds tokio = '1' to the
workspace dependencies so future crates can inherit. Adds singularmem-
search and singularmem-core to the MCP crate's direct deps for Error
From impls (they already came in transitively via singularmem-retrieve).

The serve() function runs the rmcp stdio server with serverInfo
{name: 'singularmem-mcp', version: CARGO_PKG_VERSION} and empty tool
registry. tracing output goes to stderr only — stdout is reserved
for JSON-RPC.

Smoke-tested manually with an initialize message; full integration
test lands in Task 5."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 3: `Config` struct + 3 unit tests

**Files:**
- Create: `crates/singularmem-mcp/src/config.rs`
- Modify: `crates/singularmem-mcp/src/lib.rs` (add `pub mod config;`)
- Modify: `crates/singularmem-mcp/src/main.rs` (build Config from Args + pass to serve())
- Modify: `crates/singularmem-mcp/src/server.rs` (accept `Config` parameter)

- [ ] **Step 1: Write the failing tests**

Create `crates/singularmem-mcp/src/config.rs`:

```rust
//! Server configuration assembled from clap args + env vars + built-in
//! defaults.

use std::path::PathBuf;

use singularmem_retrieve::Adapter;

/// Runtime configuration for the MCP server.
pub struct Config {
    /// Path to the `SQLite` store backing the server.
    pub store_path: PathBuf,
    /// Default adapter name when the client doesn't specify one.
    /// Must be the `name()` of one of `known_adapters`.
    pub default_adapter: String,
    /// Registered adapters available to clients. Mirrors the root
    /// binary's `known_adapters()` registry.
    pub known_adapters: Vec<Box<dyn Adapter>>,
}

impl Config {
    /// Build a config from CLI args. Adapter registry is hard-coded
    /// to the four constitutional Principle II providers.
    #[must_use]
    pub fn new(store_path: PathBuf, default_adapter: String) -> Self {
        Self {
            store_path,
            default_adapter,
            known_adapters: vec![
                Box::new(singularmem_retrieve::PlainAdapter),
                Box::new(singularmem_adapter_claude::ClaudeAdapter),
                Box::new(singularmem_adapter_openai::OpenAiAdapter),
                Box::new(singularmem_adapter_gemini::GeminiAdapter),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_new_registers_four_adapters() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "plain".to_string());
        let names: Vec<&str> = cfg.known_adapters.iter().map(|a| a.name()).collect();
        assert_eq!(names, vec!["plain", "claude", "openai", "gemini"]);
    }

    #[test]
    fn config_new_preserves_store_path() {
        let cfg = Config::new(PathBuf::from("/tmp/custom.db"), "claude".to_string());
        assert_eq!(cfg.store_path, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn config_new_preserves_default_adapter() {
        let cfg = Config::new(PathBuf::from("/tmp/store.db"), "openai".to_string());
        assert_eq!(cfg.default_adapter, "openai");
    }
}
```

- [ ] **Step 2: Wire the module into `lib.rs`**

Edit `crates/singularmem-mcp/src/lib.rs`. The current contents:

```rust
pub mod error;
pub mod server;

pub use crate::error::{Error, Result};
pub use crate::server::serve;
```

Replace with:

```rust
pub mod config;
pub mod error;
pub mod server;

pub use crate::config::Config;
pub use crate::error::{Error, Result};
pub use crate::server::serve;
```

- [ ] **Step 3: Update `server::serve()` to accept `Config`**

Edit `crates/singularmem-mcp/src/server.rs`. Update the function signature:

```rust
/// Launch the MCP server on stdio. Blocks until the client closes the
/// connection or a fatal error occurs.
///
/// # Errors
///
/// Returns the underlying [`rmcp`] transport error if stdio setup fails.
pub async fn serve(config: crate::Config) -> Result<()> {
    // ... existing body, but now config is available for Task 4's
    // tool registration ...
}
```

Pass `config` through wherever rmcp registers tool handlers (Task 4 will use it). For Task 3, the body still has the initialize handshake from Task 2 — just thread `config` in as a parameter so Task 4 can consume it.

- [ ] **Step 4: Update `main.rs` to build the Config and pass it**

Edit `crates/singularmem-mcp/src/main.rs`. Replace the `_store` / `_adapter` / `_level` discard lines and the `serve().await` call with:

```rust
    let store_path = args.store.unwrap_or_else(default_store_path);
    let config = singularmem_mcp::Config::new(store_path, args.default_adapter.as_str().to_string());

    match singularmem_mcp::serve(config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "MCP server exited with error");
            std::process::ExitCode::FAILURE
        }
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p singularmem-mcp --lib config::tests`
Expected: PASS for all three tests (`config_new_registers_four_adapters`, `config_new_preserves_store_path`, `config_new_preserves_default_adapter`).

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 8: Commit**

```bash
git add crates/singularmem-mcp/src/config.rs crates/singularmem-mcp/src/lib.rs crates/singularmem-mcp/src/server.rs crates/singularmem-mcp/src/main.rs
git commit -s -m "feat(mcp): Config struct + 3 unit tests

Config holds the store path, default adapter name, and the
known_adapters registry (the four Principle II providers, mirroring
the root binary's known_adapters function). main.rs builds Config
from clap args and threads it through to serve().

Three unit tests pin down: (1) registry has exactly the four
adapters in order, (2) store_path is preserved, (3) default_adapter
is preserved."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 4: `memory_retrieve` tool + handler + 7 unit tests

**Why this is the meatiest task:** Single function (`handle_memory_retrieve`) + tool descriptor + seven tests covering the handler's behaviour matrix from the spec.

**Files:**
- Create: `crates/singularmem-mcp/src/tools/mod.rs`
- Create: `crates/singularmem-mcp/src/tools/retrieve.rs`
- Modify: `crates/singularmem-mcp/src/lib.rs` (add `pub mod tools;`)
- Modify: `crates/singularmem-mcp/src/server.rs` (register the tool with rmcp)
- Modify: `crates/singularmem-mcp/Cargo.toml` (add `tempfile`, `singularmem-search` testing feature to dev-deps for tests)

- [ ] **Step 1: Add dev-dependencies for handler tests**

Update `crates/singularmem-mcp/Cargo.toml` to add a `[dev-dependencies]` section (or extend it if Task 2 created one):

```toml
[dev-dependencies]
tempfile = { workspace = true }
singularmem-search = { path = "../singularmem-search", features = ["testing"] }
singularmem-core = { path = "../singularmem-core" }
```

The `testing` feature on `singularmem-search` gives the test fixtures access to `MockEmbedder` (added unconditionally per the v0.3.0 pattern but the feature flag is correct convention).

- [ ] **Step 2: Create `src/tools/mod.rs`**

Create `crates/singularmem-mcp/src/tools/mod.rs`:

```rust
//! Tool implementations exposed via the MCP `tools/call` method.

pub mod retrieve;

pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs};
```

- [ ] **Step 3: Write the failing tests**

Create `crates/singularmem-mcp/src/tools/retrieve.rs`:

```rust
//! `memory_retrieve` tool: takes a query + optional limit + optional
//! adapter; returns adapter-formatted memory blocks ready to inject
//! into an LLM prompt.

use serde::{Deserialize, Serialize};

use singularmem_core::Store;
use singularmem_retrieve::{Adapter, RetrieveOptions, Retriever};
use singularmem_search::{
    EmbedderIndex, HybridSearchOptions, HybridSearcher, Index,
};

use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_retrieve` tool.
///
/// Matches the JSON schema declared in [`tool_descriptor`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRetrieveArgs {
    /// Natural-language query for the memory search.
    pub query: String,
    /// Maximum number of blocks to return. Clamped to `[1, 50]`.
    /// Default: 10.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Which adapter to format with. Falls back to the server's
    /// `default_adapter` when absent.
    #[serde(default)]
    pub adapter: Option<String>,
}

/// Output of the `memory_retrieve` handler. The MCP transport layer
/// wraps this in a `CallToolResult` text content block; the handler
/// itself just returns the formatted string.
#[derive(Debug, Clone)]
pub struct MemoryRetrieveOutput {
    /// Adapter-formatted memory blocks ready to embed in a prompt.
    pub text: String,
}

/// Handle a `tools/call` for `memory_retrieve`.
///
/// # Errors
///
/// - [`Error::Retrieve`] wrapping [`singularmem_retrieve::Error::EmptyQuery`]
///   for empty/whitespace queries.
/// - [`Error::UnknownAdapter`] if `args.adapter` is not in the registry.
/// - [`Error::Search`] / [`Error::Core`] for downstream failures
///   (missing indexes, store I/O, etc.).
pub fn handle_memory_retrieve(
    args: MemoryRetrieveArgs,
    config: &Config,
) -> Result<MemoryRetrieveOutput> {
    // 1. Resolve adapter (request arg → server default).
    let adapter_name = args.adapter.as_deref().unwrap_or(&config.default_adapter);
    let adapter: &dyn Adapter = config
        .known_adapters
        .iter()
        .find(|a| a.name() == adapter_name)
        .map(std::convert::AsRef::as_ref)
        .ok_or_else(|| Error::UnknownAdapter(adapter_name.to_string()))?;

    // 2. Clamp limit to [1, 50] per spec.
    let limit = args.limit.unwrap_or(10).clamp(1, 50);
    let opts = RetrieveOptions {
        max_blocks: limit,
        min_score: 0.0,
        search: HybridSearchOptions::default(),
    };

    // 3. Open store + indexes per-request. The spec is explicit: no
    // caching. Microsecond-scale per the v0.1.0 bench numbers.
    let store = Store::open(&config.store_path)?;
    let tantivy_path = derive_index_path(&config.store_path);
    let vectors_path = derive_vectors_path(&config.store_path);
    let has_lex = tantivy_path.exists();
    let has_sem = vectors_path.exists();

    if !has_lex && !has_sem {
        return Err(Error::Search(singularmem_search::Error::NoIndexes));
    }

    let lex = if has_lex { Some(Index::open(&tantivy_path)?) } else { None };
    let sem = if has_sem {
        let embedder: Box<dyn singularmem_search::Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
            };
        Some(EmbedderIndex::open(&vectors_path, embedder)?)
    } else {
        None
    };

    let searcher = match (&lex, &sem) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("checked above"),
    };

    // 4. Retrieve + format.
    let retriever = Retriever::new(&store, &searcher);
    let ctx = retriever.retrieve(&args.query, &opts)?;
    let text = adapter.format(&ctx);

    Ok(MemoryRetrieveOutput { text })
}

/// Derive the Tantivy sidecar path from a store path. Mirrors the
/// root binary's `derive_index_path()`.
fn derive_index_path(store_path: &std::path::Path) -> std::path::PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    std::path::PathBuf::from(s)
}

/// Derive the USearch sidecar path from a store path. Mirrors the
/// root binary's `derive_vectors_path()`.
fn derive_vectors_path(store_path: &std::path::Path) -> std::path::PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::NewItem;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Seed a fresh store + both sidecars with `n` items using
    /// MockEmbedder, then return the tempdir + a Config pointing at
    /// the store. The TempDir guard must outlive the test or the
    /// store gets cleaned up early.
    fn seeded(n: usize, default_adapter: &str) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let lex_path = dir.path().join("store.db.tantivy");
        let sem_path = dir.path().join("store.db.vectors");

        // Wire MultiHook so ingests populate both sidecars.
        let lex_hook = Index::open(&lex_path).unwrap();
        let sem_hook = EmbedderIndex::open(
            &sem_path,
            Box::new(singularmem_search::testing::MockEmbedder::default()),
        )
        .unwrap();
        let multi = singularmem_core::hook::MultiHook::new(vec![
            Box::new(lex_hook),
            Box::new(sem_hook),
        ]);
        let store = Store::open_with_hook(&store_path, Box::new(multi)).unwrap();
        for i in 0..n {
            store
                .ingest(NewItem::text(format!("seed memory number {i}")))
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        // Set the env var so the handler picks MockEmbedder when it
        // opens the vector index.
        std::env::set_var("SINGULARMEM_TEST_EMBEDDER", "mock");

        let config = Config::new(store_path, default_adapter.to_string());
        (dir, config)
    }

    #[test]
    fn handler_uses_default_adapter_when_arg_absent() {
        let (_dir, config) = seeded(3, "claude");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: None,
        };
        let out = handle_memory_retrieve(args, &config).expect("ok");
        // ClaudeAdapter wraps blocks in <documents>...</documents> XML.
        assert!(
            out.text.contains("<documents>"),
            "expected Claude XML shape: {}",
            out.text
        );
    }

    #[test]
    fn handler_uses_per_call_adapter_when_specified() {
        let (_dir, config) = seeded(3, "claude");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: Some("openai".to_string()),
        };
        let out = handle_memory_retrieve(args, &config).expect("ok");
        // OpenAiAdapter uses [N] markers; ClaudeAdapter does not.
        assert!(
            out.text.contains("[1]"),
            "expected OpenAI bracket markers: {}",
            out.text
        );
        assert!(
            !out.text.contains("<documents>"),
            "should NOT have used Claude XML: {}",
            out.text
        );
    }

    #[test]
    fn handler_unknown_adapter_returns_unknown_adapter_error() {
        let (_dir, config) = seeded(1, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: Some("nonexistent".to_string()),
        };
        let r = handle_memory_retrieve(args, &config);
        assert!(
            matches!(r, Err(Error::UnknownAdapter(ref s)) if s == "nonexistent"),
            "expected UnknownAdapter('nonexistent'): {r:?}"
        );
    }

    #[test]
    fn handler_respects_limit_arg() {
        let (_dir, config) = seeded(10, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: Some(3),
            adapter: None,
        };
        let out = handle_memory_retrieve(args, &config).expect("ok");
        // PlainAdapter emits one "## memory N" heading per block.
        let heading_count = out.text.matches("## memory").count();
        assert!(heading_count <= 3, "expected ≤3 blocks, got {heading_count}: {}", out.text);
    }

    #[test]
    fn handler_caps_limit_at_50() {
        let (_dir, config) = seeded(60, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: Some(1000),
            adapter: None,
        };
        let out = handle_memory_retrieve(args, &config).expect("ok");
        let heading_count = out.text.matches("## memory").count();
        assert!(heading_count <= 50, "expected ≤50 blocks (limit clamped), got {heading_count}");
    }

    #[test]
    fn handler_empty_query_returns_empty_query_error() {
        let (_dir, config) = seeded(1, "plain");
        let args = MemoryRetrieveArgs {
            query: "".to_string(),
            limit: None,
            adapter: None,
        };
        let r = handle_memory_retrieve(args, &config);
        assert!(
            matches!(r, Err(Error::Retrieve(singularmem_retrieve::Error::EmptyQuery))),
            "expected Retrieve(EmptyQuery): {r:?}"
        );
    }

    #[test]
    fn handler_no_indexes_returns_search_no_indexes_error() {
        // Bare tempdir with a store but NO sidecars.
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let _store = Store::open(&store_path).unwrap();
        let config = Config::new(store_path, "plain".to_string());

        let args = MemoryRetrieveArgs {
            query: "anything".to_string(),
            limit: None,
            adapter: None,
        };
        let r = handle_memory_retrieve(args, &config);
        assert!(
            matches!(r, Err(Error::Search(singularmem_search::Error::NoIndexes))),
            "expected Search(NoIndexes): {r:?}"
        );
    }
}
```

- [ ] **Step 4: Wire the module into `lib.rs`**

Edit `crates/singularmem-mcp/src/lib.rs`. The current contents:

```rust
pub mod config;
pub mod error;
pub mod server;

pub use crate::config::Config;
pub use crate::error::{Error, Result};
pub use crate::server::serve;
```

Replace with:

```rust
pub mod config;
pub mod error;
pub mod server;
pub mod tools;

pub use crate::config::Config;
pub use crate::error::{Error, Result};
pub use crate::server::serve;
pub use crate::tools::{handle_memory_retrieve, MemoryRetrieveArgs};
```

- [ ] **Step 5: Register the tool with rmcp in `server.rs`**

Edit `crates/singularmem-mcp/src/server.rs`. The current body has a stub for the rmcp setup. Update it to register `memory_retrieve` as a tool.

The exact registration shape depends on the pinned rmcp version. Conceptually:

```rust
// Tool descriptor — matches the JSON schema in the spec.
let descriptor = rmcp::ToolDescriptor::builder()
    .name("memory_retrieve")
    .description(
        "Retrieve memories from the user's local Singularmem store that are \
         relevant to a query. Returns formatted context the model can use to \
         ground its response. Memories are private to this user and stored locally."
    )
    .input_schema(serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Natural-language query describing what kind of memory to retrieve."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 50,
                "default": 10,
                "description": "Maximum number of memory blocks to return. Defaults to 10."
            },
            "adapter": {
                "type": "string",
                "enum": ["plain", "claude", "openai", "gemini"],
                "description": "Which provider-specific format to render memories with."
            }
        },
        "required": ["query"]
    }))
    .build();

// Handler wrapper: parse args, call handle_memory_retrieve, convert
// errors to MCP error responses.
server.register_tool(descriptor, move |req: rmcp::CallToolRequest| {
    let args: MemoryRetrieveArgs = serde_json::from_value(req.arguments)
        .map_err(|e| rmcp::Error::InvalidParams(e.to_string()))?;
    match handle_memory_retrieve(args, &config) {
        Ok(out) => Ok(rmcp::CallToolResult::text(out.text)),
        Err(Error::Retrieve(singularmem_retrieve::Error::EmptyQuery)) => {
            Err(rmcp::Error::InvalidParams("query must not be empty".to_string()))
        }
        Err(Error::UnknownAdapter(name)) => {
            Err(rmcp::Error::InvalidParams(format!("unknown adapter '{name}'")))
        }
        Err(Error::Search(singularmem_search::Error::NoIndexes)) => {
            Err(rmcp::Error::InternalError(
                "no memories indexed yet; run `singularmem ingest` first".to_string(),
            ))
        }
        Err(e) => Err(rmcp::Error::InternalError(e.to_string())),
    }
});
```

Adapt the method names (`register_tool`, `CallToolResult::text`, etc.) to match the actual pinned rmcp API. The conceptual structure stays the same. If the rmcp API uses traits + async fn rather than closures, port the body of the closure to a `Tool::call` impl on a struct that owns `Arc<Config>`.

- [ ] **Step 6: Run tests to verify they fail (initially)**

Run: `cargo test -p singularmem-mcp --lib tools::retrieve::tests`

If `handle_memory_retrieve` isn't yet referenced from the tests' `super::*` import, fix the imports first. Expected: at this point tests SHOULD pass because the code in Step 3 above IS the implementation. The "failing test" step is collapsed into Step 3 for this task because the test code and implementation code are written together.

Actually — re-order: write the tests FIRST (have them fail), then write the impl. The plan above places impl and tests in the same file as one Step 3 edit; in practice, an implementer following TDD would:

1. Write tests (in `mod tests`) that reference `handle_memory_retrieve` and `MemoryRetrieveArgs` types.
2. Run `cargo test` — fails to compile because those types don't exist.
3. Add the type stubs (`MemoryRetrieveArgs`, `MemoryRetrieveOutput`, `fn handle_memory_retrieve(...) -> ... { todo!() }`).
4. Run `cargo test` — compiles, tests fail with `todo!()` panic.
5. Implement `handle_memory_retrieve`.
6. Run `cargo test` — passes.

For brevity, Step 3 above gives the complete final state. The TDD discipline is: don't write the impl body before the tests exist. Both end up in the same commit.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p singularmem-mcp --lib tools::retrieve::tests`
Expected: PASS for all 7 tests (`handler_uses_default_adapter_when_arg_absent`, `handler_uses_per_call_adapter_when_specified`, `handler_unknown_adapter_returns_unknown_adapter_error`, `handler_respects_limit_arg`, `handler_caps_limit_at_50`, `handler_empty_query_returns_empty_query_error`, `handler_no_indexes_returns_search_no_indexes_error`).

- [ ] **Step 8: Run the full crate test suite**

Run: `cargo test -p singularmem-mcp`
Expected: PASS for all 10 tests (3 config + 7 handler).

- [ ] **Step 9: Run clippy**

Run: `cargo clippy -p singularmem-mcp --all-targets -- -D warnings`
Expected: zero warnings.

Watch for:
- `clippy::needless_pass_by_value` on `args: MemoryRetrieveArgs` — `args` is consumed (move into builder), so this is fine.
- `clippy::missing_panics_doc` on `seeded()` test helper — add `#[allow(clippy::missing_panics_doc)]` or document that test helpers may panic on unexpected setup failures.
- `clippy::doc_markdown` on `RetrieveOptions`, `HybridSearcher`, `Store`, etc. — backtick-wrap if flagged.

- [ ] **Step 10: Run fmt check**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/singularmem-mcp/Cargo.toml crates/singularmem-mcp/src/lib.rs crates/singularmem-mcp/src/server.rs crates/singularmem-mcp/src/tools/
git commit -s -m "feat(mcp): memory_retrieve tool + 7 handler tests

Implements the read-side tool: registers a memory_retrieve descriptor
with rmcp (with the JSON schema from the spec); handle_memory_retrieve
resolves the adapter (per-call arg → server default), clamps limit
to [1,50], opens store + indexes per-request, calls Retriever +
adapter.format(), returns text. Errors mapped to MCP error codes:
EmptyQuery → InvalidParams, UnknownAdapter → InvalidParams,
NoIndexes → InternalError with 'ingest first' message.

Seven unit tests cover: default adapter resolution, per-call adapter
override, unknown adapter error, limit respect, limit clamp at 50,
empty query error, no-indexes error. All tests use MockEmbedder
via SINGULARMEM_TEST_EMBEDDER=mock; no network."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 5: Integration test — black-box subprocess handshake

**Files:**
- Create: `crates/singularmem-mcp/tests/mcp_handshake.rs`
- Modify: `crates/singularmem-mcp/Cargo.toml` (add `assert_cmd` to dev-dependencies if not already present)

- [ ] **Step 1: Add `assert_cmd` to dev-deps**

Update `crates/singularmem-mcp/Cargo.toml` `[dev-dependencies]`:

```toml
[dev-dependencies]
tempfile = { workspace = true }
singularmem-search = { path = "../singularmem-search", features = ["testing"] }
singularmem-core = { path = "../singularmem-core" }
assert_cmd = { workspace = true }
serde_json = { workspace = true }
```

(`serde_json` may already be in production deps from Task 2 — if so, this dev-deps line is redundant. Either way is fine.)

- [ ] **Step 2: Write the failing test**

Create `crates/singularmem-mcp/tests/mcp_handshake.rs`:

```rust
//! Black-box integration test for the MCP server.
//!
//! Spawns the `singularmem-mcp` binary as a subprocess, seeds the store
//! by running the `singularmem` binary first (also as a subprocess),
//! sends JSON-RPC messages over stdin, reads responses from stdout, and
//! asserts on the protocol-level shape.
//!
//! Verifies the most failure-prone properties of an MCP server:
//! - Initialize handshake returns the expected serverInfo.
//! - tools/list includes the memory_retrieve descriptor.
//! - tools/call invokes the handler and returns a text block.
//! - stdout stays clean (no stray writes corrupt the JSON-RPC stream).
//! - stderr is drained continuously to avoid buffer-fill deadlock.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use tempfile::TempDir;

/// Locate the singularmem binary (root crate). Cargo sets this env
/// var for integration tests of binary crates in the same workspace.
fn singularmem_bin() -> &'static str {
    env!("CARGO_BIN_EXE_singularmem")
}

/// Locate the singularmem-mcp binary.
fn mcp_bin() -> &'static str {
    env!("CARGO_BIN_EXE_singularmem-mcp")
}

/// Seed `n` items into a store at `path` via the `singularmem` CLI,
/// then run reindex with embeddings (using MockEmbedder).
fn seed_via_cli(path: &Path, contents: &[&str]) {
    for content in contents {
        let status = Command::new(singularmem_bin())
            .args(["--store", path.to_str().unwrap(), "ingest", "--content", content])
            .env("SINGULARMEM_TEST_EMBEDDER", "mock")
            .status()
            .expect("singularmem ingest");
        assert!(status.success(), "ingest failed");
    }
    let status = Command::new(singularmem_bin())
        .args(["--store", path.to_str().unwrap(), "reindex", "--with-embeddings"])
        .env("SINGULARMEM_TEST_EMBEDDER", "mock")
        .status()
        .expect("singularmem reindex");
    assert!(status.success(), "reindex failed");
}

#[test]
fn handshake_and_retrieve_end_to_end() {
    let dir = TempDir::new().unwrap();
    let store = dir.path().join("store.db");

    // Seed the store via the CLI.
    seed_via_cli(&store, &["the quick brown fox jumps"]);

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

    // Drain stderr in a background thread so the child can't fill its
    // pipe buffer and block.
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

    // Helper to send a JSON-RPC message and read one response line.
    let send = |stdin: &mut std::process::ChildStdin, msg: &str| {
        writeln!(stdin, "{msg}").expect("write to mcp stdin");
        stdin.flush().expect("flush stdin");
    };
    let recv_response = |reader: &mut BufReader<std::process::ChildStdout>| -> serde_json::Value {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).expect("read from mcp stdout");
        assert!(bytes > 0, "EOF reading response");
        serde_json::from_str(line.trim()).expect("parse JSON response")
    };

    // Step 1: initialize handshake.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(
        resp["result"]["serverInfo"]["name"],
        "singularmem-mcp",
        "wrong serverInfo.name: {resp}"
    );
    assert!(
        resp["result"]["capabilities"]["tools"].is_object(),
        "tools capability missing: {resp}"
    );

    // Step 2: initialized notification (no response expected).
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    // Step 3: tools/call memory_retrieve.
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_retrieve","arguments":{"query":"fox"}}}"#,
    );
    let resp = recv_response(&mut reader);
    assert_eq!(resp["id"], 2);
    let content = resp["result"]["content"]
        .as_array()
        .expect("content array");
    assert!(!content.is_empty(), "empty content array: {resp}");
    let text = content[0]["text"].as_str().expect("text block");
    assert!(
        text.contains("the quick brown fox"),
        "expected ingested memory in response, got: {text}"
    );

    // Step 4: close stdin, wait for exit, check stderr was clean.
    drop(stdin);
    let exit = child.wait().expect("wait for mcp process");
    assert!(exit.success(), "MCP server exited with non-zero status: {exit:?}");

    let stderr_output = stderr_handle.join().expect("stderr thread");
    // stderr may contain tracing logs; that's expected. We just verify
    // we didn't deadlock by failing to read it.
    assert!(
        !stderr_output.contains("panic"),
        "stderr contains 'panic': {stderr_output}"
    );
}
```

- [ ] **Step 3: Build to make sure binary deps are wired**

Run: `cargo build --workspace --tests`
Expected: clean build. Both binaries (`singularmem` and `singularmem-mcp`) get compiled.

- [ ] **Step 4: Run the integration test**

Run: `cargo test -p singularmem-mcp --test mcp_handshake`
Expected: PASS for `handshake_and_retrieve_end_to_end`.

If this fails:
- Check that `tracing` output is going to stderr only (not stdout).
- Check that the rmcp server's response framing matches what the test expects (one JSON-RPC message per line, newline-terminated).
- Check that `notifications/initialized` doesn't elicit a response (notifications are one-way per JSON-RPC).
- Check that the integration test correctly drains stderr in a background thread (a child that fills its stderr pipe buffer will block forever waiting for the parent to drain it).
- Check that `SINGULARMEM_TEST_EMBEDDER=mock` is being passed both to the seed step AND the MCP server subprocess.

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Verify fmt clean**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/singularmem-mcp/Cargo.toml crates/singularmem-mcp/tests/mcp_handshake.rs
git commit -s -m "test(mcp): black-box integration test for handshake + tool call

Spawns singularmem-mcp as a subprocess with SINGULARMEM_TEST_EMBEDDER
=mock and SINGULARMEM_STORE set to a tempdir. Seeds the store by
running the singularmem CLI first. Sends initialize, initialized
notification, and tools/call memory_retrieve over stdin; asserts on
serverInfo.name, tools capability presence, and that the response
text contains the ingested memory.

Drains stderr in a background thread to avoid pipe-buffer-fill
deadlock. Catches the most failure-prone MCP server bugs: stdin/
stdout mixing, malformed JSON-RPC framing, broken initialize sequence."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 6: README + docs/mcp-server.md

**Files:**
- Create: `crates/singularmem-mcp/README.md`
- Create: `docs/mcp-server.md`

- [ ] **Step 1: Create `crates/singularmem-mcp/README.md`**

Create the file with the following content:

```markdown
# singularmem-mcp

Model Context Protocol (MCP) server that exposes Singularmem's local
memory store to MCP-compatible clients (Claude Code, Cursor, custom
agents). After installation, an LLM talking to one of these clients
can call the `memory_retrieve` tool to fetch relevant memories from
your personal Singularmem store and use them to ground its responses.

**Status:** sub-project 4a — read-only foundation. Sub-project 4b
will add `memory_ingest` and utility tools.

## Quick start

```sh
# Install both binaries from a local checkout.
cargo install --path crates/singularmem
cargo install --path crates/singularmem-mcp

# Seed some memories via the CLI.
singularmem ingest --content "We decided to use Argon2id for password hashing."
singularmem reindex --with-embeddings

# Verify the MCP server starts and accepts the initialize handshake.
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  | singularmem-mcp 2>/dev/null | head -1
```

You should see a JSON response containing `"name":"singularmem-mcp"`.

## MCP client configuration

### Claude Code (`.mcp.json` or `~/.config/claude-code/mcp.json`)

```json
{
  "mcpServers": {
    "singularmem": {
      "command": "singularmem-mcp",
      "args": [],
      "env": {
        "SINGULARMEM_STORE": "/Users/YOU/Library/Application Support/singularmem/store.db",
        "SINGULARMEM_DEFAULT_ADAPTER": "claude"
      }
    }
  }
}
```

### Cursor (`~/.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "singularmem": {
      "command": "singularmem-mcp",
      "args": [],
      "env": {
        "SINGULARMEM_STORE": "/Users/YOU/Library/Application Support/singularmem/store.db",
        "SINGULARMEM_DEFAULT_ADAPTER": "openai"
      }
    }
  }
}
```

Adjust `SINGULARMEM_DEFAULT_ADAPTER` to the format that matches your
client's LLM (`plain`, `claude`, `openai`, or `gemini`). The default
when omitted is `plain`.

## Available tools

### `memory_retrieve`

Retrieves memories relevant to a query and returns them formatted
for the configured (or client-specified) adapter.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | string | yes | — | Natural-language query for the search. |
| `limit` | integer | no | 10 | Maximum number of blocks to return. Clamped to `[1, 50]`. |
| `adapter` | enum string | no | server default | One of `plain`, `claude`, `openai`, `gemini`. |

**Example call:**

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "method": "tools/call",
  "params": {
    "name": "memory_retrieve",
    "arguments": {
      "query": "auth migration decisions",
      "limit": 5,
      "adapter": "claude"
    }
  }
}
```

**Response:** a single `text` content block with adapter-formatted
memory ready to embed in a prompt.

## Configuration

All three CLI flags have env-var equivalents:

| Flag | Env var | Default |
|---|---|---|
| `--store <PATH>` | `SINGULARMEM_STORE` | `~/.local/share/singularmem/store.db` (XDG) |
| `--default-adapter <NAME>` | `SINGULARMEM_DEFAULT_ADAPTER` | `plain` |
| `--log-level <LEVEL>` | `RUST_LOG` | `info` |

Precedence: per-call tool argument > CLI flag > env var > built-in
default.

## Troubleshooting

- **"No memories matched for query: ..."** — The store is empty or the
  query has no matches. Ingest some memories first via the
  `singularmem` CLI.
- **"no memories indexed yet; run `singularmem ingest` first"** —
  No `.tantivy/` or `.vectors/` sidecar exists. Run `singularmem
  reindex --with-embeddings` after ingesting.
- **Wrong default adapter** — Set `SINGULARMEM_DEFAULT_ADAPTER` in
  the MCP client's env block, or have the client pass an explicit
  `adapter` argument per call.
- **MCP server output looks empty or garbled** — Make sure no other
  process is writing to the server's stdout. The server reserves
  stdout for JSON-RPC framing; any stray write corrupts the stream.

## What's coming in sub-project 4b

- `memory_ingest` — write memories from the LLM side.
- `memory_get` — fetch a single item by ID.
- `memory_list` — enumerate items, optionally filtered by tag.
- `memory_revisions` — walk the supersedes chain for an item.

## License

Apache-2.0 (see workspace root LICENSE).
```

- [ ] **Step 2: Create `docs/mcp-server.md`**

Create the file with the following content:

```markdown
# MCP server

The MCP server is one of the constitution's open-core deliverables
(Open / Closed Split, line 245). It exposes Singularmem retrieval
over the Model Context Protocol so MCP-compatible clients can use
the open core as memory.

## Layering

```
MCP client (Claude Code, Cursor, ...)
      │ stdio JSON-RPC
      ▼
singularmem-mcp binary
      │
      ├── Configuration (CLI flags + env vars)
      ├── rmcp server loop (initialize + tools/list + tools/call)
      └── memory_retrieve handler
              │
              ▼
        Retriever (singularmem-retrieve)
              │
              ├── HybridSearcher (singularmem-search)
              │       │
              │       ├── Index (Tantivy lexical)
              │       └── EmbedderIndex (USearch + fastembed)
              │
              └── Store (singularmem-core)
                      │
                      └── SQLite on disk
```

The MCP server is a thin shell composing the existing libraries.
Domain logic lives in `singularmem-core`, `singularmem-search`, and
`singularmem-retrieve`; the MCP crate owns only transport + dispatch.

## Why a separate binary

The `singularmem-mcp` binary is a separate crate (not a subcommand
of the existing `singularmem` CLI). Three reasons:

1. **MCP ecosystem convention.** Each MCP server is typically its
   own binary. MCP client configs say `"command": "singularmem-mcp"`,
   not `"command": "singularmem", "args": ["mcp"]`.
2. **Dependency isolation.** The MCP server pulls in `rmcp` + `tokio`
   + transitive deps. CLI-only users who never use MCP don't pay
   that cost in install size or compile time.
3. **Optional install.** Users who want only the CLI can
   `cargo install singularmem` and skip the MCP server entirely.

## Available tools (4a)

- **`memory_retrieve`** — read-only retrieval against the local
  Singularmem store. Returns adapter-formatted memory blocks ready
  to embed in a prompt.

See `crates/singularmem-mcp/README.md` for the full input schema
and example calls.

## Roadmap

- **4b** (next): `memory_ingest` write tool + utility tools
  (`memory_get`, `memory_list`, `memory_revisions`).
- **Later**: HTTP/SSE transport (in addition to stdio).
- **Later**: MCP resources (read-only URIs for individual memories).
- **Later**: MCP prompts (pre-baked prompts that incorporate retrieved
  memory).

## Related docs

- `crates/singularmem-mcp/README.md` — user-facing quick-start +
  client config snippets + troubleshooting.
- `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md` —
  design spec for 4a.
- `.specify/memory/constitution.md` — Principle II (provider-agnostic
  by contract) + Open / Closed Split + Principle V (thin shells over
  libraries).
```

- [ ] **Step 3: Run rustdoc check (catches any newly-introduced doc issues)**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean.

- [ ] **Step 4: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. Markdown isn't formatted by rustfmt; this is just a paranoid sanity check.

- [ ] **Step 5: Commit**

```bash
git add crates/singularmem-mcp/README.md docs/mcp-server.md
git commit -s -m "docs(mcp): README + project-level mcp-server.md

README walks users through quick-start (install, ingest, smoke-test),
provides copy-pastable MCP client configs for Claude Code and Cursor,
documents the memory_retrieve tool's input schema with an example
call, lists the three configuration sources (CLI flags / env vars /
built-in defaults) with precedence, troubleshoots common gotchas,
and forward-points to 4b's planned tools.

docs/mcp-server.md adds project-level positioning: a layering
diagram, rationale for the separate-binary choice, and a roadmap.

No source changes."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 7: Final workspace gate

This task is a verification-only checkpoint. No source changes unless something below fails.

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
may intermittently fail; re-run once if so.

- [ ] **Step 4: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean.

- [ ] **Step 5: stdio-purity smoke test**

Re-run the manual stdio smoke from Task 2 Step 9 to confirm nothing
introduced stray stdout writes:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  | cargo run --quiet --bin singularmem-mcp 2>/dev/null \
  | head -1
```

Expected: one valid JSON line containing `"name":"singularmem-mcp"`.

- [ ] **Step 6: Cargo.lock status**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications (almost certainly will because
Task 1 added a new crate and Task 2 added rmcp + tokio), they should
already be staged in those task commits. Confirm:

```bash
git diff Cargo.lock
```

If clean, skip. If there's still uncommitted churn:

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after MCP server landing"
```

- [ ] **Step 7: Final repository status**

Run: `git status`
Expected: clean working tree (untracked `.agents/`, `.claude/`,
`skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -10`
Expected: the new commits from Tasks 1-6 sit on top of `54f2e2f`
(the v0.8.0 version-bump from sub-project 3d's wrap-up) and `a887c96`
(the 4a design spec commit).

---

## Self-review

**1. Spec coverage check** (each acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. New crate scaffold + 8 source files | 1 (scaffold), 2 (lib/server/error), 3 (config), 4 (tools/) |
| 2. `tokio` added to workspace deps | 2 |
| 3. New crate dep list | 1 (clap + dirs), 2 (rmcp/tokio/serde/etc.), 4 (dev-deps for tests) |
| 4. Binary `singularmem-mcp` produced | 1 |
| 5. CLI surface (--store, --default-adapter, --log-level) with env-vars | 1 |
| 6. Initialize handshake with serverInfo + tools capability | 2 |
| 7. tools/list returns one descriptor (memory_retrieve) | 4 |
| 8. tools/call dispatches to handler; per-request store opens | 4 |
| 9. Limit clamped to [1, 50] | 4 |
| 10. Error mapping (EmptyQuery, NoIndexes, etc.) | 4 |
| 11. stdio purity (tracing to stderr) | 2 (configures stderr writer), 5 (integration test verifies) |
| 12. 10 unit tests pass | 3 (3 config tests), 4 (7 handler tests) |
| 13. Integration test passes | 5 |
| 14. No new perf budget | (no task — verified by absence) |
| 15. README with quick-start, client configs, tool reference, troubleshooting | 6 |
| 16. docs/mcp-server.md with positioning + layering diagram | 6 |
| 17. Workspace fmt/clippy/doc all clean | 7 |
| 18. docs/formats/store-v1.md unchanged | (no task — verified by absence) |
| 19. Tag v0.9.0 on merge | (out of plan scope — maintainer's merge ritual) |
| 20. Project memory updated post-merge | (out of plan scope — same merge ritual) |

All twenty criteria covered.

**2. Placeholder scan:**
- Task 2 Step 6 uses `todo!()` in the `serve()` body as a deliberate placeholder. The narrative explicitly notes this and tells the implementer to replace it with the rmcp version-correct call shape in the same task. This is acceptable scaffolding, not a plan failure.
- Task 1's `main()` discards the parsed args with `let _`. The narrative explicitly notes Task 2 wires the rmcp loop. This is intentional.
- No "TBD", "implement later", "add appropriate error handling" patterns.

**3. Type consistency:**
- `Config { store_path, default_adapter, known_adapters }` consistent across Tasks 3 (creation) and 4 (consumption).
- `MemoryRetrieveArgs { query, limit, adapter }` matches the JSON schema in §2 of the spec (Task 4 implementation + Task 5 test JSON-RPC body).
- `Error` enum variants (Retrieve / Search / Core / UnknownAdapter / Io / InvalidStorePath) consistent across Task 2 (creation) and Task 4 (consumption).
- `handle_memory_retrieve(args, config) -> Result<MemoryRetrieveOutput>` signature consistent across Task 4 (definition) and Task 4 tests + Task 5's integration test (which exercises it indirectly via the rmcp transport).
- Adapter registry order (`plain`, `claude`, `openai`, `gemini`) consistent across Task 3 (creation), Task 4 (lookup), and the JSON schema enum.

Plan ready for execution.
