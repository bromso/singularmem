# singularmem-mcp

Model Context Protocol (MCP) server that exposes Singularmem's local
memory store to MCP-compatible clients (Claude Code, Cursor, custom
agents). After installation, an LLM talking to one of these clients
can call the `memory_retrieve` tool to fetch relevant memories from
your personal Singularmem store and use them to ground its responses.

**Status:** sub-project 16 — the server lists 15 tools (11 with
`--read-only`) and advertises all three MCP capabilities: tools,
prompts (one `wake-up` prompt), and resources (one
`singularmem://memory/{id}` template). The tool surface matches the
`singularmem` CLI's operations: retrieve, ingest, get, list,
revisions, scopes, wake-up, and graph add/query/invalidate/supersede/
timeline/stats/entities/history. Run with `--read-only` to disable
`memory_ingest` and the three graph writers for shared-memory
deployments.

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

### Pinning a default project for wake-up

Set `SINGULARMEM_PROJECT` so `memory_wakeup` and the `wake-up` prompt
default to a specific checkout without every call passing `project`
— useful when the server runs outside the repo it should wake up for
(e.g. launched from a client's own working directory):

```json
{
  "mcpServers": {
    "singularmem": {
      "command": "singularmem-mcp",
      "args": [],
      "env": {
        "SINGULARMEM_STORE": "/Users/YOU/Library/Application Support/singularmem/store.db",
        "SINGULARMEM_PROJECT": "/path/to/repo"
      }
    }
  }
}
```

Without `SINGULARMEM_PROJECT` (or a per-call `project` argument), the
server falls back to its own working directory.

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
| `scope` | string | no | (none) | Restrict to this scope path and its descendants, e.g. `"claude-code/myproj"`. |
| `scope_exact` | boolean | no | `false` | Match only the exact scope given in `scope`. |

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
| `scope` | string | no | (none) | Restrict to this scope path and its descendants, e.g. `"claude-code/myproj"`. |
| `scope_exact` | boolean | no | `false` | Match only the exact scope given in `scope`. |

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
| `scope` | string | no | (none) | Optional scope path (validated, lowercased). |

**Example response:**

```
Ingested memory 01ARZ3NDEKTSV4RRFFQ69G5FAV at 2026-05-18T14:30:00Z
```

### `memory_scopes`

Lists every scope path present in the store with its item count,
sorted by path. Use a returned path as the `scope` argument of
`memory_list` or `memory_retrieve`.

**Arguments:** none.

**Example response:**

```
claude-code/singularmem	12
claude-code/singularmem/auth	3
work/notes	5
```

If the store has no scoped items, the response is
`No scopes (all items are unscoped).`

### `memory_wakeup`

Loads the project's recent memory — the same context the editor hooks
inject at session start. Call this at the start of a session; prefer
`memory_retrieve` for a specific question.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `project` | string | no | server `--project`, else its cwd | Directory whose scopes to read. |
| `include_files` | boolean | no | `false` | Also include `files/<basename>` items (ingest-dir output). |
| `limit` | integer | no | 20 | Most recent items to consider. |
| `max_bytes` | integer | no | 8192 | Output budget in bytes; oldest blocks are dropped first. |
| `adapter` | enum string | no | server default | One of `plain`, `claude`, `openai`, `gemini`. |

An empty project is not an error — the output reports `0 items,
showing last 0` and nothing follows.

**Example response:**

```
# Singularmem wake-up — claude-code/myproj, codex/myproj, cursor/myproj — 2 items, showing last 2
# 2 memories for query: "wake-up:claude-code/myproj,codex/myproj,cursor/myproj"

## memory 1 (score=0.0000)
id: 01M1S3F0Q8W1J8Z5N0V4K2X9YB
created: 2026-09-05T18:37:03.363997Z

alpha decision
---

## memory 2 (score=0.0000)
id: 01M1S3F0QAQ4T7H6M2C8D1R5ZE
created: 2026-09-05T18:37:03.401212Z

beta decision
---
```

(The body after the first line is the adapter's own rendering; `plain`
shown here.)

### `memory_graph_add`

Records a new, independent fact in the temporal knowledge graph
(subject-predicate-object, with an optional validity window and
confidence). **Disabled when the server is launched with
`--read-only`.** Use this for facts that don't replace an existing
one; for a fact whose old value should stop being current, use
`memory_graph_supersede` instead.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `subject` | string | yes | — | Subject entity name (created if it doesn't exist). |
| `predicate` | string | yes | — | Predicate, e.g. `"uses"` or `"works_at"`. |
| `object` | string | yes | — | Object: an entity name, or a literal value when `object_is_value`. |
| `object_is_value` | boolean | no | `false` | When `true`, `object` is a literal string value rather than an entity name. |
| `subject_kind` | string | no | (none) | Kind to set on the subject if it is being created for the first time. |
| `object_kind` | string | no | (none) | Kind to set on the object entity if it is being created for the first time. Ignored when `object_is_value` is `true`. |
| `valid_from` | string | no | (none) | Start of the validity window (`YYYY-MM-DD` or RFC 3339). |
| `valid_to` | string | no | (none) | End of the validity window (`YYYY-MM-DD` or RFC 3339). |
| `confidence` | number | no | `1.0` | Confidence in `[0.0, 1.0]`. |
| `source_item_id` | string | no | (none) | ULID of the memory this fact was extracted from, if any. |
| `scope` | string | no | (none) | Scope path to record this fact under, if any. |

**Example response:**

```
01ARZ3NDEKTSV4RRFFQ69G5FAV  singularmem —uses→ tantivy  [?, open)  conf=1.00  scope=-  src=-
```

### `memory_graph_query`

Queries current or historical facts by entity or by predicate. Call
this before answering questions about who owns what, which tool is
used, or any other current-state fact — it is more reliable than
free-text retrieval for structured facts.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `entity` | string | exactly one of `entity`/`predicate` | — | Entity name to query facts about. |
| `predicate` | string | exactly one of `entity`/`predicate` | — | Predicate to query facts by. |
| `direction` | enum string | no | `both` | One of `outgoing`, `incoming`, `both`. Only meaningful with `entity`. |
| `as_of` | string | no | (none) | Restrict to facts valid at this instant (`YYYY-MM-DD` or RFC 3339). |
| `recorded_at` | string | no | (none) | Restrict to facts believed as of this record time (`YYYY-MM-DD` or RFC 3339). |
| `scope` | string | no | (none) | Restrict to this scope path and its descendants, e.g. `"claude-code/myproj"`. |
| `scope_exact` | boolean | no | `false` | Match only the exact scope given in `scope`. |

**Example response:**

```
01ARZ3NDEKTSV4RRFFQ69G5FAV  singularmem —uses→ tantivy  [?, open)  conf=1.00  scope=-  src=-
```

With no matching facts, the response is `No facts.`.

### `memory_graph_invalidate`

Ends an open fact — marks subject-predicate-object as no longer true,
without recording a replacement value. **Disabled when the server is
launched with `--read-only`.** Use this for facts that simply ended;
when there's a new value to record in its place, use
`memory_graph_supersede` instead. The original row is never modified;
this appends a closing revision.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `subject` | string | yes | — | Subject entity name. |
| `predicate` | string | yes | — | Predicate. |
| `object` | string | yes | — | Object: an entity name, or a literal value when `object_is_value`. |
| `object_is_value` | boolean | no | `false` | When `true`, `object` is a literal string value rather than an entity name. |
| `at` | string | no | now | Instant the fact ended (`YYYY-MM-DD` or RFC 3339). |
| `scope` | string | no | (none) | Scope the fact was recorded under, if any. |

**Example response:**

```
01BX5ZZKBKACTAV9WEVGEMMVRZ  singularmem —uses→ tantivy  [?, 2026-06-01T00:00:00Z)  conf=1.00  scope=-  src=-
```

### `memory_graph_supersede`

Atomically replaces one fact's value with another: closes the old
subject-predicate-object (if any) and opens the new one, in a single
transaction. **Disabled when the server is launched with
`--read-only`.** Use this for single-valued facts that changed — it's
the right tool whenever there's both an old value ending and a new
value starting.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `subject` | string | yes | — | Subject entity name. |
| `predicate` | string | yes | — | Predicate. |
| `old_object` | string | yes | — | The fact's current object. Tolerated if no open fact matches — the response reports `closed: none`. |
| `new_object` | string | yes | — | The fact's replacement object, same shape as `old_object`. |
| `object_is_value` | boolean | no | `false` | When `true`, both `old_object` and `new_object` are literal string values rather than entity names. |
| `at` | string | no | now | Instant the change took effect (`YYYY-MM-DD` or RFC 3339). |
| `scope` | string | no | (none) | Scope the fact was recorded under, if any. |

**Example response:**

```
closed: 01BX5ZZKBKACTAV9WEVGEMMVRZ  singularmem —uses→ tantivy  [?, 2026-06-01T00:00:00Z)  conf=1.00  scope=-  src=-
opened: 01CW8BZ7FQRJM4HCVCV9ABCDEF  singularmem —uses→ meilisearch  [2026-06-01T00:00:00Z, open)  conf=1.00  scope=-  src=-
```

### `memory_graph_timeline`

Lists every fact revision — open and closed alike — for an entity or
the whole graph, ordered by validity start ascending, with revisions
whose start is unknown (`[?, …`) first, then by record time and id. Use
this to see how a fact changed over time, not just its current value.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `entity` | string | no | (none) | Restrict to facts touching this entity. Omit for the whole graph. |
| `scope` | string | no | (none) | Restrict to this scope path and its descendants, e.g. `"claude-code/myproj"`. |
| `scope_exact` | boolean | no | `false` | Match only the exact scope given in `scope`. |

**Example response:**

```
[closed] 01BX5ZZKBKACTAV9WEVGEMMVRZ  singularmem —uses→ tantivy  [?, 2026-06-01T00:00:00Z)  conf=1.00  scope=-  src=-
[current] 01CW8BZ7FQRJM4HCVCV9ABCDEF  singularmem —uses→ meilisearch  [2026-06-01T00:00:00Z, open)  conf=1.00  scope=-  src=-
```

With no matching facts, the response is `No facts.`.

### `memory_graph_stats`

Reports aggregate counts over the knowledge graph: entities, open
facts, closed facts, and distinct predicates.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `scope` | string | no | (none) | Restrict counts to this scope path and its descendants, if any. |

**Example response:**

```
entities: 2
open facts: 1
closed facts: 1
predicates: 1
```

### `memory_graph_entities`

Lists entities in the knowledge graph, optionally filtered by `kind`
and/or `scope`. One line per entity, name ascending.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `kind` | string | no | (none) | Restrict to entities of this kind. |
| `scope` | string | no | (none) | Restrict to entities with at least one fact in this scope path and its descendants. |
| `scope_exact` | boolean | no | `false` | Match only the exact scope given in `scope`. |

**Example response:**

```
01ARZ3NDEKTSV4RRFFQ69G5FAV	singularmem	tool	3
01BX5ZZKBKACTAV9WEVGEMMVRZ	tantivy	library	1
```

Tab-separated: id, name, kind (`-` when absent), fact count. With no
matching entities, the response is `No entities.`.

### `memory_graph_history`

Walks one fact's revision chain, oldest first — every closing and
reopening that led to its current state.

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `fact_id` | string | yes | — | ULID of the fact whose revision chain to show. |

**Example response:**

```
01BX5ZZKBKACTAV9WEVGEMMVRZ  singularmem —uses→ tantivy  [?, 2026-06-01T00:00:00Z)  conf=1.00  scope=-  src=-
01CW8BZ7FQRJM4HCVCV9ABCDEF  singularmem —uses→ meilisearch  [2026-06-01T00:00:00Z, open)  conf=1.00  scope=-  src=-
```

An unknown or malformed `fact_id` is an invalid-params error.

## Prompts

### `wake-up`

Wraps `memory_wakeup` as a one-click MCP prompt: "recent memory for
the current project, ready to paste into context."

**Arguments:**

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `project` | string | no | server `--project`, else its cwd | Project directory. |

`prompts/get` returns a single `user` message whose text is exactly
the `memory_wakeup` output for that project, with every other option
(`include_files`, `limit`, `max_bytes`, `adapter`) at its default.

## Resources

### `singularmem://memory/{id}`

A read-only view of one memory, addressed by its ULID. Advertised via
`resources/templates/list`; `resources/list` itself always returns an
empty list — a memory store is not a browsing experience, so reach a
memory by ID first (`memory_get`, `memory_retrieve`, or
`memory_list`), then read it as a resource if your client wants it
attached to the conversation rather than returned as a tool result.

`resources/read` for `singularmem://memory/<ulid>` returns one
`text/plain` contents entry:

```
id: 01ARZ3NDEKTSV4RRFFQ69G5FAV
created_at: 2026-05-18T14:30:00Z
scope: claude-code/myproj
source: -
tags: fox, animals

the quick brown fox jumps over the lazy dog
```

Any other URI scheme, a malformed ULID, or a well-formed ULID with no
matching item all return the MCP `resource_not_found` error, naming
the requested URI. Read-only mode changes nothing here — resources are
always readable.

## Configuration

All CLI flags have env-var equivalents:

| Flag | Env var | Default |
|---|---|---|
| `--store <PATH>` | `SINGULARMEM_STORE` | `~/.local/share/singularmem/store.db` (XDG) |
| `--default-adapter <NAME>` | `SINGULARMEM_DEFAULT_ADAPTER` | `plain` |
| `--log-level <LEVEL>` | `RUST_LOG` | `info` |
| `--read-only` | `SINGULARMEM_READ_ONLY` | `false` |
| `--project <DIR>` | `SINGULARMEM_PROJECT` | server's working directory |

`--project` sets the default project directory for `memory_wakeup`
and the `wake-up` prompt when a call omits `project`.

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
- **"server is read-only; memory_ingest is disabled"** (or the same
  message naming `memory_graph_add`, `memory_graph_invalidate`, or
  `memory_graph_supersede`) — The server was launched with
  `--read-only` or `SINGULARMEM_READ_ONLY=true`. Either drop the
  flag/env var to enable writes, or use the `singularmem` CLI (it
  bypasses MCP's read-only mode since it talks directly to the
  store).

## What's coming next

The MCP server's tool surface covers items and, as of sub-project 14,
the temporal knowledge graph; sub-project 16 added wake-up (tool +
prompt), the two remaining graph readers, and the
`singularmem://memory/{id}` resource. Future MCP work will likely
live in separate sub-projects:

- **HTTP / SSE transport** (in addition to stdio) for remote MCP
  deployments.

## License

Apache-2.0 (see workspace root LICENSE).
