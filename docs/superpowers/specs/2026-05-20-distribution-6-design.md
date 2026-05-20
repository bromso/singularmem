# Distribution & Packaging (Sub-project 6) — Design Spec

**Date:** 2026-05-20
**Status:** Approved (pending user review of written spec)
**Sub-project:** 6 (distribution & packaging — npm publish only; CLI/MCP binary distribution deferred)
**Builds on:** 5a/5b/5c (TS SDK foundation + reads + search/retrieve + writes; `v0.13.0`)

## Summary

Makes `npm install singularmem` work without a Rust toolchain. Adopts the napi-rs industry-standard distribution pattern: a single main package (`singularmem`) carries the JS dispatcher + TS types but no binaries, and five per-platform sub-packages (`@bromso/singularmem-<triple>`) carry the prebuilt `.node` binaries. npm's `optionalDependencies` semantics install only the sub-package matching the consumer's platform. A new GitHub Actions workflow builds the matrix on every PR and publishes to npm on `v*.*.*` tag pushes with provenance attestation.

CLI binary distribution (Homebrew, cargo install, GitHub Releases via cargo-dist) and MCP server binary distribution are out of scope for 6 — they'll get their own later sub-project.

## Motivation

After 5c, the TS SDK is feature-complete for v0. JS consumers can do `npm install singularmem` today, but the install triggers `napi build`, which requires Rust + cargo. That's a non-starter for the majority of npm users.

Sub-project 6 closes the gap: install works on the 5 most common platforms with no toolchain. Build-from-source remains as the escape hatch for niche platforms (FreeBSD, RISC-V, etc.) and contributor workflows.

## Section 1 — Architecture overview

```
                      ┌──────────────────────────────────────┐
                      │  npm install singularmem              │
                      └──────────────────────────────────────┘
                                       │
                                       ▼
         ┌─────────────────────────────────────────────────────────────┐
         │  singularmem (main package)                                 │
         │  - index.js (platform-dispatcher: require's the right sub) │
         │  - index.d.ts (TS surface)                                  │
         │  - no .node binaries                                        │
         │  - optionalDependencies lists all 5 platform packages      │
         └─────────────────────────────────────────────────────────────┘
                                       │
                       npm installs ONLY the matching one
                                       │
       ┌───────────────┬───────────────┼───────────────┬────────────────┐
       ▼               ▼               ▼               ▼                ▼
  @bromso/         @bromso/         @bromso/         @bromso/         @bromso/
  singularmem-     singularmem-     singularmem-     singularmem-     singularmem-
  linux-x64-gnu    linux-arm64-gnu  darwin-x64       darwin-arm64     win32-x64-msvc
  - .node          - .node          - .node          - .node          - .node
  - package.json   - package.json   - package.json   - package.json   - package.json
    (os: linux,      (os: linux,      (os: darwin,     (os: darwin,     (os: win32,
     cpu: x64,       cpu: arm64,      cpu: x64)        cpu: arm64)      cpu: x64)
     libc: glibc)    libc: glibc)
```

**Key behaviors:**
- Consumers do `npm install singularmem`. npm reads `optionalDependencies`, sees 5 platform packages, attempts each, succeeds on the one matching `process.platform` + `process.arch` + libc, silently skips the others.
- The main package's `index.js` does runtime platform detection and `require()`s the matching `@bromso/singularmem-<triple>` binary.
- If no matching platform package is available (consumer on an unsupported arch), the runtime falls back to looking for a local `singularmem.<triple>.node` (source-build escape hatch).

