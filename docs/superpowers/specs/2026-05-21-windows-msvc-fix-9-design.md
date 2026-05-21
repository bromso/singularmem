---
title: Windows MSVC binary fix (Sub-project 9)
date: 2026-05-21
status: draft
sub-project: 9-windows-msvc-fix
supersedes: none
---

# Windows MSVC binary fix (Sub-project 9) — Design Spec

**Date:** 2026-05-21
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 9 (Windows MSVC binary fix)
**Builds on:** 7 (cargo-dist adoption, `v0.15.0`) and 8 (release infra hygiene, `v0.16.0`). Closes the Windows platform gap deferred from sub-project 7.

## Summary

Re-enables the `x86_64-pc-windows-msvc` target in cargo-dist's matrix so `v0.17.0`'s release pipeline produces a Windows archive alongside the 3 existing platforms (linux x64, darwin x64, darwin arm64). The fix is a single line in a new `.cargo/config.toml`: setting `target-feature=-crt-static` for the Windows MSVC target so the `cxx` crate uses `/MD` (dynamic CRT) linkage, matching ort-sys's pre-built onnxruntime binaries.

No library code changes. Surface is `.cargo/config.toml` (new file), `dist-workspace.toml`, `.github/workflows/release.yml` (regenerated), and README/RELEASING.md (restore Windows sections that sub-project 8 removed).

## Problem & motivation

Sub-project 7 attempted a 4-platform cargo-dist release (linux x64, darwin x64, darwin arm64, windows x64-msvc) at `v0.15.0`. The Windows build failed with a CRT runtime-library mismatch:

```
error LNK2038: mismatch detected for 'RuntimeLibrary':
  value 'MT_StaticRelease' doesn't match value 'MD_DynamicRelease' in
  libort_sys-...(onnxruntime_c_api.obj)
```

The conflict is between the `cxx` crate (compiled with `/MT` static CRT) and `ort-sys` (compiled with `/MD` dynamic CRT, because Microsoft's pre-built onnxruntime binaries Microsoft ships use `/MD`). MSVC's linker refuses to mix `/MT` and `/MD` objects in the same binary.

Sub-project 7 dropped Windows from the targets list to ship v0.15.0; sub-project 8 left the gap as-is and ran a 3-platform release at v0.16.0. The Windows user-install path has been documented as "build from source" since then.

Investigation in this session shows:
- `ort-sys` cannot be made to use `/MT` — the pre-built onnxruntime binaries are fixed.
- The `cxx` crate, when compiled normally via `cc` on `*-pc-windows-msvc`, should default to `/MD`. Something is forcing `/MT`. The most likely cause: cargo's default `target-feature=+crt-static` setting for `*-pc-windows-msvc` (Rust's MSVC target enables static CRT by default since Rust 1.71).
- Setting `target-feature=-crt-static` explicitly via `.cargo/config.toml` forces dynamic CRT for the Rust portion AND the cc-built C++ portions (cxx, etc.).

Approach is `.cargo/config.toml`-only — minimal surface, easily reversible, no dependency changes.

## Goals & non-goals

### Goals

1. `x86_64-pc-windows-msvc` is back in `dist-workspace.toml` `targets`, AND the PR's build matrix builds Windows successfully (no link errors).
2. `v0.17.0` release pipeline produces a Windows archive (`singularmem-x86_64-pc-windows-msvc.zip` + sha256) and a Windows-targeted PowerShell installer (`singularmem-installer.ps1`).
3. `singularmem.exe` and `singularmem-mcp.exe` run end-to-end on a Windows-latest GH Actions runner via a new smoke-test workflow: at minimum, `singularmem --version` returns `0.17.0` without runtime crashes, and an `init → ingest → retrieve` cycle produces a non-empty result.
4. README's "Installing the CLI" section restores Windows (platform table row + PowerShell installer code block) and removes the temporary "Windows MSVC prebuilt binaries are temporarily unavailable" disclaimer.
5. A new `.github/workflows/smoke-test-windows.yml` workflow exists, triggers via `workflow_dispatch`, runs on `windows-latest`, downloads the latest (or specified) release archive, and asserts the binaries start + run a basic CLI cycle. This is the *automated* verification path that replaces "user manual verification on a Windows machine" — the maintainer doesn't need Windows hardware to validate releases.

### Non-goals

