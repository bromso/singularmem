---
title: Singularmem — Bootstrap & Constitution Finalize
date: 2026-05-15
status: draft
sub-project: 0-bootstrap
supersedes: none
---

# Singularmem — Bootstrap & Constitution Finalize

This is sub-project **0** of Singularmem. Its purpose is to resolve every
unspecified placeholder in the project constitution and to stand up the repo
skeleton that every later sub-project (Memory Store v0, Search v0, provider
adapters, MCP server, SDK bindings, distribution, proprietary GUI) will hang
off of.

This spec ships no domain code. It ships **governance artefacts** and a
do-nothing CLI binary that exists only to prove the build pipeline is real.

## Problem & motivation

The Singularmem constitution (v0.2.0, drafted 2026-05-15) names a working
title, a license slot, a reference-hardware slot, four performance-budget
slots, and a non-Rust-binding choice — all as `[PLACEHOLDER]` strings to be
resolved "before ratification". Until they are resolved, no sub-project can
proceed:

- A spec for the on-disk format cannot reference license terms that don't
  exist.
- A plan's Constitution Check (Principle X) cannot be enforced without
  reference hardware and budget numbers.
- A contributor cannot sign off without a CONTRIBUTING file describing how.

Bootstrap exists to remove these blockers in one short, low-risk PR. It is
sequenced before all other sub-projects in the decomposition (see
[Decomposition](#decomposition)).

## Goals & non-goals

### Goals

1. Resolve every `[PLACEHOLDER]` in the constitution to a concrete value.
2. Commit the constitution to `.specify/memory/constitution.md` (the path
   the constitution's own amendment procedure names).
3. Commit the three templates the constitution's governance section
   requires: `spec-template.md`, `plan-template.md`, `tasks-template.md`.
4. Stand up the Cargo workspace skeleton with **zero crates yet** and a
   single root-level do-nothing binary at `src/main.rs`.
5. Commit the standard top-level governance files (README, CONTRIBUTING
   with DCO, SECURITY, CODE_OF_CONDUCT, LICENSE, NOTICE) plus the
   formatter/linter/editor configuration files.
6. Stand up CI on GitHub Actions running fmt, clippy, check, build, test,
   and audit on the reference runner.
7. Register `security@singularmem.dev` so the SECURITY contact is real.

### Non-goals

- Any non-root crate.
- Any library, ingest, index, embedding, or provider code.
- Performance-budget enforcement in CI (deferred to sub-project 1 — there
  is no code to measure here).
- Release pipeline, signing, packaging (deferred to a later sub-project on
  distribution).
- The Flutter GUI or any proprietary code (deferred to a much later
  sub-project; the proprietary side has its own commercial license to be
  drafted closer to v1.0).
- The TypeScript SDK binding (deferred to its own sub-project).
- Drafting the actual proprietary commercial license document.

## Decomposition (context for why this sub-project exists)

The constitution describes a complete product with many independent
subsystems. Trying to spec all of it in one document would produce either a
100-page novel or a useless skeleton. The work is decomposed into
sub-projects, each of which gets its own spec → plan → implementation cycle:

| # | Sub-project | Notes |
|---|---|---|
| 0 | **Bootstrap & constitution finalize** | This spec. Governance only. |
| 1 | Memory Store v0 | Document model, on-disk format, ingest pipeline, ID/revision system, minimal CLI. First crate. |
| 2 | Search v0 | Tantivy lexical index, vector-index choice + integration, local ONNX embedding host. |
| 3 | Provider adapters + retrieval | Typed `LlmProvider` trait + Claude/OpenAI/Gemini/local adapters. |
| 4 | MCP server | Wraps the library for any MCP client. |
| 5 | TypeScript SDK binding | First non-Rust binding (via napi-rs). |
| 6 | Distribution & packaging | Signed binaries, homebrew, cargo, release pipeline. |
| 7+ | Proprietary Flutter GUI + visualisations + sync | Much later. Separate commercial license. |

This sub-project is sequenced before all of the above because each of them
depends on either (a) resolved license terms, (b) defined performance
budgets, (c) the templates required by the constitution's plan-gate, or
(d) the workspace shape that this sub-project establishes.

## Recommended approach

**Approach A — Minimum viable bootstrap.** Resolve every placeholder.
Commit the constitution and the three templates. Stand up a Cargo workspace
with zero crates yet and a root-level do-nothing binary that prints
`singularmem 0.0.0` and exits. Commit the standard top-level governance and
configuration files. CI runs fmt, clippy, check, build, test, and audit on
`ubuntu-latest`. A `macos-latest` advisory job is wired up but
non-blocking. This is what this spec adopts.

### Approaches discarded

- **Approach B — Bootstrap + skeleton crates.** Same as A plus stub crates
  (`crates/singularmem-core`, `crates/singularmem-cli`, etc.). Rejected:
  crate boundaries are a design decision that belongs in each downstream
  sub-project, not in bootstrap. Premature scaffolding ages badly.
- **Approach C — Bootstrap-only, no Cargo workspace.** Just governance
  artefacts. Rejected: the first PR for sub-project 1 would then have to
  introduce both Cargo machinery and the actual domain work in a single
  diff, mixing concerns and complicating review.

## Resolved constitution placeholders

| Placeholder | Resolved value | Rationale |
|---|---|---|
| Working title | **Singularmem** | Maintainer decision; the repo directory is already named this. |
| Ratification date | **2026-05-15** | This PR ratifies. |
| `[OPEN_LICENSE]` | **Apache-2.0** | Permissive with patent grant. The de facto standard for open-core; compatible with the proprietary tier linking the open library. Strong-copyleft licenses (GPL/AGPL) are structurally incompatible with the open-core model defined by Principle III, because the same maintainer authors both tiers. |
| `[COMMERCIAL_LICENSE]` | **"Proprietary, all rights reserved — full terms to be published with the first proprietary release."** | No proprietary code exists yet. Drafting an EULA before the GUI exists is premature. The placeholder is filled with the *commitment* to a commercial license, not the license itself. |
| `[REFERENCE_HARDWARE]` | **Primary: GitHub Actions `ubuntu-latest` (x86_64, 4 vCPU, 16 GB RAM).** Secondary, advisory: Apple Silicon M-series Mac, 16 GB RAM. | CI must be reproducible by any contributor; that means a hosted runner. M-series is the realistic dev machine and proprietary GUI target, so it is tracked but not blocking. |
| `[INDEX_QUERY_P95_MS]` | **100** | Generous default for Tantivy + vector lookup at v1 scale (~100K items). Allows slack pending vector-index choice; tighten once measured. |
| `[INGEST_THROUGHPUT_PER_S]` | **50** | Achievable with inline CPU ONNX embeddings (e.g. `all-MiniLM-L6-v2`) on the reference runner. Tighten once measured. |
| `[STARTUP_BUDGET_MS]` (CLI) | **200** | Comfortable for a Rust CLI that opens an embedded store and an index handle. GUI cold-start budget is deferred to the Flutter sub-project. |
| `[BINARY_SIZE_BUDGET_MB]` | **150** | Assumes the embedded ONNX runtime, Tantivy, the chosen vector index, and dependencies. Revisit if model weights are downloaded on first use rather than embedded. |
| Non-Rust binding | **TypeScript first** (via `napi-rs`). Python is tracked as a separate sub-project, not bootstrap. | TS is where LLM-agent tooling lives (Claude Code, Cursor, custom Node agents) — highest leverage. The constitution commits to "at least one" non-Rust binding; TS satisfies it. Python is a real second target but doubles maintenance for no v0 benefit. |

## Architecture

This sub-project ships no architecture in the domain sense. What it *does*
ship is the **layout convention** that every later sub-project will conform
to:

- The Cargo workspace is the unit of composition. New libraries land as
  new members of `crates/*`. Each crate is a Principle V "library first"
  unit with its own public API and tests.
- The root-level binary at `src/main.rs` is the user-facing CLI entry
  point. In bootstrap it prints version and exits. Sub-project 1 will
  thicken it into a thin shell over `crates/singularmem-core`.
- Governance lives in `.specify/`. The constitution is at
  `.specify/memory/constitution.md`. The three templates are at
  `.specify/templates/`. This mirrors the path the constitution's
  amendment procedure itself names.
- Design specs live in `docs/superpowers/specs/`, one file per
  sub-project, named `YYYY-MM-DD-<topic>-design.md`. Implementation plans
  live in `docs/superpowers/plans/`.

## Repo layout

```
singularmem/
├── .agents/                          # existing — left alone
├── .claude/                          # existing — left alone
├── .github/
│   └── workflows/
│       └── ci.yml                    # fmt + clippy + check + build + test + audit
├── .specify/
│   ├── memory/
│   │   └── constitution.md           # v0.2.0, placeholders resolved
│   ├── templates/
│   │   ├── spec-template.md
│   │   ├── plan-template.md
│   │   └── tasks-template.md
│   └── README.md                     # what this directory is for
├── crates/                           # empty for now; sub-project 1 adds the first
├── docs/
│   └── superpowers/
│       ├── specs/
│       │   ├── README.md             # convention notes
│       │   └── 2026-05-15-bootstrap-design.md  # this file
│       └── plans/
│           └── README.md
├── src/
│   └── main.rs                       # do-nothing `singularmem --version`
├── .editorconfig
├── .gitignore
├── CODE_OF_CONDUCT.md                # Contributor Covenant 2.1
├── CONTRIBUTING.md                   # DCO required; no CLA
├── Cargo.toml                        # workspace + root package
├── LICENSE                           # Apache-2.0 full text
├── NOTICE                            # Apache-2.0 attribution
├── README.md
├── SECURITY.md                       # security@singularmem.dev
├── clippy.toml
├── rust-toolchain.toml               # channel = stable; rustfmt + clippy
├── rustfmt.toml
└── skills-lock.json                  # existing — left alone
```

Three points worth flagging in the layout:

1. The Cargo workspace root is also a package. It owns the `singularmem`
   binary at `src/main.rs`. When sub-project 1 introduces
   `crates/singularmem-core`, the root binary becomes a thin shell over
   that crate — the exact Principle V "thin shell" pattern.
2. `.specify/` mirrors the path the constitution names. Do not move it
   without an amendment.
3. The brainstorming skill's default for spec location is
   `docs/superpowers/specs/`, which this layout adopts.

## Templates committed in this sub-project

The constitution's governance section requires three templates. Each ships
with a fixed shape so that later sub-projects can produce conformant
artefacts mechanically.

### `spec-template.md`

Frontmatter fields: `title`, `date`, `status`, `sub-project`, `supersedes`.

Section order: Problem & motivation · Goals & non-goals · Recommended
approach (with discarded alternatives noted) · Architecture · Data model ·
Interfaces (CLI / library / wire) · Error handling · Testing strategy ·
Open questions · **Constitution Check** (per-principle table covering
Principles I, II, III, V, VI, X).

The Constitution Check appears in the spec because the spec is where
principle questions get *answered*; the plan re-verifies them as a gate.

### `plan-template.md`

Frontmatter fields: `spec` (path to the spec), `sub-project`, `status`,
`target-release`.

Section order: Approach summary (one paragraph from the spec) ·
Step-by-step implementation milestones · Task list (using
`tasks-template.md` entries) · **Constitution Check** (mandatory; one to
two lines per Principle I, II, III, V, VI, X) · Risks & mitigations ·
Verification plan (including perf-budget measurements where Principle X
applies) · Rollback plan if applicable.

### `tasks-template.md`

Each task entry has required fields: `id`, `subject`, `description`,
`acceptance-criteria`, `assigned-skill`, `blocks`, `blocked-by`, `owner`.

The `assigned-skill` field is a free-form string in v0 (e.g.
`test-driven-development`, `rust-best-practices`, `vitest-testing`,
`biome-linting`, `accessibility-auditor`, `systematic-debugging`). It is
the load-bearing field that makes the constitutional instruction "assign
relevant skills to the tasks you create" mechanical rather than
aspirational. It may be constrained to an enum in a later amendment once
the skill catalog stabilises.

## Top-level governance files

### `README.md`

Three short sections: a one-line tagline; a status banner ("Pre-v0.1 ·
bootstrap phase · constitution v0.2.0 ratified 2026-05-15"); a one-paragraph
open/closed split summary linking to `.specify/memory/constitution.md`; a
build/run section (currently: `cargo build && ./target/debug/singularmem
--version`); a contributing pointer; a license footer. No marketing copy —
the constitution does that work.

### `CONTRIBUTING.md`

Dev environment setup (`rustup`, `cargo fmt`, `cargo clippy`); the
sub-project workflow (brainstorm → spec → plan → tasks → implementation →
review, with skill links); PR requirements (Constitution Check passes,
perf budgets unbroken, tests added, fmt + clippy clean); **DCO sign-off
required on every commit** (`Signed-off-by:`). **No CLA.** The open-core
model's structural rule — that proprietary code never ingests open
contributor code; it only links via Apache-2.0 APIs — makes a CLA
unnecessary. Apache-2.0 already grants what the proprietary tier needs.

### `SECURITY.md`

Coordinated disclosure to `security@singularmem.dev`. Scope: open-core
components only (no proprietary code exists yet). Acknowledgement SLA:
7 days. Fix-or-disclose SLA: 90 days. PGP key TBD; not blocking.

### `CODE_OF_CONDUCT.md`

Contributor Covenant 2.1, verbatim. Contact: `security@singularmem.dev`
(same inbox; can be split if reporting volume warrants it later).

### `LICENSE` and `NOTICE`

`LICENSE`: full Apache-2.0 text.
`NOTICE`: `Copyright 2026 Jonas Broms and Singularmem contributors.
Licensed under the Apache License, Version 2.0.`

### Configuration files

- `.gitignore` — Rust standard: `target/`, `*.swp`, `.DS_Store`, `.idea/`,
  `.vscode/` (except shared config files).
- `.editorconfig` — UTF-8, LF line endings, 4-space indent for Rust,
  2-space for TOML/YAML/JSON/MD.
- `rustfmt.toml` — `edition = "2021"`, `max_width = 100`,
  `imports_granularity = "Crate"`.
- `clippy.toml` — empty file; lint configuration lives in
  `[workspace.lints]` in `Cargo.toml` (pedantic + nursery enabled, with
  explicit allow-list for the noisy lints).
- `rust-toolchain.toml` — channel `stable`, components `rustfmt clippy`.
  Pins the toolchain so CI and contributors converge.

### `Cargo.toml` (workspace root)

```toml
[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.0.0"
edition = "2021"
rust-version = "1.80"
license = "Apache-2.0"
repository = "https://github.com/jonasbroms/singularmem"
authors = ["Jonas Broms"]

[workspace.lints.clippy]
pedantic = "warn"
nursery = "warn"
# explicit allow-list for noisy lints — populated as needed

[package]
name = "singularmem"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true

[[bin]]
name = "singularmem"
path = "src/main.rs"
```

### `src/main.rs`

```rust
fn main() {
    println!("singularmem {}", env!("CARGO_PKG_VERSION"));
}
```

That is the entire bootstrap binary. It exists so install, build, and CI
pipelines are real and verifiable.

## CI

Single workflow at `.github/workflows/ci.yml`. Triggers: push to `main`,
all pull requests. Runs on `ubuntu-latest` (the primary reference
runner). A separate advisory job runs on `macos-latest` and is marked
non-blocking.

Blocking jobs on `ubuntu-latest`:

- **fmt** — `cargo fmt --all -- --check`
- **clippy** — `cargo clippy --all-targets --all-features -- -D warnings`
- **check** — `cargo check --all-targets --all-features`
- **build** — `cargo build --all-targets --release`
- **test** — `cargo test --all-targets` (trivially passes; harness is in
  place for sub-project 1)
- **audit** — `cargo audit` against the RustSec advisory database

The toolchain is pinned via `rust-toolchain.toml`. Dependency caching is
handled by `Swatinem/rust-cache`.

**No release pipeline.** Deferred to the distribution sub-project.

**Principle X explicitly deferred.** There is no code to measure in
bootstrap. The first perf-budget gate lands alongside the first crate in
sub-project 1.

**Principle VI satisfied trivially.** No tests means no network
dependency in the test phase. From sub-project 1 onward, tests must run
with the network disabled.

## Data model

N/A for bootstrap. The first data model is introduced by sub-project 1
(Memory Store v0).

## Interfaces

- **CLI**: `singularmem --version` is the only command. Output:
  `singularmem 0.0.0` to stdout. Exit code 0. No flags. No subcommands.
- **Library**: no public library API in bootstrap. The first library API
  is introduced by sub-project 1.
- **Wire (MCP, HTTP, etc.)**: none. Introduced by later sub-projects.

## Error handling

N/A. The do-nothing binary has no failure modes. Principle VII's full
weight applies from sub-project 1 onward.

## Testing strategy

No unit or integration tests are added in bootstrap. The test harness is
present (`cargo test --all-targets` runs cleanly with zero tests). The
first tests appear in sub-project 1, where they must:

- Run with networking disabled (Principle VI).
- Cover every end-to-end memory operation (ingest → index → query →
  retrieve → export) using only open components (Principle III.b).

## Open questions

The bootstrap implementation must resolve, but they are operational
rather than design:

1. **DCO enforcement mechanism** — the DCO GitHub App (legacy) versus a
   Probot or GitHub Action equivalent. Either works; pick during
   implementation. Acceptance criterion 7 requires *some* DCO check to
   be active.
2. **`security@singularmem.dev` registration** — depends on domain
   acquisition. Out-of-repo task; tracked in the plan.
3. **GitHub repository visibility** — bootstrap PR assumes public from
   day one. If a private incubation period is preferred, flip the visibility
   when creating the repo. Not load-bearing for the spec.

## Acceptance criteria

Bootstrap is done when *all* of the following are observable on `main`:

1. **`.specify/memory/constitution.md`** committed at v0.2.0 with all
   eight placeholders resolved to the values in
   [Resolved constitution placeholders](#resolved-constitution-placeholders).
   Verified by `grep -E '\[OPEN_LICENSE\]|\[COMMERCIAL_LICENSE\]|\[REFERENCE_HARDWARE\]|\[INDEX_QUERY_P95_MS\]|\[INGEST_THROUGHPUT_PER_S\]|\[STARTUP_BUDGET_MS\]|\[BINARY_SIZE_BUDGET_MB\]' .specify/memory/constitution.md`
   returning empty.
2. **`.specify/templates/{spec,plan,tasks}-template.md`** committed with
   the sections from [Templates committed in this sub-project](#templates-committed-in-this-sub-project).
3. **Repo layout** matches [Repo layout](#repo-layout).
4. **Governance files** (README, CONTRIBUTING with DCO + no CLA, SECURITY
   pointing at `security@singularmem.dev`, CODE_OF_CONDUCT Contributor
   Covenant 2.1, LICENSE Apache-2.0 full text, NOTICE) all committed.
5. **`cargo build --release && ./target/release/singularmem --version`**
   prints `singularmem 0.0.0` and exits 0.
6. **CI green** on the bootstrap PR — fmt, clippy, check, build, test,
   audit all pass on `ubuntu-latest`. The `macos-latest` advisory job
   runs but does not gate.
7. **DCO enforcement live** — verified by an unsigned commit being
   rejected on a test PR.
8. **`security@singularmem.dev` resolves** to a real inbox the maintainer
   can read. (Out-of-repo task; tracked in the plan.)
9. **No `[PLACEHOLDER]` strings remain** in `constitution.md` or any
   committed governance file.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | Bootstrap ships only governance artefacts and a do-nothing binary. No data, no network at runtime, no server. Trivially compliant. |
| **II — Provider-Agnostic by Contract** | No provider integration in bootstrap. First relevance is sub-project 3. |
| **III — Open Core with a Stable Boundary** | Wholly on the open side. Apache-2.0 chosen. The one-way ratchet (III.a), open-side viability (III.b), and paid-tier exit (III.c) become governance-mechanizable because the constitution and templates exist in-tree from this PR forward. The amendment procedure requires re-checking III.a on every Open/Closed Split change. |
| **V — Composable Library Architecture** | The workspace shape enforces "thin shells over libraries". No crate is created prematurely; the first crate appears in sub-project 1 when there is a library to put in it. |
| **VI — Deterministic and Offline-Testable** | No tests in bootstrap → trivially passes. Test harness is in place. Sub-project 1 must demonstrate offline-pass on its first commit. |
| **X — Performance Budgets, Enforced in CI** | Budgets are *defined* in the constitution as part of this PR. Enforcement is deferred to sub-project 1 (no code to measure). The deferral is explicit and recorded here. |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy
Telemetry Boundary), and IX (Accessible by Default) are not specifically
re-checked here because bootstrap touches none of their surfaces (no
non-CLI surface; no failure modes; no telemetry; no interactive UI beyond
`--version`). They will be re-checked in every sub-project that does
touch them.
