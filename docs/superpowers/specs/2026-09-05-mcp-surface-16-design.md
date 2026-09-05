# MCP and Node surface pass (sub-project 16) — design

**Date:** 2026-09-05
**Status:** approved design, awaiting plan
**Programme:** mempalace parity, sub-project 16 (after 14 knowledge graph
and 15 retrieval benchmark; before 17 Python SDK)

## Goal

Expose what the libraries already do through the two remote surfaces
that lag behind the CLI:

- The MCP server gains wake-up (as a tool and as a prompt), the two
  graph readers the CLI has and MCP lacks (`entities`, `history`), and
  read-only resources for individual memories.
- The Node binding gains the knowledge graph and wake-up.

No new domain logic. Every new entry point delegates to
`singularmem-core`, `singularmem-retrieve`, or the adapters, per
Principle V.

## Non-goals

- HTTP/SSE transport for MCP.
- Enumerating memories as resources (`resources/list` stays empty).
- Ingest sources (transcripts, directories) from Node or MCP.
- Agent diaries, coordination, or any mempalace agent feature.
- Changing any existing tool's arguments or output.

## MCP server

### Configuration

New flag `--project <DIR>` (env `SINGULARMEM_PROJECT`) on
`singularmem-mcp`. It is the default project for wake-up when a call
omits `project`. When neither is set, the server's current working
directory is used. The `Config` struct gains `project: Option<PathBuf>`.

Project resolution, shared by the tool and the prompt:

1. `project` argument, else `Config.project`, else `std::env::current_dir()`.
2. The path must exist and be a directory; otherwise
   `invalid_params("project <path> is not a directory")`.
3. Scopes are derived with `singularmem_retrieve::wakeup::ScopeSet::for_project(dir, include_files)`,
   which canonicalises the path the same way the CLI's `wake-up` does,
   so symlinked checkouts match the scopes the hooks wrote.

### `memory_wakeup` tool

Always listed (reader). Arguments:

| name | type | default | meaning |
|---|---|---|---|
| `project` | string | server default | directory whose scopes to read |
| `include_files` | bool | false | also include `files/<basename>` |
| `limit` | integer ≥ 1 | 20 | most recent items to consider |
| `max_bytes` | integer ≥ 256 | 8192 | output budget, oldest blocks dropped first |
| `adapter` | string | server default adapter | `plain`, `claude`, `openai`, `gemini` |

Output: one text content block, byte-identical to what
`singularmem wake-up --project <dir>` prints with the same options:
the header line
`# Singularmem wake-up — <scopes> — <total> items, showing last <kept>`
followed by the adapter-rendered blocks, produced by
`wakeup::build` + `wakeup::render`. An empty project is not an error:
the header reports `0 items, showing last 0` and nothing follows.

Description (model-facing): "Call at the start of a session to load the
project's recent memory. Returns the same context the editor hooks
inject. Prefer `memory_retrieve` for a specific question."

### `wake-up` prompt

The server enables the `prompts` capability. `prompts/list` returns one
prompt:

- name `wake-up`, description "Recent memory for the current project,
  ready to paste into context"
- arguments: `project` (optional, string)

`prompts/get` returns a single `user` message whose text is exactly the
`memory_wakeup` output for that project with all other options at
their defaults. Unknown prompt names return the rmcp not-found error.

### `memory_graph_entities` tool

Reader. Arguments `kind?` (string), `scope?` (string), `scope_exact?`
(bool). Output: one line per entity in the store's `entities` order
(name ascending), tab-separated `id`, `name`, `kind` (`-` when absent),
`fact_count`, exactly as the CLI's `graph entities` human output; `"No
entities."` when empty. The scope filters entities to those with at
least one fact in scope, as the CLI does (delegates to
`Store::entities`).

### `memory_graph_history` tool

