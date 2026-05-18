# singularmem (Node.js bindings)

Native Node.js bindings for [Singularmem](https://github.com/bromso/singularmem), a local-first persistent memory layer for LLM workflows.

## Installation

```bash
npm install singularmem
```

> **Note (sub-project 5a):** This package currently builds from source on install. Prebuilt platform binaries are planned for a future release.
> Building from source requires a Rust toolchain (rustup, cargo).

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

## API

See `index.d.ts` for the full TypeScript surface. The current public API is:

- `Store.open(path, options?)` — async static factory
- `store.get(id)` — async point lookup
- `store.list(options?)` — async list with optional `{ tags?, limit? }`
- `store.revisions(id)` — async revision chain (oldest → newest)
- `store.formatVersion()` — on-disk format version string
- `store.export()` — full JSONL dump

Search, retrieve, and write methods will land in subsequent sub-projects.

## Versioning

The npm package version tracks the workspace version of the underlying Rust crates. A CI check verifies they stay in sync.

## License

Apache-2.0