**Decisions locked in this section:**
- Scoped sub-packages under `@bromso` (the maintainer's npm namespace; matches the GitHub org). Easy migration to `@singularmem/*` later if commercial split warrants.
- Source-build fallback preserved as the safety net for unsupported platforms.
- Main package version + all 5 sub-package versions kept in lockstep — bump together, publish together. Enforced by the tag-version-vs-package.json verification step.
- Platform matrix: `linux x64-gnu`, `linux arm64-gnu`, `darwin x64`, `darwin arm64`, `win32 x64-msvc`. ~95% of npm consumer environments. Skip musl, FreeBSD, Android, RISC-V (build-from-source only).

## Section 2 — package.json layout + the generated index.js

### Main package (`crates/singularmem-node/package.json`)

```jsonc
{
  "name": "singularmem",
  "version": "0.13.0",
  "description": "Local-first persistent memory for LLM workflows — native Node bindings",
  "main": "index.js",
  "types": "index.d.ts",
  "license": "Apache-2.0",
  "repository": {
    "type": "git",
    "url": "https://github.com/bromso/singularmem.git",
    "directory": "crates/singularmem-node"
  },
  "files": [
    "index.js",
    "index.d.ts",
    "README.md"
  ],
  "napi": {
    "name": "singularmem",
    "package": { "name": "@bromso/singularmem" },
    "triples": {
      "defaults": false,
      "additional": [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc"
      ]
    }
  },
  "scripts": {
    "build": "napi build --platform --release",
    "postbuild": "node scripts/patch-index.js",
    "build:debug": "napi build --platform",
    "postbuild:debug": "node scripts/patch-index.js",
    "test": "node --test test/*.test.mjs && npm run typecheck",
    "typecheck": "tsc --noEmit",
    "artifacts": "napi artifacts",
    "prepublishOnly": "napi prepublish -t npm --skip-gh-release"
  },
  "optionalDependencies": {
    "@bromso/singularmem-linux-x64-gnu": "0.13.0",
    "@bromso/singularmem-linux-arm64-gnu": "0.13.0",
    "@bromso/singularmem-darwin-x64": "0.13.0",
    "@bromso/singularmem-darwin-arm64": "0.13.0",
    "@bromso/singularmem-win32-x64-msvc": "0.13.0"
  },
  "devDependencies": {
    "@napi-rs/cli": "^2.18.0",
    "typescript": "^5.4.0"
  },
  "engines": { "node": ">=20.12.0" }
}
```

**Crucial differences from the current (5c) `package.json`:**
- `"defaults": false, "additional": [...5 explicit triples...]` — replaces the current `defaults: true` (which expanded to ~14 platforms, way over-broad). Forces napi-rs to emit a dispatcher that only knows about our 5.
- New `napi.package.name` field tells `@napi-rs/cli` which scope to use when generating sub-package names. Set to `@bromso/singularmem`; produces `@bromso/singularmem-linux-x64-gnu` etc.
- New `optionalDependencies` block with all 5 sub-packages pinned to the same version.
- `"files"` no longer lists `*.node` — the main package ships no binaries.
- New `artifacts` script (downloads built `.node` files from CI artifacts) and `prepublishOnly` (runs `napi prepublish` to assemble the per-platform packages before publishing).

### Per-platform sub-package (`npm/<triple>/package.json` — auto-generated)

`@napi-rs/cli`'s `napi prepublish` command auto-generates these. Each looks like:

```jsonc
{
  "name": "@bromso/singularmem-linux-x64-gnu",
  "version": "0.13.0",
  "os": ["linux"],
  "cpu": ["x64"],
  "main": "singularmem.linux-x64-gnu.node",
  "files": ["singularmem.linux-x64-gnu.node"],
  "license": "Apache-2.0",
  "engines": { "node": ">=20.12.0" },
  "libc": ["glibc"]
}
```

The `os`/`cpu`/`libc` fields tell npm which platform this sub-package targets. npm uses these as installation-time filters: a darwin-arm64 user sees the `linux-x64-gnu` sub-package in `optionalDependencies` but skips it because `os: ["linux"]` doesn't match `process.platform`.

The 5 sub-package `package.json` files don't need to be committed. The `npm/<triple>/` directory structure IS committed but only contains README.md + LICENSE templates that `napi prepublish` copies into each generated sub-package.

### Generated `index.js` — platform dispatcher

`napi build` generates a platform-dispatching `index.js` that detects the runtime platform and loads the right sub-package:

```javascript
// Auto-generated by @napi-rs/cli — DO NOT EDIT
const { existsSync, readFileSync } = require('fs')
const { join } = require('path')

const { platform, arch } = process

let nativeBinding = null
let loadError = null

switch (platform) {
  case 'darwin':
    if (arch === 'x64') {
      try { nativeBinding = require('@bromso/singularmem-darwin-x64') } catch (e) { loadError = e }
    } else if (arch === 'arm64') {
      try { nativeBinding = require('@bromso/singularmem-darwin-arm64') } catch (e) { loadError = e }
    }
    break
  case 'linux':
    if (arch === 'x64') {
      try { nativeBinding = require('@bromso/singularmem-linux-x64-gnu') } catch (e) { loadError = e }
    } else if (arch === 'arm64') {
      try { nativeBinding = require('@bromso/singularmem-linux-arm64-gnu') } catch (e) { loadError = e }
    }
    break
  case 'win32':
    if (arch === 'x64') {
      try { nativeBinding = require('@bromso/singularmem-win32-x64-msvc') } catch (e) { loadError = e }
    }
    break
}

// Source-build fallback (for unsupported platforms or local dev)
if (!nativeBinding) {
  const localFile = join(__dirname, `singularmem.${platform}-${arch}.node`)
  if (existsSync(localFile)) {
    nativeBinding = require(localFile)
  } else {
    throw new Error(
      `Failed to load native binding for ${platform}-${arch}. ` +
      `Either your platform isn't in the prebuilt set, or the install was incomplete. ` +
      `See https://github.com/bromso/singularmem/tree/main/crates/singularmem-node#building-from-source ` +
      `for build instructions. Underlying error: ${loadError?.message ?? 'unknown'}`
    )
  }
}

