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
