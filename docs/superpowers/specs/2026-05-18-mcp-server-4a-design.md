---
title: MCP server — foundation + read tool (sub-project 4a)
date: 2026-05-18
status: draft
sub-project: 4a-mcp-server-foundation
supersedes: none
---

# MCP server — foundation + read tool (sub-project 4a)

This sub-project ships the foundation of a Model Context Protocol
(MCP) server for Singularmem so MCP-compatible clients — Claude Code,
Cursor, and custom agents — can retrieve memories without going
through the CLI. A new crate `singularmem-mcp` produces a
`singularmem-mcp` binary that speaks JSON-RPC over stdio, handles the
MCP initialize handshake, and exposes a single read tool
(`memory_retrieve`) backed by the existing
`singularmem-retrieve::Retriever` + the four adapters that shipped in
sub-projects 3a–3d.

**Sub-project 4b** will add `memory_ingest` (the write side) plus
utility tools (`memory_get`, `memory_list`, `memory_revisions`) on
top of this foundation.

## Problem & motivation

The constitution names "MCP server" as an open-core deliverable so
"any MCP-compatible client (Claude Code, Cursor, custom agents) can
use the open core as memory" (line 245). After sub-project 3 (v0.8.0)
shipped all four provider adapters, every prerequisite for that
deliverable exists: a memory store, lexical+semantic+hybrid search,
formatted retrieval through a typed adapter contract. The only
missing piece is the transport — a way for an MCP client to invoke
retrieval over the wire.

Bundling the full MCP server (read + write + utility tools) into one
sub-project would land 25–35 tasks in one PR. That fights the
discipline that made sub-projects 2a/2b/2c, 3a/3b/3c/3d each
reviewable. The split adopted here — 4a (foundation + read) and 4b
(write + utilities) — keeps each PR in the ~10-task budget and
delivers a working-but-read-only MCP server in 4a that can be wired
into Claude Code immediately. Users can ingest via the existing CLI;
the LLM retrieves via MCP.

## Goals & non-goals

### Goals

1. New crate `singularmem-mcp` producing a `singularmem-mcp` binary.
2. stdio transport via the official `rmcp` Rust SDK.
3. MCP initialize handshake correctly handled (`Initialize` /
   `Initialized` / `tools/list` / `tools/call`).
4. One tool: `memory_retrieve` with `query` (required), `limit`
   (optional, default 10, max 50), `adapter` (optional, enum of the
   four registered providers).
5. Configuration via CLI flags + env vars + built-in defaults
   (precedence: per-call arg > CLI flag > env var > built-in).
6. End-to-end integration test that spawns the binary and exercises
   the full initialize → tools/list → tools/call flow.
7. Per-client config documentation for Claude Code and Cursor.
8. Workspace version bumps to `v0.9.0` on merge.

### Non-goals

- **No write tools.** `memory_ingest` is sub-project 4b. Users seed
  the store via the existing `singularmem ingest` CLI.
- **No additional read tools.** `memory_get`, `memory_list`,
  `memory_revisions` are sub-project 4b.
- **No HTTP / SSE transport.** stdio only for v0. HTTP transport is
  a separate later sub-project if demand emerges.
- **No resources or prompts.** MCP supports both alongside tools.
  4a declares only `capabilities { tools: {} }`. Resources and
  prompts can come later.
- **No streaming responses.** The `memory_retrieve` tool returns a
  single complete text block, not progressive content.
- **No tool-call cancellation.** rmcp may surface
  `notifications/cancel` events; 4a ignores them. In-flight retrieval
  is microseconds anyway.
- **No long-lived store handle.** Open the store + indexes
  per-request. Premature optimization to keep a long-lived handle
  given Tantivy's writer-lock constraints (sub-project 2c's
  `cmd_search` lessons).
- **No `--read-only` flag.** 4a is functionally read-only because the
  only tool is `memory_retrieve`. 4b will revisit when `memory_ingest`
  arrives.
- **No adapter plugin loading.** The four built-in adapters are
  hard-coded into the server's `known_adapters` list, mirroring the
  root binary. Plugin discovery is a much later concern.
- **No on-disk changes.** Server is read-only over the existing
  store + sidecars.

## Recommended approach