// post-build patch (scripts/patch-index.js) adds the JS wrapper Store class +
// adapters namespace + liftItem helper after this point — existing 5a/5b/5c
// logic, unchanged.
```

The `patch-index.js` post-build script still runs after every `napi build` and adds the JS wrapper class + adapters namespace + liftItem helper. Sub-project 6 doesn't change the patch; it just adapts to the new platform-dispatching shell.

**Decisions locked in this section:**
- `optionalDependencies` lists all 5 platform packages pinned to the exact version. npm's "best-effort install" semantics handle the per-platform filtering automatically.
- Per-platform `package.json` files are generated by `napi prepublish`, not hand-maintained. The 5 sub-package metadata (os/cpu/libc) lives in `package.json#napi.triples.additional` only.
- The source-build fallback (looking for `singularmem.<triple>.node` in `__dirname`) is preserved for local development — `npm run build` in the repo still works the same way and produces the local `.node` that the dispatcher finds.
- Main package `files` excludes binaries; binaries only ship in sub-packages.

## Section 3 — CI build matrix

### New workflow: `.github/workflows/npm-publish.yml`

Two jobs: a build matrix (5 platforms in parallel) and a publish job that downloads all 5 artifacts and pushes them to npm. The build runs on every PR to catch breakage; the publish runs only on tag pushes.

```yaml
name: npm publish

on:
  push:
    tags: ['v*.*.*']
  pull_request:
    paths:
      - 'crates/singularmem-node/**'
      - '.github/workflows/npm-publish.yml'
  workflow_dispatch:
    inputs:
      dry_run:
        description: 'Run build matrix without publishing'
        type: boolean
        default: true

jobs:
  build:
    name: build ${{ matrix.triple }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - triple: x86_64-unknown-linux-gnu
            runner: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - triple: aarch64-unknown-linux-gnu
            runner: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            cross: true
          - triple: x86_64-apple-darwin
            runner: macos-13
            target: x86_64-apple-darwin
          - triple: aarch64-apple-darwin
            runner: macos-14
            target: aarch64-apple-darwin
          - triple: x86_64-pc-windows-msvc
            runner: windows-latest
            target: x86_64-pc-windows-msvc
    runs-on: ${{ matrix.runner }}
    defaults:
      run:
        working-directory: crates/singularmem-node
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: ./
      - name: Install cross-rs (linux arm64 only)
        if: matrix.cross == true
        run: |
          cargo install cross --git https://github.com/cross-rs/cross
      - name: npm install
        run: npm install
      - name: Build native binding
        run: |
          if [ "${{ matrix.cross }}" = "true" ]; then
            npm run build -- --target ${{ matrix.target }} --use-cross
          else
            npm run build -- --target ${{ matrix.target }}
          fi
        shell: bash
      - name: Upload .node artifact
        uses: actions/upload-artifact@v4
        with:
          name: binding-${{ matrix.triple }}
          path: crates/singularmem-node/*.node
          if-no-files-found: error
          retention-days: 7
```

**Matrix decisions:**
- **`ubuntu-latest` for both linux x64 + linux arm64.** linux arm64 builds via `cross-rs` (a cargo wrapper that runs in a Docker container with the target toolchain). Avoids native arm64 CI runners which are slower and have fewer GH Actions minutes available.
- **`macos-13` for x86_64-apple-darwin** (Intel mac). Apple's last x86_64 macOS CI runners. GitHub may eventually drop these; if so, fall back to native compilation under Rosetta on `macos-14` via `cargo build --target x86_64-apple-darwin` (slower but works).
- **`macos-14` for aarch64-apple-darwin** (Apple Silicon). Native arm64 runners.
- **`windows-latest` for x86_64-pc-windows-msvc.** MSVC toolchain.
- **`fail-fast: false`** — if one platform breaks, the others still build (helps debug platform-specific issues).