- **Linux arm64 binary.** Separate blocker (fastembed → ureq → openssl-sys cross-compile). Future sub-project.
- **npm CI publish 404 (sub-project 6b candidate).** Investigation-heavy; pending.
- **Backfill npm publishes of v0.15.0 / v0.16.0 / v0.17.0.** User-only operational task (TOTP-gated). Out of PR scope.
- **Switching the embedder.** Not removing fastembed; not introducing a pure-Rust alternative; not bumping fastembed/ort versions.
- **Adding Windows arm64** (`aarch64-pc-windows-msvc`). Out of scope; Windows arm64 has its own toolchain complications.
- **Library code changes.** Nothing in `crates/*/src/**` should change.
- **Performance budget revisions.** Build-system fix, no perf impact.
- **Re-introducing `lto = "thin"` to `[profile.dist]`.** Stays off (per sub-project 8). Was originally removed during the v0.15.0 saga as a Windows-fix hypothesis that turned out not to matter — but no reason to re-add now.

## Recommended approach

Single PR titled `chore(ci): sub-project 9 — Windows MSVC binary fix`. Five sequenced steps:

1. **Create `.cargo/config.toml`** with the Windows-MSVC-targeted rustflags:

   ```toml
   [target.x86_64-pc-windows-msvc]
   rustflags = ["-C", "target-feature=-crt-static"]
   ```

   File didn't exist before this PR (only `.cargo/audit.toml` is there). New file, ~3 lines plus an explanatory comment.

2. **Re-add Windows MSVC to `dist-workspace.toml` targets** and restore `"powershell"` in installers:

   ```toml
   installers = ["shell", "powershell", "homebrew"]
   targets = [
     "x86_64-unknown-linux-gnu",
     "x86_64-apple-darwin",
     "aarch64-apple-darwin",
     "x86_64-pc-windows-msvc",
   ]
   ```

   `"powershell"` was retained in sub-project 8's installers list as no-op (warning-emitting); this re-activates it because Windows is back in targets.

3. **Run `dist generate --mode ci`** to regenerate `release.yml` with Windows back in the matrix. Diff should show: new `build-local-artifacts (x86_64-pc-windows-msvc)` matrix entry, plus PowerShell installer generation in the global-artifacts job.

4. **Restore README sections** that sub-project 8 removed:
   - Platform table row: `| Windows | x86_64 (MSVC) |`
   - PowerShell installer code block: `powershell -ExecutionPolicy ByPass -c "irm https://github.com/bromso/singularmem/releases/latest/download/singularmem-installer.ps1 | iex"`
   - Remove the "Windows MSVC prebuilt binaries are temporarily unavailable due to a CRT runtime-library mismatch in the ort-sys / cxx dependency chain (ONNX Runtime FFI via fastembed); tracked as a follow-up sub-project." sentence.

5. **Open the PR.** PR build matrix runs the full 4-platform build (now including Windows) via `pr-run-mode = "upload"`. If the Windows build succeeds, the fix works. Merge, bump workspace version `0.16.0 → 0.17.0` on main as a follow-on commit per existing convention, tag `v0.17.0`, watch the release. No RC dry-run (sub-project 8 documented that dist refuses to release if workspace version doesn't match tag; RC requires extra version-bump-and-revert dance that isn't worth it for a one-platform-fix).

### Approaches discarded