Reader. Argument `fact_id` (string, ULID). Output: the revision chain
oldest first, one rendered fact per line in the shared
`render_fact` format. `FactIdNotFound` and a malformed id map to
`invalid_params`.

Tool counts: 15 normally, 11 in read-only mode (the 3 graph writers and
`memory_ingest` hidden). The wire tests assert both counts and the
names.

### Resources

The server enables the `resources` capability.

- `resources/templates/list` returns one template:
  `uriTemplate: "singularmem://memory/{id}"`, name `memory`, mime
  `text/plain`, description "A single memory by ULID".
- `resources/list` returns an empty list.
- `resources/read` with `singularmem://memory/<ulid>` returns one text
  contents entry (`mimeType: text/plain`, same `uri`) formatted as:

  ```
  id: <ulid>
  created_at: <rfc3339>
  scope: <scope or ->
  source: <source or ->
  tags: <comma-separated or ->

  <content>
  ```

- Any other scheme or path shape → `resource_not_found`; a well-formed
  URI with an unknown or malformed ULID → `resource_not_found` with the
  id in the message. Read-only mode changes nothing here.

### Error mapping

| condition | MCP error |
|---|---|
| `Validation`, `FactNotFound`, `FactIdNotFound`, `InvalidId`, `ReadOnly` | `invalid_params` |
| project path missing / not a directory | `invalid_params` naming the path |
| unknown adapter name | `invalid_params` listing known adapters |
| unknown prompt name | rmcp `invalid_params` "prompt not found" |
| unknown resource | `resource_not_found` |
| anything else | `internal_error` |

### `memory_retrieve` description

Gains one sentence: "For a session's opening context, call
`memory_wakeup` instead."

## Node binding

New file `crates/singularmem-node/src/graph.rs` holding a second
`#[napi] impl Store` block (napi-rs merges impl blocks; if the build
rejects that, the methods live in `store.rs` and only the helpers move
to `graph.rs`) and `crates/singularmem-node/src/wakeup.rs`. All methods
are async (`AsyncTask`) like the existing ones. Types in `types.rs`.

### Types (TypeScript view)

```ts
interface EntityRef { id: string; name: string }
type FactObject = { entity: EntityRef } | { value: string }
interface Fact {
  id: string; subject: EntityRef; predicate: string; object: FactObject;
  validFrom: string | null; validTo: string | null; confidence: number;
  sourceItemId: string | null; scope: string | null; supersedes: string | null;
  recordedAt: string;
}
interface NewFact {
  subject: string; predicate: string; object: string; objectIsValue?: boolean;
  subjectKind?: string; objectKind?: string; validFrom?: string; validTo?: string;
  confidence?: number; sourceItemId?: string; scope?: string;
}
interface GraphQueryOptions { direction?: "outgoing" | "incoming" | "both";
  asOf?: string; recordedAt?: string; scope?: string; scopeExact?: boolean }
interface FactChangeOptions { objectIsValue?: boolean; at?: string; scope?: string }
interface TimelineEntry { fact: Fact; current: boolean }
interface GraphStats { entities: number; openFacts: number; closedFacts: number; predicates: number }
interface EntitySummary { id: string; name: string; kind: string | null; factCount: number }
interface WakeupOptions { project?: string; includeFiles?: boolean; limit?: number;
  maxBytes?: number; adapter?: "plain" | "claude" | "openai" | "gemini" }
interface Wakeup { text: string; total: number; shown: number; scopes: string[] }
```

### Methods on `Store`

