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

## Installing the CLI

The `singularmem` CLI and `singularmem-mcp` server ship as prebuilt binaries for:

| Platform | Architecture |
|---|---|
| Linux | x86_64 (glibc) |
| macOS | x86_64 (Intel) |
| macOS | ARM64 (Apple Silicon) |
| Windows | x86_64 (MSVC) |

For Linux ARM64, Alpine Linux/musl, FreeBSD, or other platforms, build from source (see [Building from source on GitHub](https://github.com/bromso/singularmem#building-from-source)).

### Homebrew tap (macOS + Linux)

```bash
brew install bromso/tap/singularmem
```

Both `singularmem` and `singularmem-mcp` are placed on `PATH`.

### Curl-bash installer (Linux + macOS)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bromso/singularmem/releases/latest/download/singularmem-installer.sh | sh
```

### PowerShell installer (Windows)

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/bromso/singularmem/releases/latest/download/singularmem-installer.ps1 | iex"
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