### PR behavior

Every PR that touches `crates/singularmem-node/**` runs the full 5-platform build matrix. This catches build regressions before they reach a release tag. The artifacts are uploaded with 7-day retention so reviewers can download the binaries for ad-hoc testing if they want, but they don't get published.

The existing `node-bindings` CI job (Linux-only, runs the full test suite) stays as-is. PRs see both jobs: `node-bindings` for tests, `npm publish / build` for the matrix.

### Workflow concerns + costs

- **CI time**: ~5 min/platform in parallel = ~5-7 min wall-clock per PR (limited by the slowest platform). Existing PR CI is already ~7 min total, so this nearly doubles it for napi-touching PRs but doesn't slow other PRs.
- **GH Actions minutes**: macos runners cost 10x linux. Two macos runners per PR adds ~10 macos-minutes per PR. For a low-volume repo this is a non-issue; if it becomes a concern, gate the matrix on `paths` filters (already done in the workflow above).
- **`fail-fast: false`** means a flaky platform run doesn't kill the others — single-platform retries are cheap.

### Cross-compilation strategy for linux arm64

`cross-rs` runs the cargo build inside a Docker container that has the arm64 toolchain pre-installed. Standard napi-rs CI pattern. Alternatives considered:
- **Native arm64 runners** (`ubuntu-24.04-arm`): available in GH Actions but slower and more expensive. Defer.
- **`zig cc` linker** / `cargo-zigbuild`: adds complexity for no clear win.

## Section 4 — Publish workflow (tag-triggered + provenance)

### The `publish` job (same workflow, runs after all 5 builds succeed)

```yaml
  publish:
    name: publish to npm
    needs: build
    if: |
      startsWith(github.ref, 'refs/tags/v') &&
      (github.event_name != 'workflow_dispatch' || !inputs.dry_run)
    runs-on: ubuntu-latest
    permissions:
      id-token: write   # required for npm provenance via GitHub OIDC
      contents: read
    defaults:
      run:
        working-directory: crates/singularmem-node
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'
      - name: Verify tag version matches package version
        run: |
          TAG_VERSION="${GITHUB_REF#refs/tags/v}"
          PKG_VERSION=$(node -p "require('./package.json').version")
          test "$TAG_VERSION" = "$PKG_VERSION" || {
            echo "::error::tag v$TAG_VERSION doesn't match package.json version $PKG_VERSION"
            exit 1
          }
      - name: npm install
        run: npm install
      - name: Download all platform artifacts
        uses: actions/download-artifact@v4
        with:
          path: crates/singularmem-node/artifacts
          pattern: binding-*
          merge-multiple: true
      - name: Move artifacts into place
        run: npm run artifacts
      - name: Assemble per-platform packages
        run: npx napi prepublish -t npm --skip-gh-release
      - name: List what's about to publish (for log clarity)
        run: |
          ls -la npm/
          for dir in npm/*/; do
            echo "=== $dir ==="
            cat "${dir}package.json"
          done
      - name: Publish to npm with provenance
        run: |
          for dir in npm/*/; do
            cd "$dir"
            npm publish --access public --provenance
            cd ../..
          done
          npm publish --access public --provenance
        env:
          NPM_CONFIG_PROVENANCE: true
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
```

### The publish flow, end to end

1. Maintainer pushes a `v0.X.0` tag
2. GitHub Actions triggers the `npm publish` workflow
3. The `build` job's 5-platform matrix runs in parallel — each produces a `singularmem.<triple>.node` artifact
4. After ALL 5 succeed, the `publish` job runs on ubuntu-latest
5. `publish` checks out the tagged commit, downloads the 5 artifacts, runs `napi prepublish` to assemble the per-platform `npm/<triple>/` directories
6. Each `npm/<triple>/` gets `npm publish --provenance` — produces 5 sub-package publishes (`@bromso/singularmem-linux-x64-gnu@0.X.0`, etc.)
7. Finally the main `singularmem@0.X.0` package publishes — its `optionalDependencies` now point at the just-published sub-packages
8. npm's transparency log records the provenance attestations linking each package back to the GitHub workflow run that built it

