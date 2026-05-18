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
              ├── memory_list     (read; Store::list / list_by_tags)
              ├── memory_revisions (read; Store::revision_history)
              └── memory_ingest   (write; Store::ingest + auto-wired hooks)

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

## Roadmap

The v0 tool surface is complete with 4b. Future MCP work:

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
- `.specify/memory/constitution.md` — Principle II (provider-agnostic
  by contract) + Open / Closed Split + Principle V (thin shells over
  libraries).
