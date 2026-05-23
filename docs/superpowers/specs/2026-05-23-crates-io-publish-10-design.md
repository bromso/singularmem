---
title: crates.io publish (Sub-project 10)
date: 2026-05-23
status: draft
sub-project: 10-crates-io-publish
supersedes: none
---

# crates.io publish (Sub-project 10) — Design Spec

**Date:** 2026-05-23
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 10 (crates.io publish — `cargo install singularmem` + library crate distribution)
**Builds on:** 7 (cargo-dist adoption, `v0.15.0`) and 8 (release-infra hygiene, `v0.16.0`).
**Sibling:** 9 (Windows MSVC binary fix, paused — independent of this work).

## Summary

Enables `cargo install singularmem` for Rust developers and makes all 7 library crates installable as dependencies via `cargo add singularmem-core` (etc.). The v0.17.0 release tag fires both the existing cargo-dist binary pipeline AND a new automated `cargo publish` pass that publishes 8 crates to crates.io in dependency order. Single PR; surface is `dist-workspace.toml` + `crates/singularmem-node/Cargo.toml` + the regenerated `.github/workflows/release.yml`.

Maintainer-only one-time setup: confirm crates.io account, generate API token, add `CRATES_IO_TOKEN` repo secret, and bootstrap-publish v0.16.0 of each crate manually to claim namespaces before the first automated CI publish.

## Problem & motivation

Singularmem ships as binaries (cargo-dist, Homebrew, curl-bash installer) and as a Node SDK (npm package), but it does NOT ship as Rust source on crates.io. Two consequences:

1. **Rust app developers can't depend on it.** Embedding singularmem's library API in a Rust app today requires `singularmem-core = { git = "https://github.com/bromso/singularmem", tag = "v0.16.0" }` in their Cargo.toml — non-standard, no caching benefit, requires git access at build time. After this sub-project, `singularmem-core = "0.17"` just works.
2. **`cargo install singularmem` doesn't work.** The most common Rust-developer install path is unavailable; users have to know about Homebrew/curl-bash/binary downloads. After this sub-project, Rust developers get the install path they expect.

crates.io is also the bridge to docs.rs — once a crate is published, docs build automatically and become browsable at `https://docs.rs/singularmem-core/latest`. No separate docs hosting needed.

Adding crates.io to the release matrix is cheap because cargo-dist 0.31.0 has built-in support: `"cargo"` in `publish-jobs` triggers a `cargo publish` pass in the right dep order on every tag push. Same automation pattern as the existing Homebrew tap publish.

## Goals & non-goals

### Goals

1. All 7 library crates + the root binary crate (8 total) publish to crates.io on the v0.17.0 tag push.
2. `cargo install singularmem` on a clean Rust toolchain installs both `singularmem` (CLI) and `singularmem-mcp` (MCP server) binaries, identical to the cargo-dist Homebrew/curl installs.
3. `cargo add singularmem-core` (or any of the 7 library crates) just works in downstream Rust projects.
4. Future releases (v0.18.0+) automatically publish to crates.io with zero maintainer intervention — same fire-and-forget as the Homebrew tap.
5. docs.rs builds succeed for each crate (verified at `https://docs.rs/<crate>/latest` after first publish).

### Non-goals

- **Publishing `singularmem-node` to crates.io.** It's a napi-rs cdylib for npm consumption; `cargo install` doesn't make sense for it. Explicitly excluded via `publish = false` in its Cargo.toml.
- **Backfilling old versions to crates.io.** Versions v0.1.0 through v0.16.0 are NOT republished. crates.io history starts at v0.17.0 (with v0.16.0 bootstrap commits if the manual claim path is used — see below).
- **Resolving the Windows MSVC binary fix from sub-project 9.** Out of scope; sub-project 9 is paused independently. `cargo install` works on linux/macOS regardless of Windows binary status.
- **Backfilling npm publishes of v0.15.0 + v0.16.0.** TOTP-blocked, user-only operational task. Unrelated.
- **Library code changes.** Nothing in `crates/*/src/**` should change. The crate metadata (description, license, repository, version) is already in place from sub-project 7.
- **Adding `homepage`, `documentation`, `keywords`, `categories` metadata.** Nice-to-have polish for crates.io listings, defer to a future docs-polish sub-project.
- **Setting up custom docs.rs configuration.** docs.rs auto-builds the default features; no custom build settings needed for v0.

