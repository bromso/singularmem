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
      └── Tool handlers:
              ├── memory_retrieve (read; uses Retriever + adapter)
              ├── memory_get      (read; Store::get)
              ├── memory_list     (read; Store::list_by_tags_scoped)
              ├── memory_revisions (read; Store::revision_history)
              ├── memory_scopes   (read; Store::scopes)
              ├── memory_ingest   (write; Store::ingest + auto-wired hooks)
              ├── memory_graph_query      (read; Store::query_entity / query_predicate)
              ├── memory_graph_timeline   (read; Store::timeline)
              ├── memory_graph_stats      (read; Store::graph_stats)
              ├── memory_graph_add        (write; Store::add_fact)
              ├── memory_graph_invalidate (write; Store::invalidate_fact)
              └── memory_graph_supersede  (write; Store::supersede_fact)

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

## Available tools

- **`memory_retrieve`** — semantic + lexical hybrid retrieval against
  the local store, returning adapter-formatted prompt-ready blocks.
  Accepts an optional `scope` (+ `scope_exact`) argument to restrict
  results to a scope subtree or an exact scope. Its description tells
  the model to call `memory_graph_query` first for current facts.
- **`memory_get`** — fetch a single memory by ULID with full metadata.
- **`memory_list`** — enumerate memories, optionally filtered by
  tag (AND-semantics) and/or `scope` (+ `scope_exact`).
- **`memory_revisions`** — walk the supersedes chain newest-first.
- **`memory_scopes`** — list every scope path in the store with its
  item count, sorted by path. Use a returned path as the `scope`
  argument of `memory_list` or `memory_retrieve`.
- **`memory_ingest`** — add a new memory. Auto-wires Tantivy +
  USearch hooks so the new memory is immediately retrievable.
  Accepts an optional `scope` argument (validated, lowercased).
  Disabled when the server is launched with `--read-only`.
- **`memory_graph_query`** — query current or historical facts by
  entity or by predicate, with `as_of`/`recorded_at`/`direction`/
  `scope` filters.
- **`memory_graph_timeline`** — list every fact revision (open and
  closed) for an entity or the whole graph, ordered by validity
  start.
- **`memory_graph_stats`** — entity, open-fact, closed-fact, and
  distinct-predicate counts, optionally scoped.
- **`memory_graph_add`** — record a new, independent fact
  (subject-predicate-object with an optional validity window and
  confidence). Disabled when the server is launched with
  `--read-only`.
- **`memory_graph_invalidate`** — end an open fact without recording
  a replacement. Append-only: the original row is never modified.
  Disabled when the server is launched with `--read-only`.
- **`memory_graph_supersede`** — atomically close an old fact and
  open its replacement in one transaction. Disabled when the server
  is launched with `--read-only`.

See `crates/singularmem-mcp/README.md` for the full input schemas
and example calls.

## Read-only mode

Launch with `--read-only` (or `SINGULARMEM_READ_ONLY=true`) to
exclude `memory_ingest`, `memory_graph_add`, `memory_graph_invalidate`,
and `memory_graph_supersede` from the tool surface — 8 tools remain
(down from 12). Use cases:

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

## Roadmap

The item tool surface was complete with 4b; sub-project 14 added the
temporal knowledge graph's six `memory_graph_*` tools. Future MCP
work:

- **HTTP / SSE transport** (in addition to stdio).
- **MCP resources** — read-only URIs for individual memories
  (`singularmem://memory/<id>`).
- **MCP prompts** — pre-baked prompts that incorporate retrieved
  memory.

## Related docs

- `crates/singularmem-mcp/README.md` — user-facing quick-start +
  client config snippets + troubleshooting.
- `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md` —
  design spec for 4a.
- `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md` —
  design spec for the temporal knowledge graph (sub-project 14),
  including the `memory_graph_*` wire contract and error handling.
- `.specify/memory/constitution.md` — Principle II (provider-agnostic
  by contract) + Open / Closed Split + Principle V (thin shells over
  libraries).