| method | delegates to | returns |
|---|---|---|
| `addFact(fact: NewFact)` | `Store::add_fact` | `Fact` |
| `queryEntity(name, opts?)` | `Store::query_entity` | `Fact[]` |
| `queryPredicate(predicate, opts?)` (`direction` ignored) | `Store::query_predicate` | `Fact[]` |
| `invalidateFact(subject, predicate, object, opts?)` | `Store::invalidate_fact` | `Fact` |
| `supersedeFact(subject, predicate, oldObject, newObject, opts?)` | `Store::supersede_fact` | `{ closed: Fact \| null; opened: Fact }` |
| `timeline(entity?, opts?)` | `Store::timeline` | `TimelineEntry[]` |
| `graphStats(opts?)` | `Store::graph_stats` | `GraphStats` |
| `entities(opts?)` (`kind`, `scope`, `scopeExact`) | `Store::entities` | `EntitySummary[]` |
| `factHistory(factId)` | `Store::fact_history` | `Fact[]` |
| `wakeup(opts?)` | `wakeup::build` + `render` | `Wakeup` |

Timestamps in: any string accepted by `graph::time::parse_point`
(`YYYY-MM-DD` or RFC 3339). Timestamps out: jiff `Display` (RFC 3339,
UTC). `objectIsValue` applies to both `oldObject` and `newObject` in
`supersedeFact`, matching the CLI's `--value`.

`wakeup` project resolution: `opts.project` else `process.cwd()` (the
binding has no server config). Unknown `adapter` → `Validation` coded
error listing the four names.

### Errors

Reuse `error.rs`'s coded mapping, which already covers
`FactNotFound`, `FactIdNotFound`, `AmbiguousFactRevision`, `InvalidId`,
`Validation`, `ReadOnly`. A bad timestamp string is a `Validation`
error whose `field` is the option name (`validFrom`, `asOf`, …).

## Testing

All offline.

- MCP unit tests (`tools/wakeup.rs`, `tools/graph.rs`, `resources.rs`,
  `prompts.rs`): seeded temp store with items under
  `claude-code/proj-a`, `claude-code/proj-b`, `files/proj-a`;
  wake-up for a real temp directory named `proj-a` returns only
  `proj-a` items, `include_files` adds the `files/` item, a symlink to
  that directory yields the same header scopes, `max_bytes` drops
  oldest blocks, unknown adapter → invalid params, missing directory →
  invalid params; entities and history text; resource read known /
  unknown / malformed; prompt list has one prompt, prompt get returns
  the wake-up text; read-only server still serves wake-up, entities,
  history, resources.
- Wire tests: 15 / 11 tools, capability flags for prompts and
  resources present in `initialize`, `prompts/list` and
  `resources/templates/list` shapes.
- Node: `test/graph.test.mjs` (add → query, as-of before/after
  supersede, invalidate → empty, timeline `[closed, current]` order,
  stats, entities, history, coded errors for `FactNotFound` and a bad
  timestamp) and `test/wakeup.test.mjs` (seed under
  `claude-code/<tmpdir basename>`, `wakeup({project: tmpdir})` returns
  the header with that scope and total). `npm run typecheck` covers the
  declarations.

## Documentation

- `docs/mcp-server.md`: tool list (15), prompt, resource template,
  `--project`, read-only count 11.
- `crates/singularmem-mcp/README.md`: a section per new tool, the
  prompt, the resource, the flag; client config example with
  `SINGULARMEM_PROJECT`.
- `crates/singularmem-node/README.md`: "Knowledge graph" and "Wake-up"
  sections with examples.
- README status line: "MCP wake-up and resources, Node graph API".

## Acceptance criteria

1. `singularmem-mcp` lists 15 tools (11 with `--read-only`); `initialize`
   advertises tools, prompts, resources.
2. `memory_wakeup` output equals `singularmem wake-up --project <dir>`
   output for the same store and options (asserted in a test that runs
   both code paths against one store).
3. `prompts/get wake-up` returns that same text as one user message.
4. `resources/read singularmem://memory/<id>` returns the item;
   unknown id → `resource_not_found`.
5. The Node suite passes with the new files; `npm run typecheck` clean.
6. Workspace fmt, clippy (pedantic + nursery, `-D warnings`), and tests
   clean.

## Deviations

None yet. Record implementation-time deviations here.