A new crate `crates/singularmem-mcp/` produces a separate binary
`singularmem-mcp`. The crate depends only on `singularmem-retrieve`
plus the four adapter crates (for the `known_adapters` registry that
mirrors the root binary's) plus the official `rmcp` SDK, `tokio` (new
workspace dep), and the usual `clap`/`serde`/`tracing` ensemble.

The binary parses CLI args + env vars via clap, builds a `Config`
struct, registers one tool handler with rmcp, and runs the server
loop on tokio's multi-threaded runtime. All `tracing` output is
configured to stderr; stdout is reserved for JSON-RPC. The
`memory_retrieve` tool handler opens a fresh `Store` + index pair per
request (microsecond-scale cost) and dispatches to the existing
`Retriever::retrieve` + adapter `format` path.

The MCP wire layer (initialize handshake, JSON-RPC framing, message
dispatch) is rmcp's job. The MCP server's job is registering the tool
descriptor + the handler closure that maps `MemoryRetrieveArgs` →
`CallToolResult`.

### Approaches discarded

- **Approach B — single sub-project covering scaffold + stdio + all
  read + write tools.** Rejected at the brainstorm: ~25-task PR
  fights the bite-sized-PR discipline established across sub-projects
  2 and 3.

- **Approach C — three-way split (4a scaffold only, 4b read, 4c
  write).** Rejected: 4a-scaffold-only is too tiny (~6 tasks) and
  isn't useful on its own. The current 4a-with-one-read-tool ships
  value immediately.

- **Approach D — subcommand `singularmem mcp` of the existing
  binary.** Rejected at brainstorm: pulls `rmcp` + tokio +
  transitives into every `singularmem` install, including users who
  never use MCP. Separate binary keeps dependency surfaces isolated.

- **Approach E — hand-rolled JSON-RPC + MCP message types.**
  Rejected: ~400-600 lines of code to maintain against the moving
  MCP spec, when the official SDK is available and tracks upstream.
  Hand-rolling buys nothing in v0.

