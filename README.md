# Singularmem

Singularmem is a local-first persistent memory layer for LLM-driven
workflows. It stores, indexes, and exposes the artefacts a developer
or agent accumulates over time — conversations, files, decisions,
embeddings, provenance — and bridges them to any LLM provider through
a stable, vendor-neutral interface.

> **Status:** Pre-v0.1 · bootstrap phase · constitution v0.2.0
> ratified 2026-05-15. No usable functionality yet beyond a version
> probe.

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

## Build

This repository currently builds a do-nothing CLI binary that exists
only to verify the build pipeline.

```bash
cargo build
./target/debug/singularmem
# → singularmem 0.0.0
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every commit must be signed
off (`git commit -s`); there is no CLA.

## License

Open components: [Apache-2.0](LICENSE). Proprietary components are
governed by a separate commercial license (terms TBD with the first
proprietary release).