- **Approach B — Switch ort to `load-dynamic` feature.** Onnxruntime.dll loads at runtime instead of static-linking. Bypasses the CRT issue entirely but complicates the Windows user install story (user must install onnxruntime separately on their machine; `singularmem.exe` doesn't ship onnxruntime.dll). Bigger surface: fastembed feature flag changes + runtime DLL probing in core. Rejected because Approach A is smaller and reversible.
- **Approach C — Switch embedder to a pure-Rust alternative** (candle-transformers, burn). Eliminates the C++ FFI dependency entirely. Largest scope: touches `singularmem-search/embedder.rs`, all tests, possibly perf budgets. May change embedding model semantics. Rejected as over-scope for a platform fix.

## Constraints

| Constraint | Notes |
|---|---|
| `.cargo/config.toml` is committed to the repo | Project-wide build setting; all contributors get the same Windows linkage. Not a per-user override. |
| Other targets unaffected | The `[target.x86_64-pc-windows-msvc]` table only applies when building for that triple. Linux/Darwin builds ignore it. |
| `cxx` crate's `/MD` linkage must be consistent across all callsites in the workspace | If any other dep forces `/MT` (e.g., via its own build.rs), the link error returns. Mitigation: the PR's build matrix on Windows is the canary. |
| `dist generate --mode ci` is the canonical regeneration path | Set up in sub-project 8. No manual edits to `release.yml`. `allow-dirty` stays absent in dist-workspace.toml. |
| Sub-project 8's residual config items must survive regeneration | `pr-run-mode = "upload"`, `install-path = "CARGO_HOME"`, `tap = "bromso/homebrew-tap"`, `publish-jobs = ["homebrew"]`, `[profile.dist]` without `lto = "thin"`. All survive trivially since we're only adding to targets/installers, not touching them. |

## Architecture

No new components, no new library boundaries. Affected surfaces:

- **NEW: `.cargo/config.toml`** — 3 lines + comment. Pure config.
- **`dist-workspace.toml`** — add `x86_64-pc-windows-msvc` back to `targets`, add `powershell` back to `installers`.
- **`.github/workflows/release.yml`** — regenerated by `dist generate --mode ci`. Adds Windows matrix entry + PowerShell installer step.
- **`README.md`** — restore Windows platform row + PowerShell installer code block; remove the temporary "unavailable" disclaimer.

No changes to:
- Any crate's source code
- `Cargo.toml` (root or sub-crates)
- `Cargo.lock` (no dependency changes)
- `.github/workflows/{ci,dco,npm-publish}.yml`
- Tests

## Data model

None. No on-disk format changes, no schema migrations.

## Interfaces

No CLI, library, or wire-protocol changes. `singularmem --version` advances `0.16.0 → 0.17.0` via the follow-on version-bump commit; the binary surface is otherwise identical.

The PowerShell installer is back in the install paths:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/bromso/singularmem/releases/latest/download/singularmem-installer.ps1 | iex"
```

## Error handling

Operational, not code:

- **`dist generate` produces unexpected diffs.** Reconcile per sub-project 8's pattern. Document residuals in PR description.
- **Windows build STILL fails after the rustflags fix.** Detection: PR's `build-local-artifacts (x86_64-pc-windows-msvc)` job goes red. Recovery: pause sub-project 9 implementation; inspect the new failure (could be a different CRT path, a new ort/cxx version mismatch, etc.); re-evaluate approach (fall back to approach B — ort `load-dynamic`).
- **Windows build succeeds in CI but `singularmem.exe` crashes at runtime.** Detection: manual smoke-test on Windows after v0.17.0 release. Recovery: yank the v0.17.0 release (mark as prerelease, push a v0.17.1 fix); investigate. Mitigation: the CRT linkage doesn't affect program logic, only how runtime services (allocator, mutex, file I/O) are reached. Crash here would be very rare.
- **README/RELEASING.md edits accidentally remove other sub-project 8 content.** Detection: PR diff review. Recovery: amend the README edit; re-push.

Per Principle VII, all failure modes preserve state — no destructive operations (no force-push to main, no tag overwrites of v0.16.0 or earlier).

## Testing strategy

No new unit or integration tests; build-system fix only. Verification is operational:

**On the PR (before merge):**
1. PR's `release.yml` runs in `upload` mode — 4-platform build matrix completes successfully, including the new `build-local-artifacts (x86_64-pc-windows-msvc)` job.
2. Inspect the Windows archive uploaded by the PR run: `singularmem-x86_64-pc-windows-msvc.zip` should contain `singularmem.exe`, `singularmem-mcp.exe`, README, LICENSE.
3. CI workflow stays green (`ci.yml`, `dco.yml`, `npm-publish.yml`). Sub-project 8 already validated these don't regress.

**On main after merge + version bump + `v0.17.0` tag:**
1. Full release.yml DAG green: `plan → build × 4 → build-global → host → publish-homebrew-formula → announce` — 8 jobs total (1 more than v0.16.0's 7 because of the extra build platform).
2. GH Release `v0.17.0` published with **26 assets** (v0.16.0's 20 + 6 new for Windows: 2 platform archives × 2 binaries × 2 files = 4, plus 2 PowerShell installers).
3. Homebrew tap updated to `0.17.0` for both formulas.
4. PowerShell installer + `singularmem --version` produces `0.17.0` on a clean Windows machine (manual user verification).
5. At least one CLI operation end-to-end on Windows: e.g. `singularmem init` followed by `singularmem ingest --content "hello"` followed by `singularmem retrieve "hello"`. Validates that the binary runs, not just exits.

Per Principle VI: existing offline-testable suite is unchanged — not affected by this sub-project's surface.
Per Principle III.b: no closed-source dependency introduced — sub-project is `.cargo/config.toml` + `dist-workspace.toml` + `.github/workflows/release.yml` + `README.md` only.

## Open questions

1. **Will `-crt-static` actually fix the cxx mismatch in our specific crate graph?** Hypothesis is strong but not proven until the PR's Windows build runs green. If it doesn't, fall back to Approach B (ort `load-dynamic`). Implementation must keep the PR open until Windows build is verified green before merging.
2. **Does cargo-dist 0.31.0 emit any additional Windows-specific config in `release.yml` when Windows is in targets that we should be aware of?** Probably not — sub-project 7 already had Windows in targets and the workflow shape was identical to non-Windows. Implementation should diff the regenerated workflow to confirm.
3. **Should `.cargo/config.toml` include comments explaining WHY the rustflag is set?** Yes — without context, a future maintainer might remove the flag thinking it's leftover. Implementation should include a comment block referencing this spec.

## Acceptance criteria

Numbered, observable, testable. The sub-project is done when all items are observable on `main` and at the tagged release:

1. **PR's Windows build job succeeds.** `gh run view <pr-run-id> --json jobs --jq '.jobs[] | select(.name | contains("windows-msvc")) | .conclusion'` returns `"success"`.
2. **`.cargo/config.toml` exists and contains the windows-msvc rustflags.** `grep -q 'target-feature=-crt-static' .cargo/config.toml` exits 0.
3. **`dist-workspace.toml` has Windows in targets.** `grep -q 'x86_64-pc-windows-msvc' dist-workspace.toml` exits 0.
4. **`v0.17.0` release pipeline all 8 jobs succeed.** `plan`, `build × 4`, `build-global`, `host`, `publish-homebrew-formula`, `announce` — all `conclusion=success`.
5. **GH Release `v0.17.0` exists with 26 assets.** `gh release view v0.17.0 --json assets --jq '.assets | length'` returns `26`.
6. **GH Release assets include Windows archives + PowerShell installer.** `gh release view v0.17.0 --json assets --jq '.assets[].name' | grep -E '(windows-msvc|installer.ps1)'` returns at least 4 entries: `singularmem-x86_64-pc-windows-msvc.zip` + its sha256, `singularmem-mcp-x86_64-pc-windows-msvc.zip` + its sha256, plus at least one `*-installer.ps1`. Exact count depends on whether dist 0.31.0 emits one or two PowerShell installer scripts.
7. **Homebrew tap updated to 0.17.0.** Both formulas show `version "0.17.0"`.
8. **`singularmem --version` on Windows returns `0.17.0`.** Verified by `smoke-test-windows.yml` workflow run on `windows-latest`. Maintainer triggers the workflow via `workflow_dispatch` after v0.17.0 release; job exits 0.
9. **`singularmem-mcp --version` on Windows returns `0.17.0`.** Same channel as criterion 8 — same workflow asserts both binaries.
10. **End-to-end smoke test on Windows.** Same workflow: `singularmem init <tempdir>` then `singularmem ingest --content "test"` then `singularmem retrieve "test"` produces a non-empty result. Workflow asserts the retrieve output contains the test content.
11. **`smoke-test-windows.yml` workflow exists and is dispatched at least once against v0.17.0 with all jobs green.** Verified via `gh run list --workflow=smoke-test-windows.yml`.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls in library code. The Windows binary continues to embed fastembed's downloaded ONNX model the same way the macOS/Linux binaries do. |
| **II — Provider-Agnostic by Contract** | No adapter changes. All four constitutional providers (plain, claude, openai, gemini) continue to ship in the Windows binary. |
| **III — Open Core with a Stable Boundary** | Sub-project surface is build config + workflow YAML + README — all open-side. No closed-source coupling introduced. |
| **V — Composable Library Architecture** | No library API changes. |
| **VI — Deterministic and Offline-Testable** | No test changes; existing offline-testable suite is unchanged. |
| **X — Performance Budgets, Enforced in CI** | No performance-touching code changes. CRT linkage may marginally change Windows-binary startup time but Windows isn't on the perf-budget enforcement matrix (Linux x64 is). |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy Telemetry), and IX (Accessible by Default) are not touched by this sub-project's surface.