- **Approach F — long-lived store handle across requests.** Rejected
  for v0. Tantivy's single-writer-per-directory constraint
  (sub-project 2c's `cmd_search` lessons) makes "open once, use
  forever" trickier than "open per-request" when the same store
  might also be in use by the `singularmem` CLI. Per-request opens
  are microseconds.

- **Approach G — exhaustive tool input schema (mode, min_score,
  fetch_multiplier, rrf_k, etc.).** Rejected: too many parameters
  degrade LLM tool-call quality. The MCP server uses sensible
  defaults; CLI users have the full surface.

## Architecture

Components:

- **`crates/singularmem-mcp/`** — new crate, version `0.9.0`
  (workspace-locked).
- **`crates/singularmem-mcp/Cargo.toml`** — production deps on
  `singularmem-retrieve` + the four adapter crates + `rmcp` + `tokio`
  + `clap`/`serde`/`thiserror`/`tracing`/`tracing-subscriber`. Adds
  `tokio` to `[workspace.dependencies]` (new workspace dep).
- **`crates/singularmem-mcp/src/main.rs`** — clap parsing, tokio
  runtime setup, server launch via `serve()`. ~80 lines.
- **`crates/singularmem-mcp/src/lib.rs`** — public `serve()` entry +
  re-exports for tests.
- **`crates/singularmem-mcp/src/server.rs`** — initialize handshake,
  tool registration, dispatch. ~120 lines.
- **`crates/singularmem-mcp/src/config.rs`** — `Config` struct +
  `from_args(&Args) -> Config`. Mirrors the root binary's
  `known_adapters()` list.
- **`crates/singularmem-mcp/src/tools/mod.rs`** — tool module.
- **`crates/singularmem-mcp/src/tools/retrieve.rs`** —
  `memory_retrieve` tool descriptor + handler.
- **`crates/singularmem-mcp/src/error.rs`** — `Error` enum + `Result`
  alias. Variants wrap upstream `singularmem_retrieve::Error` (via
  `#[from]`) plus add MCP-specific variants (`UnknownAdapter`,
  `Transport`).
- **`crates/singularmem-mcp/tests/mcp_handshake.rs`** — single
  black-box integration test.
- **`crates/singularmem-mcp/README.md`** — quick-start + MCP client
  configs + tool reference + troubleshooting.
- **`docs/mcp-server.md`** — project-level positioning + layering
  diagram + rationale.

The MCP server is a "thin shell composing libraries" per constitution
Principle V. Domain logic remains in `singularmem-core/-search/-retrieve`
+ the adapter crates. The MCP crate owns transport + dispatch only.

## Data model

**No changes.** MCP server consumes `RetrievedContext` /
`MemoryBlock` / the `Adapter` trait from sub-project 3a unchanged.
No persistent data, no on-disk artefacts, no `format_version` bump.

## Interfaces

### CLI (binary surface)

```
singularmem-mcp [OPTIONS]

Options:
      --store <PATH>           Path to the SQLite store. Defaults to the per-user XDG data dir
                               (same convention as `singularmem`).
      --default-adapter <NAME> Default adapter when clients don't specify one
                               [default: plain] [possible values: plain, claude, openai, gemini]
      --log-level <LEVEL>      tracing log level for stderr
                               [default: info] [possible values: trace, debug, info, warn, error]
```

All args have env-var equivalents (`SINGULARMEM_STORE`,
`SINGULARMEM_DEFAULT_ADAPTER`, `RUST_LOG`) via clap's `env` attribute.

### MCP wire protocol

Server responds to:

- **`initialize`** — replies with `serverInfo { name:
  "singularmem-mcp", version: <CARGO_PKG_VERSION> }` and
  `capabilities { tools: {} }`. Declares no resources or prompts in
  4a.
- **`notifications/initialized`** — no reply (notifications are
  one-way per JSON-RPC).
- **`tools/list`** — returns exactly one descriptor: `memory_retrieve`
  with the JSON schema below.
- **`tools/call { name: "memory_retrieve", arguments: {...} }`** —
  dispatches to the handler.

Any other request returns the MCP standard error response (rmcp
default behaviour: `MethodNotFound`).

### `memory_retrieve` tool descriptor

```json
{
  "name": "memory_retrieve",
  "description": "Retrieve memories from the user's local Singularmem store that are relevant to a query. Returns formatted context the model can use to ground its response. Memories are private to this user and stored locally.",
  "inputSchema": {
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
        "description": "Which provider-specific format to render memories with. Defaults to the server's --default-adapter.",
        "enum": ["plain", "claude", "openai", "gemini"]
      }
    },
    "required": ["query"]
  }
}
```

### Tool handler logic

```rust
pub fn handle_memory_retrieve(
    args: MemoryRetrieveArgs,
    config: &Config,
) -> Result<CallToolResult> {
    // 1. Resolve adapter (request arg → config default).
    let adapter_name = args.adapter
        .as_deref()
        .unwrap_or(&config.default_adapter);
    let adapter = config.known_adapters
        .iter()
        .find(|a| a.name() == adapter_name)
        .ok_or_else(|| Error::UnknownAdapter(adapter_name.to_string()))?;

    // 2. Build RetrieveOptions (clamped to spec'd bounds).
    let limit = args.limit.unwrap_or(10).min(50).max(1);
    let opts = RetrieveOptions {
        max_blocks: limit,
        min_score: 0.0,
        search: HybridSearchOptions::default(),
    };

    // 3. Open store + indexes per-request (microsecond cost).
    let store = Store::open(&config.store_path)?;
    let resolved = resolve_search_mode(&config.store_path, SearchMode::Auto)?;
    let (lex, sem) = open_indexes_for_mode(&resolved)?;
    let searcher = build_searcher(&lex, &sem);

    // 4. Retrieve + format.
    let retriever = Retriever::new(&store, &searcher);
    let ctx = retriever.retrieve(&args.query, &opts)?;
    let formatted = adapter.format(&ctx);

    Ok(CallToolResult::text(formatted))
}
```

`resolve_search_mode`, `open_indexes_for_mode`, and `build_searcher`
are small helper functions in a `src/handlers.rs` module (or
inlined into `tools/retrieve.rs` if simpler). They mirror the
existing root binary's `cmd_retrieve` open logic.

### Configuration (`Config` struct)

```rust
pub struct Config {
    pub store_path: PathBuf,
    pub default_adapter: String,
    pub known_adapters: Vec<Box<dyn singularmem_retrieve::Adapter>>,
}
```

`Config::from_args(&Args) -> Config` registers the four adapters
hard-coded (mirroring the root binary's `known_adapters()`). The
duplication is light (~10 lines) and intentional for v0; a shared
adapter-discovery library is overengineering before we have multiple
consumers.

### Wire (HTTP / IPC)

stdio only. No HTTP/SSE/IPC sockets in 4a. Future sub-projects can
add transports without changing the tool surface.

## Error handling

Per Principle VII: every error variant maps to a specific MCP error
code with a human-readable message.

| Source | Error variant | MCP error code | Notes |
|---|---|---|---|
| `Retriever::retrieve` | `Error::EmptyQuery` | `InvalidParams` | Client sent empty/whitespace query |
| `HybridSearcher::search` | `Error::Search(NoIndexes)` | `InternalError` | Message: "no memories indexed yet; ingest some first" |
| `HybridSearcher::search` | `Error::Search(IndexMissing)` | `InternalError` | Same message as NoIndexes |
| `Retriever::retrieve` | `Error::Core(NotFound)` | `InternalError` | Race between search and store.get; rare |
| Adapter lookup | `Error::UnknownAdapter` | `InvalidParams` | Should be caught by JSON-schema enum validation; defensive |
| rmcp transport | `Error::Transport` | (rmcp handles) | Logged to stderr; server may exit if fatal |
| Anything else | wrapped via `#[from]` | `InternalError` | With underlying message |

**No silent failures.** Every error path either returns an MCP error
response (which rmcp surfaces to the client) or logs at
`tracing::error!` to stderr and continues. The server never panics
on tool-call errors.

**stdio purity**: `tracing` is configured to write to stderr only
(via `tracing_subscriber::fmt().with_writer(std::io::stderr)` in
`main.rs`). stdout is owned by rmcp for JSON-RPC framing. The
integration test verifies this property — any `println!` or stray
stdout write would corrupt the protocol stream.

## Testing strategy

### Unit tests (`crates/singularmem-mcp/src/`)

Ten tests over pure functions, no subprocess spawning:

| Test file | Test | What it pins down |
|---|---|---|
| `config.rs` | `from_args_uses_built_in_store_default` | `--store` omitted → `store_path` is XDG data dir |
| `config.rs` | `from_args_respects_explicit_store` | `--store /tmp/x` → `store_path` is `/tmp/x` |
| `config.rs` | `from_args_registers_four_adapters` | `known_adapters` has exactly `plain`, `claude`, `openai`, `gemini` |
| `tools/retrieve.rs` | `handler_uses_default_adapter_when_arg_absent` | `args.adapter = None` + `config.default_adapter = "claude"` → output is Claude XML shape |
| `tools/retrieve.rs` | `handler_uses_per_call_adapter_when_specified` | `args.adapter = Some("openai")` overrides config default |
| `tools/retrieve.rs` | `handler_unknown_adapter_returns_invalid_params_error` | Adapter name not in registry → `Error::UnknownAdapter` |
| `tools/retrieve.rs` | `handler_respects_limit_arg` | `args.limit = Some(3)` → exactly 3 blocks in output |
| `tools/retrieve.rs` | `handler_caps_limit_at_50` | `args.limit = Some(1000)` → clamped to 50 |
| `tools/retrieve.rs` | `handler_empty_query_returns_invalid_params_error` | `args.query = ""` → MCP InvalidParams |
| `tools/retrieve.rs` | `handler_no_indexes_returns_internal_error` | Fresh store, no sidecars → InternalError "ingest some first" |

Each test creates a `TempDir` + seeds via `Store::ingest` (using
`MockEmbedder` from `singularmem-search::testing`) + invokes the
handler directly with synthetic `MemoryRetrieveArgs`. No tokio
runtime, no subprocess.

### Integration test (`crates/singularmem-mcp/tests/mcp_handshake.rs`)

One black-box end-to-end test:

| Test | What it pins down |
|---|---|
| `handshake_and_retrieve_end_to_end` | Spawn `singularmem-mcp` subprocess with `SINGULARMEM_TEST_EMBEDDER=mock` + `SINGULARMEM_STORE=<tempdir>/store.db`. Pre-seed the store via the `singularmem` CLI subprocess (ingest + reindex `--with-embeddings`). Send three JSON-RPC messages over stdin: `initialize`, `notifications/initialized`, `tools/call { name: "memory_retrieve", arguments: { query: "fox" } }`. Assert: `serverInfo.name == "singularmem-mcp"` in initialize response; `tools/call` response contains a text content block with the ingested memory's full content; stderr drained (no buffer-fill deadlock); exit code 0 when the test closes stdin. |

This test catches the most common MCP-server bugs: stdin/stdout
mixing, malformed JSON-RPC framing, broken initialize sequence,
stderr buffer-fill deadlock.

### Perf budget

**No new budget.** Per-request cost dominated by `Retriever::retrieve`
(already budgeted at `hybrid_search_latency < 150 ms`) + JSON-RPC
encode/decode (microseconds). A dedicated `mcp_retrieve_latency`
budget would track `hybrid_search_latency + ε` and be
noise-dominated. Sub-project 4b's `memory_ingest` may warrant a
write-path budget.

### Offline guarantee

Per Principle VI: all unit tests use `MockEmbedder`. The integration
test spawns subprocesses with `SINGULARMEM_TEST_EMBEDDER=mock` set.
No network for any test. The `tests-offline` advisory CI job picks
up the new tests automatically.

### Lint + fmt + doc gates

Same as all prior sub-projects:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
- `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`

Watch for tokio-specific lints (`clippy::unused_async`,
`clippy::needless_pass_by_value` on async closures) and
`clippy::doc_markdown` on `MCP`, `JSON-RPC`, `stdio`, `tokio`,
`rmcp` in doc-comments.

## Open questions

None at spec time. Three notes for the implementation plan:

1. **`rmcp` version pin.** The plan's Task 1 should check
   crates.io for the latest `rmcp` version at implementation time
   and pin it exactly (e.g., `rmcp = "=0.X.Y"` matching how
   `tantivy`/`fastembed`/`usearch` are pinned in
   `singularmem-search/Cargo.toml`). If the API has shifted from the
   examples used during brainstorming, fix the handler signatures.

2. **`tokio` as new workspace dep.** Add to
   `[workspace.dependencies]` with `version = "1"` + the features
   needed (`rt-multi-thread`, `macros`, `io-std`). Future crates can
   inherit. The plan's Task 1 handles this edit.

3. **Per-request store opens vs caching.** Per the Recommended
   Approach + Non-goals, 4a opens the store per-request. The plan
   should not "optimize" by caching — it would just add complexity
   without measurable benefit at 4a's tool surface.

## Acceptance criteria

1. New crate `crates/singularmem-mcp/` exists with `Cargo.toml`,
   `src/main.rs`, `src/lib.rs`, `src/server.rs`, `src/config.rs`,
   `src/error.rs`, `src/tools/mod.rs`, `src/tools/retrieve.rs`.
   Version workspace-locked.
2. Workspace `Cargo.toml` adds `tokio` to `[workspace.dependencies]`
   with `version = "1"` plus features `rt-multi-thread`, `macros`,
   `io-std`. The new crate consumes it via `tokio = { workspace =
   true, features = [...] }`.
3. New crate's `Cargo.toml` depends on `singularmem-retrieve`, the
   four adapter crates, `rmcp` (version pinned exactly), `tokio`,
   `clap`, `serde`, `serde_json`, `thiserror`, `tracing`, and
   `tracing-subscriber`.
4. Binary `singularmem-mcp` produced by `cargo build --release --bin
   singularmem-mcp`.
5. CLI surface: `--store <PATH>`, `--default-adapter <NAME>` (enum:
   `plain|claude|openai|gemini`), `--log-level <LEVEL>` (enum:
   `trace|debug|info|warn|error`). All three have env-var equivalents
   (`SINGULARMEM_STORE`, `SINGULARMEM_DEFAULT_ADAPTER`, `RUST_LOG`).
   Built-in defaults: XDG data dir, `plain`, `info`.
6. MCP initialize handshake: server responds to `initialize` with
   `serverInfo { name: "singularmem-mcp", version:
   <CARGO_PKG_VERSION> }` + `capabilities { tools: {} }`. Responds
   to `notifications/initialized` (no reply).
7. `tools/list` returns exactly one descriptor: `memory_retrieve`
   with the JSON schema from the Interfaces section.
8. `tools/call { name: "memory_retrieve", arguments: {...} }`
   dispatches to the handler. Handler resolves adapter from
   `args.adapter` → `config.default_adapter`; opens store + indexes
   per-request; calls `Retriever::retrieve` + adapter `format`;
   returns a single `text` content block.
9. `limit` is clamped to `[1, 50]` regardless of input value.
10. Error mapping per the Error Handling table: `EmptyQuery` →
    `InvalidParams`; `NoIndexes` → `InternalError` with "ingest some
    first"; unknown adapter → `InvalidParams` (defensive); all other
    errors → `InternalError` with the underlying message.
11. **stdio purity**: all `tracing` output goes to stderr; stdout is
    reserved for JSON-RPC. Verified by the integration test.
12. All 10 unit tests from the Testing section pass on
    `ubuntu-latest` and `macos-latest`.
13. The single integration test
    `handshake_and_retrieve_end_to_end` passes, exercising spawn →
    initialize → tools/call against a real subprocess with
    `SINGULARMEM_TEST_EMBEDDER=mock`.
14. No new perf budget; bounded by existing `hybrid_search_latency <
    150 ms`.
15. `crates/singularmem-mcp/README.md` exists with quick-start, MCP
    client config snippets for Claude Code and Cursor, tool
    reference, and troubleshooting section.
16. `docs/mcp-server.md` exists with constitutional positioning +
    layering diagram + rationale for the separate-binary choice.
17. `cargo fmt --check`, `cargo clippy --workspace --all-targets
    --tests --benches -- -D warnings`, and `RUSTDOCFLAGS='-D
    missing-docs -D warnings' cargo doc --workspace --no-deps` all
    clean.
18. `docs/formats/store-v1.md` unchanged; `format_version` stays
    `"1"`.
19. Tagged on merge as `v0.9.0` (additive MINOR bump per Principle V).
20. Project memory updated post-merge with 4a deliverables + the
    v0.9.0 milestone + forward-pointer to 4b's `memory_ingest` scope.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. The MCP server runs locally on the user's machine; transport is stdio between the user's MCP client and the user's MCP server. Both processes are local. |
| **II — Provider-Agnostic by Contract** | Provider-agnostic by construction: the `adapter` tool argument lets the client pick from the four registered providers per-call. Removing any adapter still leaves three others available; the registry-based design from 3a/3b/3c/3d carries forward. |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (new crate, new binary). III.b: no on-disk changes; `format_version` unchanged. The MCP server is named in the constitution's Open/Closed Split (line 245) as an open-core deliverable. |
| **IV — CLI-First** | `singularmem-mcp` is itself a CLI binary (with `--store`/`--default-adapter`/`--log-level`). Every config knob reachable from the command line. The MCP protocol layer is a new transport, not a replacement for CLI. |
| **V — Composable Library Architecture** | The MCP server is a thin shell over `singularmem-retrieve::Retriever` + the adapter crates. Domain logic lives in the libraries; the binary owns only transport + dispatch. Matches the constitution's wording: "The CLI, desktop GUI, MCP server, and any provider adapters are thin shells composing these libraries." |
| **VI — Deterministic and Offline-Testable** | All unit tests use `MockEmbedder`. The integration test spawns the binary with `SINGULARMEM_TEST_EMBEDDER=mock`. No network. The `tests-offline` advisory CI job picks them up automatically. |
| **VII — Honest Failure Modes** | Every error variant maps to a specific MCP error code (`InvalidParams` or `InternalError`) with a human-readable message. `tracing::warn!`/`tracing::error!` to stderr supplements with context. No silent failures. No panics on tool-call errors. |
| **VIII — Privacy Telemetry** | No telemetry. The server logs query text + adapter name at `info` level (for local debugging via `RUST_LOG`); no PII or memory content in logs. |
| **IX — Accessible by Default** | Tool output is whatever the adapter produces. PlainAdapter is ASCII-only; ClaudeAdapter/OpenAIAdapter are mostly ASCII; GeminiAdapter uses U+2014 em-dash per its spec. The MCP client renders text however it does — accessibility is the client's responsibility, not the server's. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Bounded by existing `hybrid_search_latency < 150 ms`. JSON-RPC encode/decode is microsecond-scale; rmcp + serde are well-optimized. |

No principle at ⚠️ or ❌. The biggest sensitivity is **stdio purity
for JSON-RPC** — any `eprintln!` (fine, goes to stderr) or stray
`println!` (corrupts protocol) in transitively-called code paths
would break the transport. The integration test catches this
directly.
