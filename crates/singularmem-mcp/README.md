# singularmem-mcp

Model Context Protocol (MCP) server that exposes Singularmem's local
memory store to MCP-compatible clients (Claude Code, Cursor, custom
agents). After installation, an LLM talking to one of these clients
can call the `memory_retrieve` tool to fetch relevant memories from
your personal Singularmem store and use them to ground its responses.

**Status:** sub-project 4b — read + write tools shipped. The server's
tool surface matches the `singularmem` CLI's operations: retrieve,
ingest, get, list, revisions. Run with `--read-only` to disable
ingest for shared-memory deployments.

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

Retrieves memories relevant to a query and returns them formatted for
the configured (or client-specified) adapter.

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

## Configuration

All three CLI flags have env-var equivalents:

| Flag | Env var | Default |
|---|---|---|
| `--store <PATH>` | `SINGULARMEM_STORE` | `~/.local/share/singularmem/store.db` (XDG) |
| `--default-adapter <NAME>` | `SINGULARMEM_DEFAULT_ADAPTER` | `plain` |
| `--log-level <LEVEL>` | `RUST_LOG` | `info` |
| `--read-only` | `SINGULARMEM_READ_ONLY` | `false` |

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
- **"server is read-only; memory_ingest is disabled"** — The server
  was launched with `--read-only` or `SINGULARMEM_READ_ONLY=true`.
  Either drop the flag/env var to enable writes, or use the
  `singularmem` CLI for ingest (the CLI bypasses MCP's read-only
  mode since it talks directly to the store).

## What's coming next

The MCP server's tool surface is now complete for v0. Future MCP work
will likely live in separate sub-projects:

- **HTTP / SSE transport** (in addition to stdio) for remote MCP
  deployments.
- **MCP resources** — read-only URIs for individual memories
  (`singularmem://memory/<id>`).
- **MCP prompts** — pre-baked prompts that incorporate retrieved
  memory for one-click "ask Singularmem about X" workflows.

## License

Apache-2.0 (see workspace root LICENSE).
