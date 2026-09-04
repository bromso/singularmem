# Singularmem

Singularmem is a local-first persistent memory layer for LLM-driven
workflows. It stores, indexes, and exposes the artefacts a developer
or agent accumulates over time — conversations, files, decisions,
embeddings, provenance — and bridges them to any LLM provider through
a stable, vendor-neutral interface.

> **Status:** v0.16.0, plus unreleased transcript ingestion (v0.17.0) and
> scoping (v0.18.0) — memory store, hybrid search (Tantivy + USearch),
> provider adapters, MCP server, TypeScript SDK, bulk transcript ingestion,
> and hierarchical item scoping. Constitution v0.2.0 ratified 2026-05-15.

## Open core

Singularmem ships as **open core**:

- The **open** components — memory engine, on-disk format, indexes,
  embedding pipeline, LLM provider adapters, CLI, MCP server, library
  SDK, and the TypeScript binding — are licensed under
  [Apache-2.0](LICENSE) and live in this repository.
- The **proprietary** components — the desktop GUI (Flutter), premium
  visualisations, and cross-device sync — are sold under a separate
  commercial license to sustain development.

The boundary between the two is a [constitutional matter](.specify/memory/constitution.md#open--closed-split),
not a product-management one. The constitution's Principle III.a is a
**one-way ratchet**: features may move from proprietary to open, never
the reverse.

## Quickstart

```bash
# Make every past Claude Code session searchable
singularmem ingest-transcript            # defaults to ~/.claude/projects
singularmem ingest-transcript --project "$PWD"   # only sessions from this repo

# Index a source tree (honours .gitignore; re-runs only pick up changes)
singularmem ingest-dir .

singularmem search "why did we pick tantivy"
singularmem retrieve --adapter claude "release process"

# Every item gets a default scope (claude-code/<project> or files/<dirname>);
# use it to narrow a search or see what's in the store
singularmem search --scope claude-code/singularmem "why tantivy"
singularmem scope list
```

Both bulk verbs are idempotent: re-running ingests nothing already present.

Two known limitations of the bulk verbs, both tracked for a follow-up
(details in [docs/formats/store-v2.md](docs/formats/store-v2.md#known-limitations)):

- Superseded items stay in the search indexes until `singularmem
  reindex`, so repeated `ingest-dir` runs over a changing tree
  accumulate stale search hits.
- If a file's chunk count changes between runs its `external_id` shape
  changes too (`file:<path>` vs `file:<path>#n`), so the old item is
  orphaned rather than superseded.

## Installing the CLI

The `singularmem` CLI and `singularmem-mcp` server ship as prebuilt binaries for:

| Platform | Architecture |
|---|---|
| Linux | x86_64 (glibc) |
| macOS | x86_64 (Intel) |
| macOS | ARM64 (Apple Silicon) |

For Windows, Linux ARM64, Alpine Linux/musl, FreeBSD, or other platforms, build from source (see [Building from source on GitHub](https://github.com/bromso/singularmem#building-from-source)). Windows MSVC prebuilt binaries are temporarily unavailable due to a CRT runtime-library mismatch in the ort-sys / cxx dependency chain (ONNX Runtime FFI via fastembed); tracked as a follow-up sub-project.

### Homebrew tap (macOS + Linux)

```bash
brew install bromso/tap/singularmem
```

Both `singularmem` and `singularmem-mcp` are placed on `PATH`.

### Curl-bash installer (Linux + macOS)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bromso/singularmem/releases/latest/download/singularmem-installer.sh | sh
```

### Manual download

Visit https://github.com/bromso/singularmem/releases/latest, download the archive matching your platform, extract, and add the contained `singularmem` + `singularmem-mcp` binaries to your `PATH`.

### Verify the install

```bash
singularmem --version
singularmem-mcp --version
```

Both should report the same version (the latest tagged release).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every commit must be signed
off (`git commit -s`); there is no CLA.

## License

Open components: [Apache-2.0](LICENSE). Proprietary components are
governed by a separate commercial license (terms TBD with the first
proprietary release).