### Provenance attestation

`npm publish --provenance` works by:
- GitHub Actions issues an OIDC token signed by GitHub (encoding repo + commit + workflow run)
- npm CLI sends this token to npm registry alongside the publish
- npm verifies the OIDC signature with GitHub and records the attestation on Sigstore's public transparency log
- Consumers can run `npm audit signatures` to verify the package was built by the claimed workflow

No keys to manage. No secrets to rotate beyond `NPM_TOKEN`. The `id-token: write` permission grants GitHub permission to mint the OIDC token for the workflow.

### NPM_TOKEN setup (one-time, manual)

The maintainer needs to:
1. Create an "automation" token on https://www.npmjs.com/settings/<user>/tokens (npm automation tokens bypass 2FA for CI)
2. Add it as repo secret `NPM_TOKEN` in https://github.com/bromso/singularmem/settings/secrets/actions

Documented in a new `crates/singularmem-node/RELEASING.md`. One-time setup.

### Dry-run mode

The workflow's `workflow_dispatch` trigger has a `dry_run` boolean input (default `true`). When set:
- The build matrix runs
- Artifacts upload
- The `publish` job's `if:` condition evaluates `false` (matches `dry_run == true`)
- No npm publishes happen

Lets the maintainer trigger the matrix manually and verify everything works without pushing to npm. Useful for the first few releases until trust in the workflow is built.

### Version-drift safety

The "Verify tag version matches package version" step prevents accidentally publishing the wrong version. If someone pushes `v0.14.0` but `package.json` still says `0.13.0`, the workflow fails before any publishes happen.

### Tag → release post-merge ceremony

After 6 lands, the post-merge ceremony for each new sub-project becomes:
1. Merge PR
2. `git tag -a v0.X.0 -m "..."` + `git push origin v0.X.0`
3. Wait ~10 min — npm publish workflow runs automatically
4. Verify: `npm view singularmem version` shows the new version

No manual `npm publish` step ever runs on the maintainer's machine.

## Section 5 — Local dev workflow (unchanged)

The whole point of sub-project 6 is that nothing changes for local developers. The existing 5a-5c workflow stays:

```bash
cd crates/singularmem-node
npm install            # installs @napi-rs/cli + typescript dev deps
npm run build          # cargo build + napi codegen + post-build patch
npm test               # node --test test/*.test.mjs + tsc --noEmit
```

The local `napi build` produces `singularmem.<triple>.node` in the crate directory. The platform-dispatching `index.js` (regenerated by `napi build`) still has the source-build fallback that finds this local file — so `node -e "require('./')"` works the same way it did in 5a.

**What's different is gitignore:** the local `.gitignore` already excludes `*.node` and `node_modules/`. Sub-project 6 adds:

```
artifacts/
npm/*/singularmem.*.node
```

The `npm/` directory layout (with one subdirectory per triple) gets committed because it holds the per-platform README + LICENSE templates that `napi prepublish` uses. Only the `.node` files inside it are gitignored.

### Source-build escape hatch for unsupported platforms

A consumer on, say, FreeBSD x64 (not in our matrix) does `npm install singularmem`. npm tries to install all 5 `optionalDependencies` — none match `os: freebsd`, so all are silently skipped. Then `require('singularmem')` runs the platform dispatcher in `index.js`, which:

1. Tries `require('@bromso/singularmem-linux-x64-gnu')` etc. — all throw MODULE_NOT_FOUND (none installed)
2. Falls back to looking for a local `singularmem.<platform>-<arch>.node` in the package's own directory — also not there (no prebuilt for FreeBSD)
3. Throws the documented error: `Failed to load native binding for freebsd-x64. ... See <link> for build instructions.`

The README's "Building from source" section (Section 6 below) tells them how to clone the repo and run `npm install` → `napi build`. They can then `npm link` or copy the produced `.node` into their `node_modules/singularmem/`.

## Section 6 — Acceptance criteria + Constitution Check

### Acceptance criteria

**Package layout:**
- `crates/singularmem-node/package.json` has `optionalDependencies` listing the 5 `@bromso/singularmem-<triple>` sub-packages, all pinned to the workspace version
- `napi.triples` has `defaults: false` and an explicit 5-triple `additional` list
- `napi.package.name` is `@bromso/singularmem` (drives the sub-package naming)
- Main package's `files` excludes `*.node` — binaries only ship in sub-packages
- 5 sub-package template directories exist under `crates/singularmem-node/npm/<triple>/`

