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
