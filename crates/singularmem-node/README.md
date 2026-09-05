# singularmem (Node.js bindings)

Native Node.js bindings for [Singularmem](https://github.com/bromso/singularmem), a local-first persistent memory layer for LLM workflows.

## Install

```bash
npm install singularmem
```

That's it. Prebuilt native bindings are available for these platforms:

| Platform               | Architecture            | Notes                                |
|------------------------|-------------------------|--------------------------------------|
| Linux                  | x86_64 (glibc)          |                                      |
| macOS                  | x86_64 (Intel)          |                                      |
| macOS                  | ARM64 (Apple Silicon)   |                                      |
| Windows                | x86_64 (MSVC)           |                                      |

Node.js 20.12 or newer is required. No Rust toolchain needed on the supported platforms.

For other platforms (Linux ARM64, Alpine Linux/musl, FreeBSD, RISC-V, etc.), see [Building from source](#building-from-source) below.

## Usage

```javascript
import { Store } from 'singularmem';

const store = await Store.open('./memory.db');

const items = await store.list({ tags: ['recipes'], limit: 10 });
for (const item of items) {
  console.log(item.id, item.createdAt.toISOString(), item.content);
}

const oldest = items[0];
const chain = await store.revisions(oldest.id);
console.log(`item has ${chain.length} revisions`);
```

## Read-only mode

```javascript
const store = await Store.open('./memory.db', { readOnly: true });
```

Read-only stores reject every write at the SQLite layer.

## Error handling

All thrown errors have a structured `code` field:

```javascript
try {
  await store.get('not-a-ulid');
} catch (err) {
  if (err.code === 'InvalidId') {
    // ...
  }
}
```

Possible codes:

| Code | Meaning |
|---|---|
| `NotFound` | The requested item does not exist |
| `Validation` | Input failed validation (e.g. empty content) |
| `SupersedesNotFound` | A `supersedes` target was not found |
| `AmbiguousLatest` | The revision chain forks |
| `UnsupportedFormatVersion` | The store file is newer than this binding supports |
| `ReadOnly` | A write was attempted on a read-only store |
| `InvalidId` | A ULID string was malformed |
| `InvalidStorePath` | The store path is empty or otherwise invalid |
| `Sqlite` | Underlying SQLite error |
| `Io` | Filesystem or I/O error |
| `Json` | JSON serialisation/deserialisation error |
| `FactNotFound` | `invalidateFact`/`supersedeFact` found no open fact matching the triple |
| `FactIdNotFound` | `factHistory` was given a well-formed ULID that names no fact |
| `AmbiguousFactRevision` | A fact's revision chain forks; the library refuses to pick a branch |

## API

See `index.d.ts` for the full TypeScript surface. The current public API is:

- `Store.open(path, options?)` — async static factory
- `store.get(id)` — async point lookup
- `store.list(options?)` — async list with optional `{ tags?, limit?, scope?, scopeExact? }`
- `store.revisions(id)` — async revision chain (oldest → newest)
- `store.search(query, options?)` — hybrid search over Tantivy + USearch indexes (options include `scope?`, `scopeExact?`)
- `store.retrieve(query, options?)` — search + context assembly, ready for adapters (options include `scope?`, `scopeExact?`)
- `store.formatVersion()` — on-disk format version string
- `store.export()` — full JSONL dump
- `store.scopes()` — distinct scope paths with item counts, sorted by path
- `store.setScope(id, scope)` — move an item to `scope` (or clear it with `null`/`undefined`), no new revision
- `store.addFact(fact)` / `store.queryEntity(name, options?)` / `store.queryPredicate(predicate, options?)` / `store.invalidateFact(...)` / `store.supersedeFact(...)` / `store.timeline(entity?, options?)` / `store.graphStats(options?)` / `store.entities(options?)` / `store.factHistory(factId)` — the knowledge graph (see [Knowledge graph](#knowledge-graph))
- `store.wakeup(options?)` — a project's recent memory, rendered for a prompt (see [Wake-up](#wake-up))

## Search

Run a hybrid search over the store's indexes (Tantivy lexical + USearch
semantic + RRF fusion).

```javascript
import { Store } from 'singularmem';

const store = await Store.open('./memory.db');

const results = await store.search('cat care', {
  mode: 'hybrid',     // 'auto' (default) | 'lexical' | 'semantic' | 'hybrid'
  limit: 10,          // default 10
  fetchMultiplier: 3, // default 3
  rrfK: 60,           // default 60
});

for (const hit of results.hits) {
  console.log(hit.score, hit.kind, hit.item.content);
}
```

Indexes must exist on disk before `search()` can find anything. Build
them via the CLI: `singularmem reindex --with-embeddings --store ./memory.db`.

Mode `'auto'` probes for what's available and degrades. Explicit modes
fail with `code: 'IndexMissing'` or `code: 'HybridMissingIndex'` if a
required sidecar is absent.

## Retrieve

Higher-level convenience that runs search, fetches the full content per
hit, and returns a structured `RetrievedContext` suitable for passing to
an adapter.

```javascript
import { Store, adapters } from 'singularmem';

const store = await Store.open('./memory.db');
const ctx = await store.retrieve('cat care', {
  minScore: 0.1,
  limit: 5,
});

const prompt = adapters.claude.format(ctx);
```

## Ingest

Persist new items to the store:

```javascript
import { Store } from 'singularmem';

const store = await Store.open('./memory.db');

const item = await store.ingest({
  content: 'cats are great pets',
  tags: ['recipes', 'cats'],
  source: 'morning-notes',
  metadata: { authorId: 42 },
  scope: 'Team/Pets', // normalised (lowercased) to 'team/pets'
});

console.log(item.id, item.createdAt);
```

If Tantivy + USearch sidecars exist at the store path (created by
`singularmem reindex --with-embeddings`), the new item is automatically
written to those indexes too — `store.search()` will find it
immediately. If no sidecars exist, ingest writes SQLite only; run
`reindex` later to make older content searchable.

### Supersession

To revise an existing item, pass its ULID as `supersedes`:

```javascript
const v1 = await store.ingest({ content: 'old version' });
const v2 = await store.ingest({ content: 'new version', supersedes: v1.id });

// store.revisions(v2.id) returns [v1, v2] in oldest→newest order
```

### Read-only stores

Opening a store with `{ readOnly: true }` causes `ingest()` to reject
with `code: 'ReadOnly'`.

## Scoping

Items can carry a hierarchical scope path (e.g. `"team/backend"`), set via
`NewItem.scope` at ingest time. Paths are validated and lowercased; a
malformed path (double slash, `.`/`..` segment, disallowed character, etc.)
rejects with `code: 'Validation'`.

`list()`, `search()`, and `retrieve()` all accept `{ scope?, scopeExact? }`:
by default `scope` matches that path and everything beneath it; pass
`scopeExact: true` to match only the exact path.

```javascript
await store.ingest({ content: 'backend runbook', scope: 'team/backend' });

// Matches 'team/backend' and any deeper scope, e.g. 'team/backend/oncall'.
const items = await store.list({ scope: 'team' });

// Matches only the exact scope.
const exact = await store.list({ scope: 'team/backend', scopeExact: true });
```

`store.scopes()` lists every distinct scope with its item count:

```javascript
const counts = await store.scopes(); // [{ path: 'team/backend', count: 3 }, ...]
```

`store.setScope(id, scope)` moves an item to a new scope (or clears it with
`null`/`undefined`) without creating a new revision. Tantivy's copy of the
scope is not updated until the next `singularmem reindex`.

```javascript
await store.setScope(item.id, 'team/frontend');
await store.setScope(item.id, null); // clears the scope
```

## Knowledge graph

Record and query subject–predicate–object facts alongside your items.
Facts are versioned: `invalidateFact` closes a standing fact, and
`supersedeFact` closes one and opens its replacement in a single
transaction. Nothing is ever overwritten — `factHistory` walks the full
revision chain.

```javascript
import { Store } from 'singularmem';

const store = await Store.open('./memory.db');

const fact = await store.addFact({
  subject: 'Singularmem',
  predicate: 'uses',
  object: 'Tantivy',
  validFrom: '2026-05-16',
});
```

The object of a fact is either another entity or a literal value —
exactly one of `FactObject.entity` / `FactObject.value` is set; the other
is `undefined` (napi omits `None` fields entirely rather than emitting
`null` — see [Error handling](#error-handling) below for the general rule):

```javascript
fact.object; // { entity: { id: '01H...', name: 'Tantivy' } } — .value is undefined
```

Query an entity as of a point in time:

```javascript
const asOfMay = await store.queryEntity('Singularmem', { asOf: '2026-05-20' });
```

Replace a fact with its successor in one transaction:

```javascript
const { closed, opened } = await store.supersedeFact(
  'Singularmem', 'uses', 'Tantivy', 'Meilisearch',
);
// closed is undefined if there was no matching open fact to close
```

Walk a project's history and its aggregate shape:

```javascript
const timeline = await store.timeline('Singularmem'); // [{ fact, current }, ...]
const stats = await store.graphStats();               // { entities, openFacts, closedFacts, predicates }
const entities = await store.entities({ kind: 'library' });
const chain = await store.factHistory(fact.id);        // every revision, oldest first
```

`queryEntity`, `queryPredicate`, `timeline`, `graphStats` and `entities`
all accept a `scope` / `scopeExact` filter, same semantics as
[Scoping](#scoping). See `index.d.ts` for every option field
(`GraphQueryOptions`, `FactChangeOptions`, `GraphScopeOptions`,
`EntityListOptions`) and the `Fact` / `TimelineEntry` / `GraphStats` /
`EntitySummary` / `SupersedeResult` return shapes.

Graph-specific error codes: `FactNotFound` (no open head matches
`invalidateFact`'s triple), `FactIdNotFound` / `AmbiguousFactRevision`
(bad or forking `factHistory` chain) — on top of the codes in
[Error handling](#error-handling).

## Wake-up

The same "what happened recently in this project" context the editor
hooks inject at session start, available directly:

```javascript
import { Store } from 'singularmem';

const store = await Store.open('./memory.db');

const w = await store.wakeup({ project: process.cwd() });
console.log(w.text);   // rendered prompt, header + blocks
console.log(w.total);  // items matching the scope set
console.log(w.shown);  // items considered after `limit`; the header inside `text` reports how many survived `maxBytes`
console.log(w.scopes); // ['claude-code/<basename>', 'codex/<basename>', 'cursor/<basename>']
```

`options.project` defaults to `process.cwd()` — this binding has no
server-side project config to fall back to. Scopes are derived from its
**raw** (uncanonicalised) basename, so a symlinked project directory reads
under the link's name, matching what the editor hooks wrote at save time.

| Option | Default | Meaning |
|---|---|---|
| `project` | `process.cwd()` | Project directory; its basename drives the scope set. |
| `includeFiles` | `false` | Also read `files/<basename>` (`singularmem ingest-dir` output). |
| `limit` | `20` | Most recent items to consider, across all scopes. |
| `maxBytes` | `8192` | Output budget; oldest blocks are dropped first, the header always survives. |
| `adapter` | `"plain"` | Prompt formatter: `"plain"`, `"claude"`, `"openai"` or `"gemini"`. |

An unknown `adapter` or a `project` that isn't a directory rejects with
`code: 'Validation'`.

## Adapters

Four pre-built adapters cover the constitutional Principle II providers:

- `adapters.plain` — Markdown blocks with `## memory N` headings
- `adapters.claude` — Anthropic `<documents><document index="N">` XML
- `adapters.openai` — Bracketed `[N]` citations with leading instruction
- `adapters.gemini` — Em-dash `Source N` headers with grounding directive

Each exposes a `name` property and a synchronous `format(ctx)` method:

```javascript
adapters.claude.format(ctx);  // returns string
```

Custom JS adapters are not supported in this release — if you need a
different format, build the string yourself from the `RetrievedContext`
that `store.retrieve()` returns.

## Error handling (5b additions)

In addition to the 5a error codes, search and retrieve can throw:

| Code | Meaning |
|---|---|
| `NoIndexes` | `mode: 'auto'` but no sidecar indexes exist |
| `IndexMissing` | Explicit mode requires a sidecar that's absent |
| `HybridMissingIndex` | `mode: 'hybrid'` but one of the two sidecars is missing |
| `EmptyQuery` | `store.retrieve('')` rejects with this (search returns empty hits instead) |
| `QueryParse` | Tantivy query syntax error |
| `Tantivy` | Tantivy-specific runtime error |
| `Usearch` | USearch-specific runtime error |
| `Embedding` | Embedder runtime error |
| `ModelDownload` | fastembed model download failure |
| `InvalidModelFiles` | Embedder model files malformed |
| `DimMismatch` | Vector dimension mismatch |
| `ModelMismatch` | Sidecar built with a different embedder model |
| `IndexCorrupted` | Sidecar exists but is unreadable |

## Versioning

The npm package version tracks the workspace version of the underlying Rust crates. A CI check verifies they stay in sync.

## Building from source

If your platform isn't in the prebuilt set, or you want to hack on Singularmem:

1. Install [Rust](https://rustup.rs) (1.80 or newer)
2. Install Node.js 20.12 or newer
3. Clone the repository:
   ```bash
   git clone https://github.com/bromso/singularmem.git
   cd singularmem/crates/singularmem-node
   ```
4. Build:
   ```bash
   npm install
   npm run build
   ```
5. The build produces a `singularmem.<triple>.node` in the package directory. To use it from another project, either:
   - Add the cloned repo as a [local file dependency](https://docs.npmjs.com/cli/v10/configuring-npm/package-json#local-paths): `npm install /path/to/singularmem/crates/singularmem-node`
   - Or copy the built `.node` and the patched `index.js` + `index.d.ts` into your own `node_modules/singularmem/` directory

If you publish a third-party prebuilt binary for an unsupported platform, please open an issue so we can consider adding it to the official matrix.

## License

Apache-2.0