## Recommended approach

Single PR titled `chore(ci): sub-project 10 — crates.io publish`. Five sequenced steps:

1. **Add `publish = false`** to `crates/singularmem-node/Cargo.toml`. Defensive — ensures any future `cargo publish` invocation doesn't accidentally push the napi-rs cdylib.

2. **Verify path-dependencies have version specs.** Spot-check each crate's `Cargo.toml` for `path = "..."` dependencies; ensure they all carry `version = "..."` alongside (required by `cargo publish`). Workspace-level deps in root `Cargo.toml` should already have this from sub-project 7's cargo-dist setup.

3. **Update `dist-workspace.toml`** to add `"cargo"` to `publish-jobs`:

   ```toml
   publish-jobs = ["homebrew", "cargo"]
   ```

4. **Run `dist generate --mode ci`** to regenerate `release.yml`. The diff adds a new `publish-crates` job (or similar name — cargo-dist's exact emitted shape may vary) that runs `cargo publish` for each library crate in dep order on tag push. The job uses `CRATES_IO_TOKEN` as an env secret.

5. **Open PR.** PR build matrix (`pr-run-mode = "upload"`) runs against the new workflow shape. The publish step is tag-gated, so the PR run won't actually publish to crates.io — verify it's listed as "skipped" not "failed". After merge + version bump 0.16.0 → 0.17.0 + tag `v0.17.0`, the publish job fires.

### One-time maintainer setup (outside the PR)

A. **Generate crates.io API token.** At https://crates.io/settings/tokens, create a token with scopes `publish-new` + `publish-update` (needed for first-time namespace claim, can rotate to `publish-update` only after).

B. **Add `CRATES_IO_TOKEN` repo secret.** At https://github.com/bromso/singularmem/settings/secrets/actions.

C. **Bootstrap-publish v0.16.0 of each crate manually.** Strongly recommended to avoid partial-failure surprises on the first CI publish run:

   ```bash
   cd /Users/jonasbroms/Sites/singularmem
   cargo login <token>
   # Order matters: deepest deps first.
   for crate_dir in crates/singularmem-core crates/singularmem-search \
                    crates/singularmem-retrieve \
                    crates/singularmem-adapter-claude \
                    crates/singularmem-adapter-gemini \
                    crates/singularmem-adapter-openai \
                    crates/singularmem-mcp; do
     (cd "$crate_dir" && cargo publish --dry-run && cargo publish)
     sleep 30  # respect crates.io rate limits + index propagation
   done
   # Root binary last:
   cargo publish --dry-run && cargo publish
   ```

   This bootstraps v0.16.0 of each crate to crates.io and reserves the namespace. After this, the CI publish job's job is "publish updates" — much less likely to fail mid-publish.

D. **Verify on docs.rs.** Each crate gets a docs.rs build automatically after first publish. Spot-check that docs build successfully (e.g., `https://docs.rs/singularmem-core/latest`).

### Approaches discarded

- **Approach B — First publish manual, subsequent CI-automated.** First publish is the bootstrap step (C above), subsequent publishes flow through CI. This is what the recommended approach does — they're not really alternatives. The "first publish manual" framing in brainstorming was about WHETHER to set up CI at all; the user picked "yes, set up CI" plus "yes, also do manual bootstrap to de-risk first CI run".
- **Approach C — Fully manual via scripted bash.** Write a `scripts/publish-crates.sh` instead of using cargo-dist's publish-job. Rejected because:
  - Loses cargo-dist's dep-graph-aware ordering (would have to hand-maintain in the script).
  - Adds operational burden every release (vs. fire-and-forget after CI is wired).
  - The current "manual npm publish" workaround is already painful enough — duplicating that pattern for crates.io is the wrong direction.

## Architecture

No new components, no new library boundaries. Affected surfaces:

- **`crates/singularmem-node/Cargo.toml`** — add `publish = false` to `[package]`. ~1 line.
- **`dist-workspace.toml`** — change `publish-jobs = ["homebrew"]` to `publish-jobs = ["homebrew", "cargo"]`. 1-line edit.
- **`.github/workflows/release.yml`** — regenerated by `dist generate --mode ci`. Adds a publish-crates job that runs after `host` succeeds and before `announce`.
- **All 7 library crate `Cargo.toml` files** — read-only check (no edits expected); confirm path-deps have version specs. If any don't, fix them.

No code changes. No new tests. No new dependencies. No on-disk format changes.

## Data model

None. crates.io publish doesn't touch on-disk format, schema, or sidecar layout. The published .crate tarballs are source-only — no compiled artifacts shipped via crates.io (Rust convention).

## Interfaces

No CLI, library, or wire-protocol changes. The published library crate APIs are exactly what's already in `crates/*/src/lib.rs`; no API surface adjustments.

New install/dependency paths become available to Rust users:

```bash
# Install the CLI:
cargo install singularmem
# (alternatively: cargo install singularmem-mcp for just the MCP server)

# Use as a library:
cargo add singularmem-core
# (and any of: singularmem-search, singularmem-retrieve,
#  singularmem-adapter-claude, singularmem-adapter-gemini,
#  singularmem-adapter-openai, singularmem-mcp)
```

Browseable docs at `https://docs.rs/singularmem-core/latest` (etc.) — auto-built by docs.rs on first publish.

## Error handling

Operational, not code:

- **First CI publish fails mid-publish.** Mitigated by manual bootstrap (Step C above). If despite bootstrap, the CI publish job fails on, say, crate 4 of 8: cargo publish is idempotent on already-published versions (re-running the job will see crates 1-3 as "already at this version, skip" or "error: already exists" — cargo-dist's publish-jobs treat this as success). Manual recovery: identify failing crate, fix root cause, re-tag or re-run the job.
- **Path-dependency missing version spec.** Detection: `cargo publish --dry-run` for the offending crate fails with `the dependency `<name>` does not specify a version`. Recovery: add `version = "x.y.z"` next to `path = "..."` in the consumer crate's Cargo.toml; commit; re-tag.
- **`singularmem-node` published by accident.** Prevented at multiple layers: (1) `publish = false` in its Cargo.toml (cargo refuses to publish), (2) cargo-dist's `cargo` publish-job reads workspace metadata and respects `publish = false`. Verified by spot-checking the dist plan output.
- **CRATES_IO_TOKEN expired or invalid.** Detection: publish-crates job fails with auth error. Recovery: rotate token at crates.io settings; update repo secret; re-run the job (cargo-dist publish-jobs are idempotent on already-published versions).
- **Crate name collision** (shouldn't happen — all 9 names confirmed available 2026-05-21). Mitigation: confirmed pre-flight; no recovery needed.

Per Principle VII, all failure modes preserve state: no destructive operations (no force-push to main, no tag overwrites, no `cargo yank` without explicit instruction).

## Testing strategy

No new unit or integration tests. Verification is operational:

**On the PR (before merge):**
1. CI green: `ci.yml`, `dco.yml`, `npm-publish.yml`, `release.yml` (in upload mode with new publish-crates job listed as SKIPPED, not failed).
2. `singularmem-node/Cargo.toml` has `publish = false` — verified by `grep`.
3. `dist plan` shows the publish-crates step in the plan output (run locally if needed).

**Manual bootstrap (one-time, between PR merge and v0.17.0 tag):**
4. All 8 library/binary crates published at v0.16.0 successfully via local `cargo publish`. Verified by `cargo search <name>` returning the published version.
5. docs.rs builds succeed for each crate (spot-check at `https://docs.rs/<crate>/latest`).

**On main after merge + version bump + v0.17.0 tag:**
6. Full release.yml DAG green: existing 8 jobs + new publish-crates job — all `conclusion=success`.
7. All 8 crates at v0.17.0 on crates.io. Verified by `curl https://crates.io/api/v1/crates/<name> | jq .crate.max_version`.
8. `cargo install --locked singularmem` on a clean Rust toolchain installs both binaries; `singularmem --version` returns `0.17.0`.
9. docs.rs builds succeed for v0.17.0 of each crate.

Per Principle VI: existing offline-testable suite unchanged; not affected by this sub-project's surface.
Per Principle III.b: no closed-source dependency introduced — sub-project surface is `Cargo.toml` files + `dist-workspace.toml` + `.github/workflows/release.yml`.

## Open questions

1. **Maintainer's crates.io username.** Likely `jonasbroms` (matching the npm account), but not yet confirmed. Implementation step C should verify by `cargo login --help` / inspecting `~/.cargo/credentials.toml` after login.
2. **Exact cargo-dist publish-job naming.** dist 0.31.0's emitted job for `"cargo"` publish-job — confirmed name + job structure pending `dist generate` output. Implementation Step 4 documents the actual emitted shape in the PR description.
3. **Should `singularmem-mcp` be installed via `cargo install singularmem` (transitively) OR is a separate `cargo install singularmem-mcp` needed?** Sub-project 7's cargo-dist setup bundled both binaries into the same archive. crates.io behavior may differ — the root `singularmem` crate's `[[bin]]` may or may not include `singularmem-mcp`. Implementation Step 5 verifies by running `cargo install --locked singularmem` on a clean test environment AFTER first publish and checking whether `singularmem-mcp` shows up on `$PATH`.

## Acceptance criteria

Numbered, observable, testable. The sub-project is done when all items are observable on `main` and at the tagged release:

1. **`dist-workspace.toml` has `cargo` in publish-jobs.** `grep -E '^publish-jobs.*cargo' dist-workspace.toml` returns a match.
2. **`singularmem-node/Cargo.toml` has `publish = false`.** `grep -q '^publish = false' crates/singularmem-node/Cargo.toml` exits 0.
3. **PR CI green; publish-crates job is SKIPPED (not failed) on PR run.** `gh pr checks <pr> | grep publish-crates` shows `skipping`.
4. **(maintainer) All 8 crates exist on crates.io at v0.16.0+** before tagging v0.17.0. Verified by `cargo search <name>` returning each.
5. **v0.17.0 release pipeline all jobs succeed including publish-crates.** `gh api /repos/bromso/singularmem/actions/runs/<run-id>/jobs --jq '.jobs[] | {name, conclusion}'` shows publish-crates `success`.
6. **All 8 crates at v0.17.0 on crates.io.** Verified by API call returning `max_version: "0.17.0"` for each.
7. **`cargo install --locked singularmem` works on a clean Rust toolchain.** Output of `singularmem --version` is `0.17.0`. (Verifiable from a temporary cargo install path on the maintainer's Mac.)
8. **docs.rs builds succeed for the foundation crates.** Manual spot-check at `https://docs.rs/singularmem-core/latest` — page renders without "build failed" banner. Verified for at least 3 of 8 crates.
9. **`CRATES_IO_TOKEN` repo secret exists.** Verified indirectly by AC 5 succeeding (publish job needs the token).

ACs 1-3 and 5-9 are agent-verifiable (with the maintainer having set up the token + bootstrap). AC 4 requires maintainer-driven manual bootstrap.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls in library code; published crates are source-only (cargo compiles locally per Principle I). |
| **II — Provider-Agnostic by Contract** | The 4 constitutional providers (plain inside retrieve, claude, openai, gemini) ship as separate crates that can be cherry-picked by downstream users. Reinforces provider-agnosticism — Rust users can depend on just `singularmem-core` + their chosen adapter. |
| **III — Open Core with a Stable Boundary** | Sub-project surface is `Cargo.toml` + `dist-workspace.toml` + `release.yml` — all open-side. No closed-source coupling. Library crates published to crates.io are exactly the open-core surface; nothing closed leaks in. |
| **V — Composable Library Architecture** | Reinforced — Rust users can compose by adding only the crates they need. |
| **VI — Deterministic and Offline-Testable** | No test changes. After `cargo install`, the offline-testable suite still passes with networking disabled. |
| **X — Performance Budgets, Enforced in CI** | No performance-touching code changes. crates.io publish doesn't affect the perf-budget profile (which uses `release`). |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy Telemetry), and IX (Accessible by Default) are not touched by this sub-project's surface.