**CI workflow:**
- `.github/workflows/npm-publish.yml` exists with two jobs: `build` (5-platform matrix) and `publish` (tag-gated)
- The `build` matrix has `fail-fast: false` and uses native runners for darwin x64/arm64 and windows; `cross-rs` for linux arm64
- Every PR touching `crates/singularmem-node/**` runs the matrix
- The `publish` job runs only on `v*.*.*` tag pushes (or `workflow_dispatch` with `dry_run: false`)
- The `publish` job has `id-token: write` permission (required for npm provenance)
- A tag-version-vs-package.json check runs before any actual publish
- `workflow_dispatch` dry-run mode is available (artifacts produced, no npm publishes; default = dry-run)

**Source-build fallback:**
- The generated `index.js` falls back to a local `singularmem.<triple>.node` if no platform sub-package is installed
- Consumers on unsupported platforms get a clear error message linking to build-from-source instructions

**Documentation:**
- New `crates/singularmem-node/RELEASING.md` covering: NPM_TOKEN setup, the tag-push release flow, how to verify provenance after publish, how to recover from a failed publish
- README extended with a "Building from source" section for unsupported platforms
- README "Installation" section updated to say "Just `npm install singularmem` — no toolchain required on supported platforms"

**One-time manual setup (NOT in this PR but documented):**
- NPM_TOKEN GitHub repo secret created (npm automation token)
- npm user that owns the token has publish access to the unscoped `singularmem` name and to the `@bromso` scope
- First successful publish creates the npm packages; subsequent publishes are automatic via tag push

### Out of scope

- CLI binary distribution (a separate later sub-project — Homebrew tap, cargo install via crates.io, GitHub Releases with cargo-dist)
- MCP server binary distribution (same — bundled with the CLI sub-project)
- musl/Alpine Linux support (deferred to a future "container support" sub-project)
- ARM64 macOS notarization / codesigning (npm doesn't require it)
- Windows ARM64, RISC-V, FreeBSD (build-from-source only)

### Constitution Check (v0.2.0)

- **I. Local-first, file-based** ✅ — packaging only; no runtime behavior change.
- **II. Provider-agnostic** ✅ — adapters unchanged.
- **III. Append-only, revisable** ✅ — no data layer changes.
- **IV. Open core** ✅ — Apache-2.0 throughout. Per-platform sub-packages carry the same license file.
- **V. Stable on-disk format** ✅ — no format changes.
- **VI. Single binary, zero deps** — partial / improving. Compiled binaries are now distributed (one per platform, ~3-5 MB each). The "consumer needs Node 20.12+" constraint from 5a stays; that's the Node runtime requirement, not a Singularmem dep.
- **VII. Composable crates** ✅ — no Rust-side changes; only packaging.
- **VIII. Tested at every layer** ✅ — extended: CI now also tests the build matrix per PR (catches platform-specific compile breakage that would otherwise only surface during release).
- **IX. Documented behavior** ✅ — RELEASING.md + extended README cover the new install + release flows.
- **X. Performance budgets** — N/A; packaging.

### Open items (deferred, not blocking)

- **Yanking strategy** — npm allows un-publishing within 72h, and `npm deprecate` thereafter. RELEASING.md documents both flows. No automated yank tooling for v0.
- **Pre-release channel** — `next` or `rc` dist-tag. Skip for v0; can add when needed.
- **Bundle size monitoring** — track `.node` file size per platform over time. Skip for v0; revisit if any sub-project pushes the binary over 50 MB.

## Next steps after this spec is approved

1. Run writing-plans skill to produce `docs/superpowers/plans/2026-05-20-distribution-6.md`
2. Execute via subagent-driven-development
3. PR, merge, version bump to 0.14.0, tag → **first automated npm publish**
4. Verify the first publish succeeded: `npm view singularmem version` returns `0.14.0`; the 5 platform sub-packages exist at `npm view @bromso/singularmem-<triple> version`
5. Verify provenance: `npm audit signatures singularmem` reports verified provenance from the GitHub workflow run
6. After the workflow proves reliable across 2-3 releases, retire the dry-run safeguard (just keep tag-push and remove the workflow_dispatch path)
