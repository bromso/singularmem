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
              ├── memory_wakeup   (read; wakeup::build + render)
              ├── memory_ingest   (write; Store::ingest + auto-wired hooks)
              ├── memory_graph_query      (read; Store::query_entity / query_predicate)
              ├── memory_graph_timeline   (read; Store::timeline)
              ├── memory_graph_stats      (read; Store::graph_stats)
              ├── memory_graph_entities   (read; Store::entities)
              ├── memory_graph_history    (read; Store::fact_history)
              ├── memory_graph_add        (write; Store::add_fact)
              ├── memory_graph_invalidate (write; Store::invalidate_fact)
              └── memory_graph_supersede  (write; Store::supersede_fact)
      ├── Prompts: wake-up (wraps memory_wakeup)
      └── Resources: singularmem://memory/{id} (read; Store::get)

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
- **`memory_wakeup`** — the project's recent memory, formatted exactly
  as `singularmem wake-up` prints it, for loading at the start of a
  session. Resolves its project directory from the `project`
  argument, else the server's `--project`, else its working
  directory.
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
- **`memory_graph_entities`** — list entities (optionally filtered by
  `kind` and/or `scope`), one per line with id, name, kind, and fact
  count.
- **`memory_graph_history`** — walk one fact's revision chain
  oldest-first by its `fact_id`.
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

## Prompts

The server enables the `prompts` capability and advertises one
prompt:

- **`wake-up`** — an optional `project` argument; returns a single
  `user` message whose text is exactly the `memory_wakeup` output for
  that project with every other option at its default. Lets an MCP
  client offer "load Singularmem's memory of this project" as a
  one-click action instead of a tool call.

## Resources

The server enables the `resources` capability.

- `resources/templates/list` returns one template:
  `singularmem://memory/{id}`, mime type `text/plain`, description "A
  single memory by ULID".
- `resources/list` returns an empty list, by design — a memory store
  is not a browsing experience. Reach a memory by ID (via
  `memory_get`, `memory_retrieve`, or `memory_list`), then read it as
  a resource if the client wants it attached rather than returned as
  a tool result.
- `resources/read` with `singularmem://memory/<ulid>` returns one
  `text/plain` contents entry:

  ```
  id: <ulid>
  created_at: <rfc3339>
  scope: <scope or ->
  source: <source or ->
  tags: <comma-separated or ->

  <content>
  ```

  Any other scheme, a malformed ULID, or an unknown ULID all map to
  the MCP `resource_not_found` error, naming the requested URI.

## Project resolution (`--project` / `memory_wakeup` / `wake-up`)

`memory_wakeup` and the `wake-up` prompt both resolve their project
directory the same way: the call's `project` argument, else the
server's `--project` flag (env `SINGULARMEM_PROJECT`), else the
server's current working directory. The directory must exist; the
scopes it maps to are derived the same way the CLI's `singularmem
wake-up` derives them, so a symlinked checkout still matches the
scopes the editor hooks wrote.

## Read-only mode

Launch with `--read-only` (or `SINGULARMEM_READ_ONLY=true`) to
exclude `memory_ingest`, `memory_graph_add`, `memory_graph_invalidate`,
and `memory_graph_supersede` from the tool surface — 11 tools remain
(down from 15). Use cases:

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
temporal knowledge graph's six `memory_graph_*` tools; sub-project 16
added `memory_wakeup`, the two remaining graph readers
(`memory_graph_entities`, `memory_graph_history`), the `wake-up`
prompt, and the `singularmem://memory/{id}` resource — the server now
lists 15 tools (11 read-only) and advertises all three MCP
capabilities (tools, prompts, resources). Future MCP work:

- **HTTP / SSE transport** (in addition to stdio).

## Related docs

- `crates/singularmem-mcp/README.md` — user-facing quick-start +
  client config snippets + troubleshooting.
- `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md` —
  design spec for 4a.
- `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md` —
  design spec for the temporal knowledge graph (sub-project 14),
  including the `memory_graph_*` wire contract and error handling.
- `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md` —
  design spec for the MCP and Node surface pass (sub-project 16):
  `memory_wakeup`, the `wake-up` prompt, `memory_graph_entities`,
  `memory_graph_history`, and the `singularmem://memory/{id}`
  resource.
- `.specify/memory/constitution.md` — Principle II (provider-agnostic
  by contract) + Open / Closed Split + Principle V (thin shells over
  libraries).
