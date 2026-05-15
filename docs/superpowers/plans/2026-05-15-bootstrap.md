# Singularmem Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship sub-project 0 of Singularmem — resolve every `[PLACEHOLDER]` in the project constitution and stand up the repo skeleton (Cargo workspace, do-nothing CLI, governance files, CI, DCO enforcement) so later sub-projects can hang off a stable foundation.

**Architecture:** Single feature branch (`bootstrap`) with one PR back to `main`. Six logical phases, each ending in a commit. No domain code; only governance artefacts, configuration files, and a 3-line do-nothing CLI binary. CI on GitHub Actions runs fmt/clippy/check/build/test/audit on `ubuntu-latest` (blocking) and `macos-latest` (advisory).

**Tech Stack:** Rust 1.80+ stable (workspace via Cargo, edition 2021); GitHub Actions for CI; `tim-actions/dco` GitHub Action for DCO sign-off enforcement; Apache-2.0 license; Contributor Covenant 2.1 for code of conduct.

---

**Frontmatter (per the plan-template this PR itself ships):**

- spec: `docs/superpowers/specs/2026-05-15-bootstrap-design.md`
- sub-project: `0-bootstrap`
- status: `ready-for-execution`
- target-release: `v0.0.0` (the constitution-ratification tag)

**Approach summary.** One feature branch, six commits, one PR. The constitution and governance scaffolding land first because everything downstream depends on them. The Cargo workspace and do-nothing binary land early so CI can run from day one. The PR cannot merge until the seven `ubuntu-latest` CI jobs pass and the DCO check confirms every commit is signed.

---

## Task 0: Pre-flight — create the feature branch

**Files:** none yet — git only.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Verify you are on `main` with a clean tree**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem status
git -C /Users/jonasbroms/Sites/singularmem log --oneline
```

Expected: branch is `main`; log shows the bootstrap design spec commit and the plan commit (the two documents that frame this work). Working tree clean — the pre-existing untracked entries (`.agents/`, `.claude/`, `skills-lock.json`) are not touched by this plan and may remain untracked.

- [ ] **Step 2: Create and check out the feature branch**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem checkout -b bootstrap
```

Expected: `Switched to a new branch 'bootstrap'`.

- [ ] **Step 3: Verify branch state**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem branch --show-current
```

Expected: `bootstrap`.

---

## Task 1: Configuration files

**Files:**

- Create: `.gitignore`
- Create: `.editorconfig`
- Create: `rust-toolchain.toml`
- Create: `rustfmt.toml`
- Create: `clippy.toml`

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Create `.gitignore`**

File: `/Users/jonasbroms/Sites/singularmem/.gitignore`

```gitignore
# Rust
/target
**/*.rs.bk
Cargo.lock.bak

# Editors
*.swp
*.swo
*~
.idea/
.vscode/*
!.vscode/settings.json
!.vscode/extensions.json

# OS
.DS_Store
Thumbs.db

# Misc
*.log
```

- [ ] **Step 2: Create `.editorconfig`**

File: `/Users/jonasbroms/Sites/singularmem/.editorconfig`

```editorconfig
root = true

[*]
charset = utf-8
end_of_line = lf
insert_final_newline = true
trim_trailing_whitespace = true

[*.rs]
indent_style = space
indent_size = 4

[*.{toml,yaml,yml,json,md}]
indent_style = space
indent_size = 2

[Makefile]
indent_style = tab
```

- [ ] **Step 3: Create `rust-toolchain.toml`**

File: `/Users/jonasbroms/Sites/singularmem/rust-toolchain.toml`

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 4: Create `rustfmt.toml`**

File: `/Users/jonasbroms/Sites/singularmem/rustfmt.toml`

```toml
edition = "2021"
max_width = 100
```

- [ ] **Step 5: Create `clippy.toml`**

File: `/Users/jonasbroms/Sites/singularmem/clippy.toml`

```toml
# Project-wide clippy lint configuration.
#
# Lint groups (pedantic, nursery) are configured in [workspace.lints.clippy]
# in Cargo.toml. This file is reserved for per-lint thresholds such as
# cognitive-complexity-threshold and is intentionally empty for v0.0.0.
```

- [ ] **Step 6: Verify all five files exist**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/.gitignore \
       /Users/jonasbroms/Sites/singularmem/.editorconfig \
       /Users/jonasbroms/Sites/singularmem/rust-toolchain.toml \
       /Users/jonasbroms/Sites/singularmem/rustfmt.toml \
       /Users/jonasbroms/Sites/singularmem/clippy.toml
```

Expected: all five listed with non-zero size.

---

## Task 2: Cargo workspace + do-nothing binary

**Files:**

- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `crates/.gitkeep`

**Assigned skill:** `rust-best-practices`

- [ ] **Step 1: Create `Cargo.toml` (workspace root + root package)**

File: `/Users/jonasbroms/Sites/singularmem/Cargo.toml`

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "0.0.0"
edition = "2021"
rust-version = "1.80"
license = "Apache-2.0"
repository = "https://github.com/jonasbroms/singularmem"
authors = ["Jonas Broms"]

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }

[package]
name = "singularmem"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[[bin]]
name = "singularmem"
path = "src/main.rs"
```

- [ ] **Step 2: Create `src/main.rs`**

Run:

```bash
mkdir -p /Users/jonasbroms/Sites/singularmem/src
```

File: `/Users/jonasbroms/Sites/singularmem/src/main.rs`

```rust
fn main() {
    println!("singularmem {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 3: Create the `crates/` directory placeholder**

`crates/*` is the workspace member glob. Git does not track empty directories, so add a `.gitkeep` so the directory survives a fresh clone. Cargo's glob will match nothing in `crates/` (the `.gitkeep` is a file, not a Cargo manifest directory), which is fine — the only workspace member is the root package until sub-project 1 adds the first crate.

Run:

```bash
mkdir -p /Users/jonasbroms/Sites/singularmem/crates
```

File: `/Users/jonasbroms/Sites/singularmem/crates/.gitkeep`

```
# This directory is reserved for workspace crates introduced by
# downstream sub-projects (Memory Store v0 onward). It is intentionally
# empty in the bootstrap commit.
```

- [ ] **Step 4: Run `cargo check` to confirm the workspace parses**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo check --all-targets --all-features
```

Expected: completes with `Finished` (compilation succeeds). Workspace lints may produce warnings; that is fine at this step — they will not be promoted to errors until CI runs with `-D warnings`.

- [ ] **Step 5: Run `cargo build --release` and execute the binary**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo build --release && ./target/release/singularmem
```

Expected stdout: exactly the line `singularmem 0.0.0`, exit code 0.

This is the manual verification for spec acceptance criterion 5. Confirm the output matches exactly before continuing.

- [ ] **Step 6: Run `cargo fmt --check`**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo fmt --all -- --check
```

Expected: no output, exit code 0.

- [ ] **Step 7: Run `cargo clippy` with the CI flags**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no clippy warnings, exit code 0.

If clippy flags an `expect_used`, `unwrap_used`, or similar pedantic lint on the do-nothing binary, the binary is genuinely too trivial for that lint to apply. Re-read the message and confirm — there should be no real issue. If a real lint surfaces, fix the code, not the lint configuration.

- [ ] **Step 8: Run `cargo test`**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && cargo test --all-targets
```

Expected: zero tests, zero failures. The harness runs cleanly because there are no tests in bootstrap (per the spec's testing strategy).

---

## Task 3: Commit Phase 1 — workspace skeleton

**Files:** none new; only commits already-created files.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Stage Phase 1 files**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem add \
  .gitignore \
  .editorconfig \
  rust-toolchain.toml \
  rustfmt.toml \
  clippy.toml \
  Cargo.toml \
  src/main.rs \
  crates/.gitkeep
```

- [ ] **Step 2: Verify the staged diff is exactly the Phase 1 files**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem status
```

Expected: 8 new files staged, no other changes staged or unstaged that you did not intend.

- [ ] **Step 3: Commit with DCO sign-off**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
chore: add Cargo workspace skeleton and do-nothing binary

Phase 1 of the bootstrap sub-project. Adds the configuration files
(.gitignore, .editorconfig, rust-toolchain.toml, rustfmt.toml,
clippy.toml), the workspace-root Cargo.toml with edition 2021 and
rust-version 1.80, a placeholder crates/ directory, and a three-line
do-nothing CLI binary at src/main.rs that prints
'singularmem 0.0.0' so the build pipeline is real and verifiable.

Signed-off-by: Jonas Broms <jonas@example.invalid>
EOF
)"
```

**Note:** The `-s` flag adds a `Signed-off-by:` trailer automatically using `git config user.email`. The explicit `Signed-off-by:` line in the message body is a defensive duplicate; if `-s` works on this machine (which it normally does once `user.email` is set), the trailer will appear once. If `git commit -s` produces an empty Signed-off-by, set `git config user.email` first. **Replace `jonas@example.invalid` with the real email** that `git config user.email` reports.

- [ ] **Step 4: Verify the commit has a sign-off trailer**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem log -1 --format='%B'
```

Expected: the commit body ends with a `Signed-off-by: ...` line. If it does not, run `git commit --amend -s` to add one.

---

## Task 4: Constitution file

**Files:**

- Create: `.specify/memory/constitution.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create the `.specify/memory` directory**

Run:

```bash
mkdir -p /Users/jonasbroms/Sites/singularmem/.specify/memory
```

- [ ] **Step 2: Create the constitution file**

File: `/Users/jonasbroms/Sites/singularmem/.specify/memory/constitution.md`

Use exactly the following content. This is the constitution v0.2.0 with every placeholder resolved to the values in the design spec.

```markdown
# Singularmem — Project Constitution

**Version:** 0.2.0
**Ratification date:** 2026-05-15
**Last amended:** 2026-05-15

---

## Mission

Singularmem is a **local-first persistent memory layer for LLM-driven
workflows**. It stores, indexes, and visualises the artefacts a developer or
agent accumulates over time — conversations, files, decisions, embeddings,
provenance — and exposes that memory to any LLM provider through a stable,
vendor-neutral interface.

It exists because the dominant alternatives leak intellectual property to
vendor-hosted memory stores, lock users into a single model provider, and treat
memory as an opaque feature rather than an inspectable artefact. This project
rejects that model.

The project ships as **open core**. The memory engine, on-disk format,
indexes, CLI, library SDK, provider adapters, and MCP server are open source.
The desktop GUI, premium visualisations, sync, and any future hosted or team
features are proprietary and paid. The boundary between the two is a
constitutional matter, not a product-management one — see Principle III and
the Open / Closed Split section.

Singularmem is the **memory component** of a broader, eventual ecosystem
covering an agent-native CLI, a thin IDE-like surface, context-compression
skills, and meta-prompting (spec → plan → create → verify). Those sister
projects live in their own repositories under their own constitutions; this
document governs the memory layer only.

---

## Core Principles

### I. Local-First and Sovereign

All memory, indexing, embedding generation, and search **MUST** run on the
user's machine by default. No memory data — raw or derived — leaves the
device without an explicit, scoped, revocable user action.

**Rationale:** Memory accumulates the highest-context, most sensitive material
a user produces. Treating it as cloud-default is the failure mode this project
is built to correct.

**Testable consequence:** A reference deployment **MUST** be operable with the
network interface disabled after first install, save for LLM provider calls
that the user explicitly initiates.

### II. Provider-Agnostic by Contract

The system **MUST** integrate with multiple LLM providers — at minimum Claude,
OpenAI/Codex, Gemini, and one fully local runtime — through a single typed
adapter contract. No principle, schema, on-disk format, or feature **MAY**
depend on the API shape of a single vendor.

**Rationale:** Vendor lock-in at the memory layer is structurally worse than at
the model layer, because memory is durable and migration-hostile.

**Testable consequence:** Removing any single provider adapter from the build
**MUST NOT** break any non-provider feature. Switching providers for an
existing memory store **MUST NOT** require re-indexing.

### III. Open Core with a Stable Boundary

The project ships as **open core**. The open components, licensed under
**Apache-2.0**, are open source and freely usable. The proprietary
components, licensed under a proprietary commercial license (full terms to be
published with the first proprietary release), are paid. The line between
the two is named in the Open / Closed Split section and governed by three
sub-rules:

**III.a — One-way ratchet.** The open / closed boundary **MAY** move toward
more open. It **MUST NOT** move toward more closed. A feature released as
open source remains open source, in this repository, under Apache-2.0, in
perpetuity. A feature released as proprietary **MAY** later be relicensed as
open; the reverse is **PROHIBITED** by this constitution.

**Rationale:** The single most damaging move an open-core company can make is
the rug-pull (Elastic 2021, HashiCorp 2023, Redis 2024). Each of those broke
user trust permanently. This rule pre-commits us against the option.

**III.b — Open-side viability.** The open core **MUST** be a complete,
self-sufficient product on its own. A user who never pays **MUST** be able to
ingest, index, query, retrieve, and export their entire memory using only
open components. Premium components are *additive convenience and experience*
— better visualisations, sync, polish, automation — never the core job.

**Testable consequence:** A test suite running against only the open
components **MUST** cover every end-to-end memory operation (ingest → index →
query → retrieve → export). If a workflow requires proprietary components to
function, it is either misclassified or violates this principle.

**III.c — Paid-tier exit guarantee.** Users on any paid tier **MUST** be able
to export all their data to the open on-disk formats and continue using the
open core without loss of memory, provenance, or metadata. The paid product
**MUST NOT** be a one-way door.

**Testable consequence:** A "downgrade to open core" command **MUST** exist
in the CLI and **MUST** be covered by integration tests on every release.

### IV. CLI-First, GUI-Visible

Every user-facing capability **MUST** be reachable through a deterministic,
text-in / text-out CLI before it appears in any graphical surface. The GUI is
a privileged consumer of the same library surface as the CLI — never a
parallel implementation.

**Rationale:** A CLI-first surface is the only one that is scriptable,
testable, agent-callable, and demonstrably non-leaky. Under the open-core
model, the CLI is also the open-source user's complete interface to the
product — so it cannot be a second-class citizen.

**Testable consequence:** Any GUI action **MUST** have a documented CLI
equivalent. Reviewers **MUST** reject GUI-only features.

### V. Composable Library Architecture

Every feature **MUST** exist first as a standalone library with a documented
public API and its own test suite. The CLI, desktop GUI, MCP server, and any
provider adapters are thin shells composing these libraries — they own
orchestration, not domain logic.

**Rationale:** The libraries are an open-source SDK as well as our own
internal building blocks. Third parties **MUST** be able to build their own
clients, UIs, and integrations on top of the open core without needing
anything from the proprietary tier.

### VI. Deterministic and Offline-Testable

Core operations **MUST** be deterministic given identical inputs and store
state. Side effects — network, system time, RNG, filesystem clock skew —
**MUST** be injected, not implicit. The full test suite **MUST** pass with
networking disabled.

**Rationale:** Memory systems that are flaky under test are flaky in
production in ways that destroy user trust. Non-determinism in memory recall
is indistinguishable from corruption from the user's perspective.

### VII. Honest Failure Modes

Errors **MUST** be surfaced to the user with three pieces of information: what
operation failed, what was attempted, and what state was preserved or rolled
back. Silent fallbacks, fabricated success states, and degraded-mode behaviour
that the user did not opt into are **PROHIBITED**.

**Rationale:** A memory tool that lies about what it remembers is worse than
one that remembers nothing.

### VIII. Privacy Telemetry Boundary

The project **MUST NOT** ship telemetry enabled by default in either tier. If
opt-in telemetry is added, it **MUST** be: locally aggregated, viewable as
plain text by the user before any transmission, severable without breaking
any feature, and never include memory content, embeddings, queries, or file
paths. This applies equally to the open and proprietary components.

### IX. Accessible by Default (WCAG 2.2 AA)

Every interactive surface — CLI prompts included — **MUST** meet WCAG 2.2 AA
where applicable. Keyboard navigation, screen-reader semantics, reduced-motion
support, and sufficient colour contrast are **NOT** post-launch tickets; they
ship with the feature or the feature does not ship. The proprietary GUI is
held to the same standard as the open CLI; paying does not buy worse
accessibility for non-paying users, nor better accessibility for paying ones.

### X. Performance Budgets, Enforced in CI

The project **MUST** publish numeric performance budgets and **MUST** measure
them in CI on the reference hardware defined below. Regressions block merge.

**Reference hardware.** Primary, blocking: GitHub Actions `ubuntu-latest`
(x86_64, 4 vCPU, 16 GB RAM). Secondary, advisory only: Apple Silicon
M-series Mac, 16 GB RAM.

Current budgets, measured on the primary reference runner:

- Index query latency: **p95 < 100 ms**
- Ingest throughput: **≥ 50 items/s**
- Cold start (CLI): **< 200 ms**
- Cold start (GUI): deferred until the Flutter sub-project ships a measurable build.
- Distributable binary size (CLI): **< 150 MB**

Budgets **MAY** be revised, but only via a constitution amendment with
explicit rationale.

---

## Open / Closed Split

This section is normative. Changing what side of the line a component lives
on requires a constitution amendment, subject to the one-way ratchet in
Principle III.a.

### Open (licensed under Apache-2.0)

- **Memory engine:** ingest pipeline, document model, ID and revision system.
- **On-disk storage format:** schema, versioning, migration tooling, format
  specification document. Third parties **MUST** be able to read a memory
  store without running our binary.
- **Indexes:** lexical (Tantivy-backed) and vector. Index format
  documented; rebuild tooling included.
- **Embedding generation:** local ONNX-based default; pluggable.
- **LLM provider adapters:** Claude, OpenAI/Codex, Gemini, and at least one
  local runtime. Adapter trait is the public extension point for community
  providers.
- **CLI:** complete surface — every operation, scriptable, deterministic.
- **MCP server:** so any MCP-compatible client (Claude Code, Cursor,
  custom agents) can use the open core as memory.
- **Library SDK:** Rust crates with documented public APIs, plus a
  TypeScript binding via napi-rs as the first non-Rust binding. Python
  is tracked as a separate sub-project, not part of v0.

### Proprietary (licensed under a proprietary commercial license — full terms TBD with the first proprietary release)

- **Desktop GUI** (Flutter): the polished, native, cross-platform
  application. Renders the open data through the open library APIs.
- **Premium visualisations:** sunburst/treemap, embedding projection,
  force-directed graph, lifeline, calendar heatmap, provenance trail,
  relevance falloff, diff view, tag ribbon. (The visualisation library
  itself is proprietary; the *data* it visualises is in open formats.)
- **Cross-device sync:** end-to-end encrypted, optional, paid.
- **Convenience automation:** scheduled ingest, watchers, pre-built
  skills/recipes for common workflows.
- **Hosted services**, if ever offered: hosted sync relay, hosted backup,
  hosted team coordination. None are planned for v1.

### Hard boundary rules

- **A proprietary component MUST NOT be required to perform any of:** ingest,
  index, query, retrieve, export. These are the core job and live entirely
  in the open tier (Principle III.b).
- **A proprietary component MUST NOT read or write a private on-disk
  format.** All persistence goes through the open, documented formats.
  Caches built by proprietary components are exempt only if they are
  reproducible from the open formats and disposable.
- **A community GUI built on the open libraries is permitted and welcome.**
  We will not pursue legal or technical measures to prevent it.

---

## Technology Constraints

Choices here are constitutional, not architectural-detail. They follow from
the principles above and require an amendment to change.

- **Core language (open):** Rust, for the memory engine, indexer, embedding
  host, CLI, provider adapters, and MCP server.
- **Desktop GUI language (proprietary):** Flutter, for cross-platform
  native rendering and the visualisation requirements.
- **Lexical index (open):** Tantivy.
- **Vector index (open):** to be selected during research phase between
  embedded candidates (LanceDB, Qdrant-embedded, USearch). Selection
  criteria **MUST** include: single-binary embedding, schema stability,
  third-party readability (Principle III), and incremental update
  performance.
- **Embedding generation (open):** local by default (fastembed or
  equivalent ONNX runtime). Cloud embedding providers **MAY** be offered as
  opt-in adapters but **MUST NOT** be the default.
- **LLM provider surface (open):** a typed Rust trait with concrete
  adapters for Claude, OpenAI/Codex, Gemini, and at least one local runtime
  (Ollama or llama.cpp). Additional adapters are community-extensible.
- **First non-Rust binding (open):** TypeScript via napi-rs. Python is a
  later sub-project.
- **Distribution:**
  - Open CLI: single signed binary, distributed via GitHub releases,
    homebrew, cargo, and direct download. Free.
  - Proprietary GUI: signed installers for macOS, Windows, and Linux,
    distributed via the project's website and OS app stores where viable.
    Sold via the project's licensing system.

---

## Visualisation Surfaces

The desktop GUI is the primary visualisation surface and is part of the
proprietary tier. Open-core users without the GUI can: use the CLI's tabular
output, query the open libraries directly to build their own visualisations,
or use any third-party community GUI.

The proprietary GUI **MUST** ship at minimum the three views marked (★) and
**SHOULD** ship the rest over time. Every graphical view **MUST** have an
accessible tabular equivalent (Principle IX).

- ★ **Sunburst / treemap** (DaisyDisk-style): hierarchy by source, project,
  or topic, sized by token weight or recency.
- ★ **2D embedding projection** (UMAP or t-SNE over the vector store):
  the most semantically honest view of a memory store. Clusters become
  visible without imposed taxonomy.
- ★ **Tabular / faceted list:** filterable, sortable, exportable. The
  unglamorous primary working view.
- **Force-directed cluster graph:** nodes are memory items, edges weighted
  by co-occurrence or vector similarity above threshold.
- **Temporal lifeline:** density of memory events over time, zoomable from
  years to minutes.
- **Calendar heatmap** (GitHub-contributions-style): activity by day,
  useful for spotting drought and burnout patterns.
- **Provenance trail:** for any retrieved snippet, the directed graph of
  sources, transformations, and consuming agents that touched it.
- **Query relevance falloff:** for any search, a histogram of relevance
  scores so the user can see *where* the meaningful results end.
- **Diff view:** the same memory item across versions/edits, with attribution.
- **Tag/facet ribbon:** stacked composition of tags or sources over time.

---

## Out of Scope (for this project)

These belong to sister projects in the broader ecosystem and **MUST NOT** be
absorbed into this repository:

- Agent runtime / orchestration (lives in the CLI project).
- Context compression skills (separate library).
- Meta-prompting workflows (spec → plan → create → verify) (separate tooling).
- A full IDE.
- Hosted multi-user collaboration. This is a single-user, local-first tool;
  team features, if ever built, require their own constitution.

---

## Governance

### Amendment procedure

Any change to this document requires:

1. A pull request modifying `.specify/memory/constitution.md`.
2. A Sync Impact Report at the top of the file (see the comment header).
3. Updates to `plan-template.md`, `spec-template.md`, and `tasks-template.md`
   for any principle that affects their gates.
4. An explicit rationale per principle changed.
5. **For any change to the Open / Closed Split section:** an explicit check
   against Principle III.a (the one-way ratchet). A pull request that moves a
   feature from open to closed **MUST** be rejected.

### Versioning policy

The constitution itself follows SemVer:

- **MAJOR:** a principle is removed, inverted, or has its MUST/SHOULD weight
  reduced. Also: any attempted move of a feature from open to closed (which
  this constitution prohibits, so a MAJOR bump here would represent a fork,
  not an amendment).
- **MINOR:** a new principle is added, or guidance is materially expanded.
  Also: moving a feature from closed to open (allowed and encouraged).
- **PATCH:** clarifications, typo fixes, wording — no semantic change.

### Commercial sustainability

The paid tier exists to sustain the open core, not the reverse. If commercial
revenue cannot fund continued development, the response **MUST** be to scale
back or sunset proprietary features — not to relicense open components. In
the event the project is wound down, the maintainers **SHOULD** make a
best-effort attempt to relicense the most-recent proprietary code under
Apache-2.0 as a parting gift to existing users.

### Compliance review

- `/speckit.analyze` **MUST** be run before any feature merge; constitutional
  violations are blocking.
- Every `plan.md` **MUST** contain a Constitution Check section that
  explicitly addresses Principles I, II, III, V, VI, and X. Other principles
  are checked but need not be re-stated unless the plan touches them.
- A quarterly review **SHOULD** revisit performance budgets (Principle X)
  against measured production data, and audit the Open / Closed Split for
  any drift or proposed boundary changes.

### Precedence

Where this document conflicts with any other guidance — including AI agent
instructions, downstream specs, plans, or task lists — **this document wins**.
The remedy for disagreement is an amendment, not a workaround. When you plan
everything, make sure to assign relevant skills to the tasks you create.
```

- [ ] **Step 3: Verify the constitution exists and contains no unresolved placeholders**

Run:

```bash
grep -E '\[OPEN_LICENSE\]|\[COMMERCIAL_LICENSE\]|\[REFERENCE_HARDWARE\]|\[INDEX_QUERY_P95_MS\]|\[INGEST_THROUGHPUT_PER_S\]|\[STARTUP_BUDGET_MS\]|\[BINARY_SIZE_BUDGET_MB\]' /Users/jonasbroms/Sites/singularmem/.specify/memory/constitution.md
```

Expected: no output, exit code 1 (grep finds nothing). This is the verification command from spec acceptance criterion 1.

If grep finds a match: stop. A placeholder remains. Re-read the spec's "Resolved constitution placeholders" table and fix the value in the constitution before continuing.

---

## Task 5: Spec template

**Files:**

- Create: `.specify/templates/spec-template.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create the `.specify/templates` directory**

Run:

```bash
mkdir -p /Users/jonasbroms/Sites/singularmem/.specify/templates
```

- [ ] **Step 2: Create `spec-template.md`**

File: `/Users/jonasbroms/Sites/singularmem/.specify/templates/spec-template.md`

```markdown
---
title: <Title of the sub-project or feature>
date: YYYY-MM-DD
status: draft | ready-for-implementation | superseded
sub-project: <e.g. 1-memory-store-v0>
supersedes: <path to prior spec, or 'none'>
---

# <Title of the sub-project or feature>

One short paragraph describing what this sub-project ships and why it
matters now.

## Problem & motivation

What problem does this solve? What is blocked until this lands? Why is
this the right time to do it?

## Goals & non-goals

### Goals

1. ...
2. ...

### Non-goals

- ...

## Recommended approach

The approach the spec adopts. One paragraph of summary, then any
necessary detail.

### Approaches discarded

- **Approach B — ...** Rejected because ...
- **Approach C — ...** Rejected because ...

## Architecture

Components, their responsibilities, and how they compose. Each component
must be a standalone library with a documented public API (Principle V).

## Data model

If the sub-project introduces or modifies a data model, describe it
here. On-disk formats must be documented (Principle III hard boundary
rule on private formats).

## Interfaces

- **CLI**: commands, flags, output shape, exit codes.
- **Library**: public API surface (function signatures, types).
- **Wire (MCP / HTTP / IPC)**: protocol contracts.

## Error handling

How failures are surfaced. Per Principle VII: errors must report what
operation failed, what was attempted, and what state was preserved or
rolled back. No silent fallbacks.

## Testing strategy

What is tested at which level (unit, integration, end-to-end).
Per Principle VI: tests must pass with networking disabled. Per
Principle III.b: tests must cover every end-to-end memory operation
using only open components.

## Open questions

Items the spec author could not resolve and which the implementation
plan or a brainstorming follow-up must address.

## Acceptance criteria

A numbered, observable, testable list. Each item names a verification
command or measurable outcome. The sub-project is done when all items
are observable on `main`.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | ... |
| **II — Provider-Agnostic by Contract** | ... |
| **III — Open Core with a Stable Boundary** | ... |
| **V — Composable Library Architecture** | ... |
| **VI — Deterministic and Offline-Testable** | ... |
| **X — Performance Budgets, Enforced in CI** | ... |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy
Telemetry), and IX (Accessible by Default) are re-checked in any
sub-project that touches their surfaces.
```

- [ ] **Step 3: Verify the file exists and has the required sections**

Run:

```bash
grep -c '^## ' /Users/jonasbroms/Sites/singularmem/.specify/templates/spec-template.md
```

Expected: at least 11 (one per top-level `##` section: Problem & motivation, Goals & non-goals, Recommended approach, Architecture, Data model, Interfaces, Error handling, Testing strategy, Open questions, Acceptance criteria, Constitution Check).

---

## Task 6: Plan template

**Files:**

- Create: `.specify/templates/plan-template.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `plan-template.md`**

File: `/Users/jonasbroms/Sites/singularmem/.specify/templates/plan-template.md`

```markdown
---
spec: docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md
sub-project: <e.g. 1-memory-store-v0>
status: draft | ready-for-execution | in-progress | merged
target-release: <e.g. v0.1.0>
---

# <Sub-project name> Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One sentence.

**Architecture:** Two or three sentences describing the shape of the
implementation.

**Tech Stack:** Key technologies / libraries.

---

## Approach summary

One paragraph lifted from the spec, describing how this plan delivers
the spec's recommended approach.

## Step-by-step implementation milestones

A bullet list of the major milestones in order. Each milestone maps to
one or more tasks below.

- M1 — ...
- M2 — ...

## Task list

The bite-sized tasks. Each task uses the `tasks-template.md` entry
shape. Tasks are checkbox-tracked and ordered for sequential execution
unless explicitly marked parallel.

### Task N: <Task title>

**Files:**

- Create: `path/to/file`
- Modify: `path/to/existing:line-range`

**Assigned skill:** `<skill-name>`

- [ ] **Step 1: ...**

(Repeat per step. Each step is one action of 2–5 minutes. Include
exact code, exact commands, expected output.)

## Constitution Check

| Principle | How this plan complies |
|---|---|
| **I — Local-First and Sovereign** | ... |
| **II — Provider-Agnostic by Contract** | ... |
| **III — Open Core with a Stable Boundary** | ... |
| **V — Composable Library Architecture** | ... |
| **VI — Deterministic and Offline-Testable** | ... |
| **X — Performance Budgets, Enforced in CI** | ... |

## Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| ... | ... | ... | ... |

## Verification plan

How we will know the sub-project succeeded:

- Build / lint / test commands and expected outputs.
- Acceptance criteria verification commands (one per criterion from
  the spec).
- Performance budget measurements where Principle X applies.

## Rollback plan

If applicable: how to revert this sub-project's changes if a
post-merge issue requires it. For purely additive sub-projects,
`git revert <merge-commit>` is usually sufficient and this section
may say so.
```

- [ ] **Step 2: Verify the file exists**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/.specify/templates/plan-template.md
```

Expected: file exists with non-zero size.

---

## Task 7: Tasks template

**Files:**

- Create: `.specify/templates/tasks-template.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `tasks-template.md`**

File: `/Users/jonasbroms/Sites/singularmem/.specify/templates/tasks-template.md`

```markdown
# Tasks Template

This document defines the required shape of a task entry inside a
`docs/superpowers/plans/*.md` plan. Every task in every plan **MUST**
follow this shape.

The `assigned-skill` field is load-bearing: it is the mechanism by
which the constitutional instruction "assign relevant skills to the
tasks you create" is enforced. A task without an `assigned-skill` is
incomplete.

## Required fields

| Field | Description |
|---|---|
| `id` | Stable identifier, monotonic within the plan (e.g. `Task 1`, `Task 2`). |
| `subject` | Imperative title (e.g. "Add CI workflow", "Write the failing test"). |
| `description` | One paragraph naming the files touched and what they accomplish. |
| `acceptance-criteria` | A short numbered or bulleted list of observable outcomes that prove this task is done. |
| `assigned-skill` | A free-form string naming the most relevant skill (e.g. `rust-best-practices`, `test-driven-development`, `vitest-testing`, `biome-linting`, `accessibility-auditor`, `systematic-debugging`, `verification-before-completion`). |
| `blocks` | List of task IDs that cannot start until this task completes. May be empty. |
| `blocked-by` | List of task IDs that must complete before this task can start. May be empty. |
| `owner` | The agent or human responsible for executing this task. May be unassigned at planning time. |

## Task entry shape

```markdown
### Task N: <subject>

**Files:**

- Create: `<path>`
- Modify: `<path>:<line-range>`
- Delete: `<path>`

**Assigned skill:** `<skill-name>`

**Blocked-by:** Task K, Task M (or "none")
**Blocks:** Task P (or "none")
**Owner:** <unassigned | agent-id | name>

**Description:** One paragraph explaining what this task does and why.

**Acceptance criteria:**

1. ...
2. ...

- [ ] **Step 1: <action>**

(exact command, exact code, expected output)

- [ ] **Step 2: <action>**

...
```

## Choosing an `assigned-skill`

Match the task to the skill that most directly governs its work:

- **Writing Rust code** → `rust-best-practices`
- **Writing tests first** → `test-driven-development`
- **Writing TypeScript/JS tests** → `vitest-testing`
- **Writing/linting TypeScript** → `biome-linting`
- **Auditing accessibility** → `accessibility-auditor`
- **Diagnosing failing tests or bugs** → `systematic-debugging`
- **Confirming acceptance criteria** → `verification-before-completion`
- **Reviewing finished work** → `requesting-code-review`

If no skill fits, `verification-before-completion` is the conservative
default. If multiple skills fit, pick the one whose checklist is most
load-bearing for the task's primary action.

## Free-form vs. enum

The `assigned-skill` field is free-form in v0 because the skill catalog
is itself evolving. A future amendment may constrain it to an enum once
the catalog stabilises.
```

- [ ] **Step 2: Verify the file exists**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/.specify/templates/tasks-template.md
```

Expected: file exists with non-zero size.

---

## Task 8: .specify/README and Phase 2 commit

**Files:**

- Create: `.specify/README.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `.specify/README.md`**

File: `/Users/jonasbroms/Sites/singularmem/.specify/README.md`

```markdown
# .specify/

This directory holds the **governance artefacts** for Singularmem.

- [`memory/constitution.md`](memory/constitution.md) — the project
  constitution. The source of truth for principles, the open/closed
  split, performance budgets, and the amendment procedure. Where this
  document conflicts with any other guidance — including AI agent
  instructions, downstream specs, plans, or task lists — the
  constitution wins.
- [`templates/`](templates/) — the required shape for spec, plan,
  and tasks documents. Sub-projects produce conformant artefacts by
  copying the templates.

Design specs and implementation plans live in
[`docs/superpowers/`](../docs/superpowers/), not here. This directory
is for things the constitution itself references and governs.
```

- [ ] **Step 2: Verify all four Phase 2 files exist**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/.specify/memory/constitution.md \
       /Users/jonasbroms/Sites/singularmem/.specify/templates/spec-template.md \
       /Users/jonasbroms/Sites/singularmem/.specify/templates/plan-template.md \
       /Users/jonasbroms/Sites/singularmem/.specify/templates/tasks-template.md \
       /Users/jonasbroms/Sites/singularmem/.specify/README.md
```

Expected: all five files listed with non-zero size.

- [ ] **Step 3: Stage Phase 2 files**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem add .specify/
```

- [ ] **Step 4: Commit Phase 2 with DCO sign-off**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
docs: add constitution v0.2.0 and governance templates

Phase 2 of the bootstrap sub-project. Commits the constitution at
.specify/memory/constitution.md with every placeholder resolved
(working title Singularmem, license Apache-2.0, reference hardware
ubuntu-latest x86_64 4vCPU 16GB primary, perf budgets 100ms p95 query
/ 50 items/s ingest / 200ms CLI cold-start / 150MB binary,
TypeScript-first non-Rust binding). Adds the three governance
templates (spec, plan, tasks) the constitution's amendment procedure
requires.

The tasks template's load-bearing field is `assigned-skill`, which
mechanises the constitutional instruction "assign relevant skills to
the tasks you create".

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5: Verify the commit has a sign-off**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem log -1 --format='%B'
```

Expected: commit body ends with a `Signed-off-by:` trailer.

---

## Task 9: LICENSE and NOTICE

**Files:**

- Create: `LICENSE`
- Create: `NOTICE`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Download the Apache-2.0 license text**

Run:

```bash
curl -fsSL https://www.apache.org/licenses/LICENSE-2.0.txt -o /Users/jonasbroms/Sites/singularmem/LICENSE
```

Expected: file created, no errors.

- [ ] **Step 2: Verify the LICENSE file looks correct**

Run:

```bash
head -3 /Users/jonasbroms/Sites/singularmem/LICENSE
wc -l /Users/jonasbroms/Sites/singularmem/LICENSE
```

Expected: file begins with `                                 Apache License` and `                           Version 2.0, January 2004`; total line count is approximately 202.

If the apache.org URL is unreachable, fall back to the canonical mirror at `https://raw.githubusercontent.com/spdx/license-list-data/main/text/Apache-2.0.txt` and re-run the verification.

- [ ] **Step 3: Create `NOTICE`**

File: `/Users/jonasbroms/Sites/singularmem/NOTICE`

```
Singularmem
Copyright 2026 Jonas Broms and Singularmem contributors.

This product is licensed under the Apache License, Version 2.0
(see LICENSE for the full text).

The proprietary components of Singularmem (the Flutter desktop GUI,
premium visualisations, and sync) are not part of this open-source
distribution and are governed by a separate commercial license.
```

- [ ] **Step 4: Verify both files exist**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/LICENSE /Users/jonasbroms/Sites/singularmem/NOTICE
```

Expected: both files exist with non-zero size.

---

## Task 10: README.md

**Files:**

- Create: `README.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `README.md`**

File: `/Users/jonasbroms/Sites/singularmem/README.md`

```markdown
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
```

- [ ] **Step 2: Verify file exists**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/README.md
```

Expected: file exists with non-zero size.

---

## Task 11: CONTRIBUTING.md

**Files:**

- Create: `CONTRIBUTING.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `CONTRIBUTING.md`**

File: `/Users/jonasbroms/Sites/singularmem/CONTRIBUTING.md`

```markdown
# Contributing to Singularmem

Thank you for considering a contribution. Singularmem is a
constitution-governed project; before opening a non-trivial PR, please
read [`.specify/memory/constitution.md`](.specify/memory/constitution.md).

## Development environment

You will need:

- Rust stable (the toolchain is pinned via `rust-toolchain.toml`; if
  `rustup` is installed, the right channel is selected automatically).
- `cargo fmt`, `cargo clippy`, and `cargo test` (installed with the
  `rustfmt` and `clippy` components, also pinned).
- Git 2.40 or newer.

Quick sanity check:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
./target/release/singularmem
# → singularmem 0.0.0
```

## Sub-project workflow

Singularmem work is organised as **sub-projects**. Each sub-project
goes through the full cycle:

1. **Brainstorm** — invoke the `brainstorming` skill to refine intent
   and pick an approach.
2. **Spec** — write the design to
   `docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md` following
   [`.specify/templates/spec-template.md`](.specify/templates/spec-template.md).
3. **Plan** — write the implementation plan to
   `docs/superpowers/plans/YYYY-MM-DD-<topic>.md` following
   [`.specify/templates/plan-template.md`](.specify/templates/plan-template.md)
   and using
   [`.specify/templates/tasks-template.md`](.specify/templates/tasks-template.md)
   for task entries.
4. **Tasks** — every task in the plan has an `assigned-skill` field.
5. **Implementation** — execute the plan task by task, committing
   frequently.
6. **Review** — invoke `requesting-code-review` before opening the
   PR.

## Pull request requirements

Every PR must:

- Pass `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets`.
- Include a **Constitution Check** in the linked plan addressing
  Principles I, II, III, V, VI, and X (per the constitution's
  governance section).
- Respect the performance budgets in Principle X (sub-project 1 and
  later — bootstrap is exempt).
- Add or update tests where the change touches testable code.
- Have **every commit signed off** (see DCO below).

## Developer Certificate of Origin (DCO), not a CLA

Singularmem uses the
[Developer Certificate of Origin 1.1](https://developercertificate.org/).
There is **no Contributor License Agreement**.

Every commit must include a `Signed-off-by` trailer:

```bash
git commit -s -m "your message"
```

The `-s` flag adds the trailer using `git config user.email`. CI
rejects unsigned commits.

The DCO is sufficient (a CLA is not necessary) because the open-core
model's structural rule — that proprietary code never *ingests* open
contributor code, only *links* to it via the Apache-2.0 library
boundary — means Apache-2.0 already grants the proprietary tier
everything it needs.

## Code of conduct

By participating in this project you agree to abide by the
[Contributor Covenant 2.1](CODE_OF_CONDUCT.md). Report violations to
`security@singularmem.dev`.

## Security

See [SECURITY.md](SECURITY.md) for the coordinated disclosure process.
```

- [ ] **Step 2: Verify file exists**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/CONTRIBUTING.md
```

Expected: file exists with non-zero size.

---

## Task 12: SECURITY.md

**Files:**

- Create: `SECURITY.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `SECURITY.md`**

File: `/Users/jonasbroms/Sites/singularmem/SECURITY.md`

```markdown
# Security Policy

## Reporting a vulnerability

Email `security@singularmem.dev` with a description of the
vulnerability and (if possible) a reproduction. Please do **not** file
a public GitHub issue for security reports.

If you prefer, GitHub Security Advisories' private vulnerability
reporting flow is also accepted; open a draft advisory via
`Security → Advisories → Report a vulnerability` on the repository.

## Scope

This policy covers the **open-source components** of Singularmem:
the memory engine, on-disk format, indexes, embedding pipeline, LLM
provider adapters, CLI, MCP server, and library SDK.

The proprietary components (desktop GUI, premium visualisations,
sync) are not part of this open-source distribution and are not
covered here.

## Response SLA

- **Acknowledgement:** within 7 calendar days of receiving the
  report.
- **Triage and fix-or-coordinate-disclosure:** within 90 calendar
  days of acknowledgement.

We will keep reporters informed throughout. If we cannot meet either
deadline, we will say so explicitly and propose a new one.

## What we ask of reporters

- Give us a reasonable opportunity to fix the issue before any public
  disclosure.
- Do not exploit the vulnerability beyond what is necessary to
  demonstrate it.
- Do not attempt to access data you do not own.

## What we will not do

- We will not pursue legal action against good-faith security
  researchers who follow this policy.
- We will not silently fix issues without crediting reporters who
  request acknowledgement.

## PGP

A PGP key for `security@singularmem.dev` is not yet published. This
is tracked as a follow-up; for the moment, transport-layer security
(TLS to the receiving mail server) is the assumed protection.
```

- [ ] **Step 2: Verify file exists**

Run:

```bash
ls -la /Users/jonasbroms/Sites/singularmem/SECURITY.md
```

Expected: file exists with non-zero size.

---

## Task 13: CODE_OF_CONDUCT.md

**Files:**

- Create: `CODE_OF_CONDUCT.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Download the Contributor Covenant 2.1 text**

Run:

```bash
curl -fsSL https://www.contributor-covenant.org/version/2/1/code_of_conduct/code_of_conduct.md \
  -o /Users/jonasbroms/Sites/singularmem/CODE_OF_CONDUCT.md
```

Expected: file created, no errors.

- [ ] **Step 2: Replace the contact placeholder with the project security email**

The downloaded file contains a placeholder line that looks roughly like:

```
[INSERT CONTACT METHOD].
```

Replace it with `security@singularmem.dev`. Run:

```bash
sed -i.bak 's|\[INSERT CONTACT METHOD\]|security@singularmem.dev|g' \
  /Users/jonasbroms/Sites/singularmem/CODE_OF_CONDUCT.md && \
rm /Users/jonasbroms/Sites/singularmem/CODE_OF_CONDUCT.md.bak
```

If the placeholder text in the Contributor Covenant has changed since this plan was written, open the file and replace whatever placeholder appears (the surrounding text "Instances of abusive, harassing, or otherwise unacceptable behavior may be reported to the community leaders responsible for enforcement at" identifies the location) with `security@singularmem.dev`.

- [ ] **Step 3: Verify no `[INSERT ...]` placeholder remains**

Run:

```bash
grep -E '\[INSERT' /Users/jonasbroms/Sites/singularmem/CODE_OF_CONDUCT.md
```

Expected: no output, exit code 1.

- [ ] **Step 4: Verify the file exists and references the security email**

Run:

```bash
grep -c 'security@singularmem.dev' /Users/jonasbroms/Sites/singularmem/CODE_OF_CONDUCT.md
```

Expected: at least 1 match.

---

## Task 14: Commit Phase 3 — top-level governance

**Files:** none new; commits files from Tasks 9–13.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Stage Phase 3 files**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem add \
  LICENSE \
  NOTICE \
  README.md \
  CONTRIBUTING.md \
  SECURITY.md \
  CODE_OF_CONDUCT.md
```

- [ ] **Step 2: Verify the staged set**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem status
```

Expected: six files staged, nothing else.

- [ ] **Step 3: Commit Phase 3 with sign-off**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
chore: add top-level governance files

Phase 3 of the bootstrap sub-project. Adds LICENSE (full Apache-2.0
text), NOTICE (attribution + commercial-tier scope note), README.md
(tagline, status banner, open/closed split summary, build snippet),
CONTRIBUTING.md (sub-project workflow, DCO sign-off requirement,
explicit no-CLA rationale), SECURITY.md (security@singularmem.dev,
7-day ack and 90-day fix-or-disclose SLAs), and CODE_OF_CONDUCT.md
(Contributor Covenant 2.1 with contact pointing at the security
inbox).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4: Verify the commit**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem log -1 --format='%h %s' && \
git -C /Users/jonasbroms/Sites/singularmem log -1 --format='%B' | grep -c 'Signed-off-by:'
```

Expected: commit appears in log; sign-off count is 1.

---

## Task 15: docs/superpowers READMEs and commit

**Files:**

- Create: `docs/superpowers/specs/README.md`
- Create: `docs/superpowers/plans/README.md`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Verify the parent directories exist (they should, from the initial commit)**

Run:

```bash
ls -d /Users/jonasbroms/Sites/singularmem/docs/superpowers/specs \
      /Users/jonasbroms/Sites/singularmem/docs/superpowers/plans
```

Expected: both directories listed.

- [ ] **Step 2: Create `docs/superpowers/specs/README.md`**

File: `/Users/jonasbroms/Sites/singularmem/docs/superpowers/specs/README.md`

```markdown
# Design specs

One spec per sub-project. Filename convention:
`YYYY-MM-DD-<topic>-design.md`.

Each spec follows
[`.specify/templates/spec-template.md`](../../../.specify/templates/spec-template.md).

A spec is the **decided** design. If the design changes substantially
after the spec is approved, write a new spec that supersedes the old
one — do not edit history. The `supersedes` frontmatter field records
the relationship.

Specs live here. Implementation plans live in
[`../plans/`](../plans/). The constitution governs both and lives in
[`.specify/memory/constitution.md`](../../../.specify/memory/constitution.md).
```

- [ ] **Step 3: Create `docs/superpowers/plans/README.md`**

File: `/Users/jonasbroms/Sites/singularmem/docs/superpowers/plans/README.md`

```markdown
# Implementation plans

One plan per sub-project. Filename convention:
`YYYY-MM-DD-<topic>.md`. The plan implements the spec of the same
date and topic in
[`../specs/`](../specs/).

Each plan follows
[`.specify/templates/plan-template.md`](../../../.specify/templates/plan-template.md)
and uses
[`.specify/templates/tasks-template.md`](../../../.specify/templates/tasks-template.md)
for the shape of every task entry.

A plan is **executable**. It contains the exact files, code, commands,
and expected outputs needed for a fresh engineer (human or agent) to
implement the sub-project. Vague tasks ("add appropriate error
handling", "implement later") are bugs in the plan.

When a plan completes — every task done, PR merged — its frontmatter
`status` becomes `merged`. The plan stays on disk as historical
record.
```

- [ ] **Step 4: Stage and commit**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem add docs/superpowers/specs/README.md docs/superpowers/plans/README.md && \
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
docs: add specs/ and plans/ directory READMEs

Phase 4 of the bootstrap sub-project. Documents the conventions for
sub-project specs and plans so that the next sub-project (Memory
Store v0) has a clear home for both artefacts and a link back to the
constitution that governs them.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 5: Verify**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem log -1 --format='%h %s'
```

Expected: latest commit subject is `docs: add specs/ and plans/ directory READMEs`.

---

## Task 16: CI workflow

**Files:**

- Create: `.github/workflows/ci.yml`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create the `.github/workflows` directory**

Run:

```bash
mkdir -p /Users/jonasbroms/Sites/singularmem/.github/workflows
```

- [ ] **Step 2: Create `ci.yml`**

File: `/Users/jonasbroms/Sites/singularmem/.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    name: rustfmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check

  clippy:
    name: clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --all-targets --all-features -- -D warnings

  check:
    name: cargo check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --all-targets --all-features

  build:
    name: cargo build (release)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --all-targets --release

  test:
    name: cargo test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-targets

  audit:
    name: cargo audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: rustsec/audit-check@v1
        with:
          token: ${{ secrets.GITHUB_TOKEN }}

  macos-advisory:
    name: macOS advisory (non-blocking)
    runs-on: macos-latest
    continue-on-error: true
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --all-targets --all-features
      - run: cargo test --all-targets
```

- [ ] **Step 3: Verify the YAML parses**

Run:

```bash
python3 -c "import yaml; yaml.safe_load(open('/Users/jonasbroms/Sites/singularmem/.github/workflows/ci.yml'))"
```

Expected: no output, exit code 0. If Python is unavailable, install `actionlint` (`brew install actionlint`) and run `actionlint .github/workflows/ci.yml` instead.

---

## Task 17: DCO workflow

**Files:**

- Create: `.github/workflows/dco.yml`

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Create `dco.yml`**

File: `/Users/jonasbroms/Sites/singularmem/.github/workflows/dco.yml`

```yaml
name: DCO

on:
  pull_request:
    types: [opened, synchronize, reopened]

jobs:
  dco-check:
    name: DCO sign-off
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Collect PR commits
        id: get-pr-commits
        uses: tim-actions/get-pr-commits@v1.3.1
        with:
          token: ${{ secrets.GITHUB_TOKEN }}

      - name: Verify each commit has a Signed-off-by trailer
        uses: tim-actions/dco@master
        with:
          commits: ${{ steps.get-pr-commits.outputs.commits }}
```

- [ ] **Step 2: Verify the YAML parses**

Run:

```bash
python3 -c "import yaml; yaml.safe_load(open('/Users/jonasbroms/Sites/singularmem/.github/workflows/dco.yml'))"
```

Expected: no output, exit code 0.

---

## Task 18: Commit Phase 5 — CI

**Files:** none new; commits Tasks 16–17.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Stage Phase 5 files**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem add .github/workflows/ci.yml .github/workflows/dco.yml
```

- [ ] **Step 2: Verify the staged set**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem status
```

Expected: two new files staged under `.github/workflows/`.

- [ ] **Step 3: Commit with sign-off**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem commit -s -m "$(cat <<'EOF'
ci: add GitHub Actions workflows for build and DCO

Phase 5 of the bootstrap sub-project. Adds two workflows:

- ci.yml: seven jobs on ubuntu-latest (fmt, clippy, check, build,
  test, audit) plus a macos-latest advisory job marked
  continue-on-error. -D warnings is applied to clippy via the CI
  command rather than RUSTFLAGS so non-CI builds remain warning-only.

- dco.yml: rejects PR commits that lack a Signed-off-by trailer,
  using tim-actions/dco. This is the CI gate that satisfies spec
  acceptance criterion 7.

Principle X (perf budget enforcement) is explicitly deferred to
sub-project 1, when the first measurable code lands.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4: Verify**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem log --oneline -6
```

Expected: a clean log of six commits on `bootstrap` (one initial spec commit on `main` followed by five phase commits on `bootstrap`).

---

## Task 19: Push branch and open the PR

**Files:** none; remote operations.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Confirm a GitHub remote is configured**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem remote -v
```

Expected: an `origin` pointing at `https://github.com/jonasbroms/singularmem.git` (or `git@github.com:jonasbroms/singularmem.git`).

If no remote is configured: the repository must be created on GitHub first. From the GitHub web UI, create the repository `jonasbroms/singularmem` (public visibility, no auto-generated README/LICENSE — those are already in this local repo). Then run:

```bash
git -C /Users/jonasbroms/Sites/singularmem remote add origin git@github.com:jonasbroms/singularmem.git
git -C /Users/jonasbroms/Sites/singularmem push -u origin main
```

The `git push -u origin main` step pushes the existing initial commit (the design spec) before the bootstrap branch is pushed.

- [ ] **Step 2: Push the `bootstrap` branch**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem push -u origin bootstrap
```

Expected: branch is pushed; GitHub returns a "Create a pull request" URL in the push output.

- [ ] **Step 3: Open the PR**

Use the `gh` CLI:

```bash
gh -R jonasbroms/singularmem pr create \
  --base main \
  --head bootstrap \
  --title "Bootstrap: constitution v0.2.0 + repo skeleton" \
  --body "$(cat <<'EOF'
## Summary

Sub-project 0 of Singularmem — resolves every placeholder in the
constitution, stands up the Cargo workspace skeleton, adds the
governance files and CI required by the spec.

- Constitution committed at \`.specify/memory/constitution.md\`
  v0.2.0 with placeholders resolved (Apache-2.0; reference hardware
  ubuntu-latest x86_64 4vCPU 16GB; perf budgets 100ms p95 query / 50
  items/s ingest / 200ms CLI cold-start / 150MB binary;
  TypeScript-first non-Rust binding).
- Cargo workspace skeleton + do-nothing CLI (\`singularmem 0.0.0\`).
- LICENSE (Apache-2.0), NOTICE, README, CONTRIBUTING (DCO, no CLA),
  SECURITY (security@singularmem.dev), CODE_OF_CONDUCT (Contributor
  Covenant 2.1).
- CI: fmt + clippy + check + build + test + audit on
  ubuntu-latest, plus macos-latest advisory.
- DCO enforcement on every PR commit.

Implements
[\`docs/superpowers/specs/2026-05-15-bootstrap-design.md\`](docs/superpowers/specs/2026-05-15-bootstrap-design.md).

## Test plan

- [ ] CI green on \`ubuntu-latest\` for all six blocking jobs.
- [ ] \`macos-latest\` advisory job runs but does not gate.
- [ ] DCO check passes (every commit on this branch is signed off).
- [ ] DCO check rejects an unsigned commit on a separate test branch.
- [ ] \`cargo build --release && ./target/release/singularmem\`
      prints \`singularmem 0.0.0\` and exits 0.
- [ ] \`grep -E '\\[OPEN_LICENSE\\]|\\[COMMERCIAL_LICENSE\\]|\\[REFERENCE_HARDWARE\\]|\\[INDEX_QUERY_P95_MS\\]|\\[INGEST_THROUGHPUT_PER_S\\]|\\[STARTUP_BUDGET_MS\\]|\\[BINARY_SIZE_BUDGET_MB\\]' .specify/memory/constitution.md\` returns empty.
- [ ] \`security@singularmem.dev\` resolves to a real inbox (manual,
      out-of-band — see Task 21 in the plan).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: `gh` prints the PR URL.

If `gh` is not authenticated, run `gh auth login` first and retry. If `gh` is unavailable, use the URL printed by `git push -u origin bootstrap` to open the PR in the GitHub web UI with the title and body above.

- [ ] **Step 4: Record the PR URL for later steps**

Save the PR number for use in Tasks 20–24. For example:

```bash
PR_URL=$(gh -R jonasbroms/singularmem pr view bootstrap --json url --jq '.url')
echo "$PR_URL"
```

---

## Task 20: Wait for CI and verify green

**Files:** none; remote verification.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Watch CI status**

Run:

```bash
gh -R jonasbroms/singularmem pr checks bootstrap --watch
```

Expected: all six blocking jobs (`rustfmt`, `clippy`, `cargo check`, `cargo build (release)`, `cargo test`, `cargo audit`) finish with status `pass`. The `macOS advisory (non-blocking)` job may pass or fail without affecting the PR.

If any blocking job fails:

1. Read the failure output via `gh -R jonasbroms/singularmem run view <run-id> --log-failed`.
2. Fix the underlying issue locally (do **not** bypass with `--no-verify` or by relaxing the CI flags).
3. Commit the fix (signed off, on the `bootstrap` branch).
4. `git push` — CI re-runs automatically.

Common surprise paths and their honest fixes:

- **clippy `-D warnings` fails on the do-nothing binary.** Read the lint name. If a specific `pedantic` or `nursery` lint genuinely fires on three lines of trivial code, add a targeted `allow` for that one lint name in `Cargo.toml`'s `[workspace.lints.clippy]`, with a comment recording why. Do **not** blanket-allow the whole `pedantic` group — the right granularity is the specific lint.
- **`cargo audit` fails on a transitive advisory.** The bootstrap binary has no real dependencies, but the rustsec database can flag something via the Cargo or std toolchain. Investigate the advisory; if it is not applicable, ignore it in `.cargo/audit.toml` with a documented reason.
- **DCO check fails on a commit.** Amend the commit with `git commit --amend -s --no-edit`, force-push the branch (`git push --force-with-lease`), and re-run.

- [ ] **Step 2: Capture passing-status proof**

Run:

```bash
gh -R jonasbroms/singularmem pr checks bootstrap
```

Expected: every required check shows `pass`. Save the output (paste into the PR as a comment if helpful) so the merge step has explicit evidence.

---

## Task 21: Test DCO enforcement (out-of-band)

**Files:** a throwaway commit on a throwaway branch — not merged.

**Assigned skill:** `verification-before-completion`

This task proves that the DCO check actually rejects unsigned commits, satisfying spec acceptance criterion 7. It happens on a **separate** branch so the bootstrap branch's commit history remains clean.

- [ ] **Step 1: Create a throwaway branch from `bootstrap`**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem checkout -b test/dco-rejection bootstrap
```

- [ ] **Step 2: Add a throwaway file and commit *without* `-s`**

Run:

```bash
echo "throwaway" > /Users/jonasbroms/Sites/singularmem/.dco-test && \
git -C /Users/jonasbroms/Sites/singularmem add .dco-test && \
git -C /Users/jonasbroms/Sites/singularmem commit -m "test: unsigned commit to verify DCO rejection"
```

Expected: the commit is created locally. **Do not include `-s`.**

- [ ] **Step 3: Push the branch and open a draft PR**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem push -u origin test/dco-rejection && \
gh -R jonasbroms/singularmem pr create \
  --base main \
  --head test/dco-rejection \
  --draft \
  --title "[DO NOT MERGE] DCO rejection test" \
  --body "Test PR to verify the DCO check rejects unsigned commits. Will be closed without merging."
```

- [ ] **Step 4: Confirm the DCO check fails on this PR**

Run:

```bash
gh -R jonasbroms/singularmem pr checks test/dco-rejection --watch
```

Expected: the **DCO** check finishes with status `fail`. The other CI checks may pass (the test/.dco-test file is unrelated to Rust).

If the DCO check passes when it should fail: the workflow is misconfigured. Re-read `.github/workflows/dco.yml`, check the action version (`tim-actions/dco@master`) is reachable, and re-run.

- [ ] **Step 5: Close the throwaway PR and delete the branch**

Run:

```bash
gh -R jonasbroms/singularmem pr close test/dco-rejection --delete-branch && \
git -C /Users/jonasbroms/Sites/singularmem checkout bootstrap && \
git -C /Users/jonasbroms/Sites/singularmem branch -D test/dco-rejection
```

Expected: the PR is closed, the remote branch is deleted, and you are back on `bootstrap`.

---

## Task 22: Out-of-band — register `singularmem.dev` and configure `security@`

**Files:** none in this repo; external services.

**Assigned skill:** `verification-before-completion`

This task is out-of-band (not a code change) but is required by spec acceptance criterion 8. It can run in parallel with Tasks 19–21 because it has no dependency on the bootstrap branch state.

- [ ] **Step 1: Register the `singularmem.dev` domain**

Through your preferred registrar (e.g. Namecheap, Porkbun, Cloudflare Registrar):

1. Verify `singularmem.dev` is available (`dig singularmem.dev` returns no A/MX records).
2. Purchase it. The `.dev` TLD requires WHOIS privacy by default and HSTS-preload, which are both fine.
3. Set the DNS provider to one that supports MX records and email forwarding (Cloudflare DNS is convenient; ImprovMX, Forward Email, or Migadu are simple forwarding services).

- [ ] **Step 2: Configure mail forwarding for `security@singularmem.dev`**

Through your chosen mail forwarding provider, create a forwarding rule:

- From: `security@singularmem.dev`
- To: your monitored personal inbox

Add the provider's required DNS records (typically two MX records and a TXT record for SPF). Verification UI from the provider should turn green within an hour.

- [ ] **Step 3: Confirm an email round-trip works**

From any other email account, send a brief test email to `security@singularmem.dev`. Confirm it arrives in your monitored inbox within 5 minutes. If it does not arrive:

1. Check the forwarding provider's logs.
2. Verify DNS propagation: `dig MX singularmem.dev +short`.
3. Wait up to 24h for DNS propagation if it is very fresh.

Do **not** mark this task complete until a real email round-trip succeeds.

---

## Task 23: Merge the PR

**Files:** none; remote merge operation.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Final pre-merge checklist**

Verify each of the following is true before merging:

```bash
# 1. CI is green
gh -R jonasbroms/singularmem pr checks bootstrap
# Expected: every blocking check is "pass".

# 2. DCO test happened
# (Verified manually in Task 21 — the test branch existed and the DCO check failed on it.)

# 3. Security email works
# (Verified manually in Task 22 — round-trip test email arrived.)

# 4. Build runs
gh -R jonasbroms/singularmem pr checkout bootstrap && \
cd /Users/jonasbroms/Sites/singularmem && \
cargo build --release && \
./target/release/singularmem
# Expected: "singularmem 0.0.0".

# 5. Placeholders all gone from the constitution
grep -E '\[OPEN_LICENSE\]|\[COMMERCIAL_LICENSE\]|\[REFERENCE_HARDWARE\]|\[INDEX_QUERY_P95_MS\]|\[INGEST_THROUGHPUT_PER_S\]|\[STARTUP_BUDGET_MS\]|\[BINARY_SIZE_BUDGET_MB\]' /Users/jonasbroms/Sites/singularmem/.specify/memory/constitution.md
# Expected: no output, exit code 1.
```

If any check fails, **stop**. Diagnose and fix before merging.

- [ ] **Step 2: Choose merge strategy**

Use a **merge commit** (not squash, not rebase). The bootstrap PR is structured as a sequence of meaningful phase commits; squashing them loses the rationale carried in each commit body. The merge commit becomes the marker for "Singularmem is now constituted".

- [ ] **Step 3: Merge**

Run:

```bash
gh -R jonasbroms/singularmem pr merge bootstrap \
  --merge \
  --delete-branch \
  --subject "Bootstrap: constitution v0.2.0 + repo skeleton (#<PR_NUMBER>)"
```

Replace `<PR_NUMBER>` with the actual PR number from Task 19.

Expected: PR is merged, `bootstrap` branch is deleted on the remote, `main` is now ahead of its prior tip by the five phase commits plus the merge commit.

- [ ] **Step 4: Pull main and verify**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem checkout main && \
git -C /Users/jonasbroms/Sites/singularmem pull && \
git -C /Users/jonasbroms/Sites/singularmem log --oneline -10
```

Expected: the log shows the merge commit at the tip of `main`, followed by the bootstrap phase commits in order.

- [ ] **Step 5: Run the final acceptance check on `main`**

Run:

```bash
cd /Users/jonasbroms/Sites/singularmem && \
cargo build --release && \
./target/release/singularmem
```

Expected: `singularmem 0.0.0`. Bootstrap is complete.

---

## Task 24: Tag the constitution ratification and update memory

**Files:** none new; a git tag + a memory update.

**Assigned skill:** `verification-before-completion`

- [ ] **Step 1: Tag the ratification**

Run:

```bash
git -C /Users/jonasbroms/Sites/singularmem tag -a constitution-v0.2.0 \
  -m "Constitution v0.2.0 ratified. Placeholders resolved; open-core boundary in force." && \
git -C /Users/jonasbroms/Sites/singularmem push origin constitution-v0.2.0
```

Expected: tag is created locally and pushed to the remote.

- [ ] **Step 2: Update the project memory to reflect the new state**

Edit `/Users/jonasbroms/.claude/projects/-Users-jonasbroms-Sites-singularmem/memory/project_singularmem_overview.md` and replace the sentence

> Until sub-project 0 (bootstrap) is implemented, the constitution does NOT
> yet exist in-repo — it lives only in the user's message history and in
> the design spec at
> `docs/superpowers/specs/2026-05-15-bootstrap-design.md`.

with

> Sub-project 0 (bootstrap) was implemented and merged on
> `<DATE>` (tag `constitution-v0.2.0`). The constitution lives in-repo
> at `.specify/memory/constitution.md`.

Replace `<DATE>` with the actual merge date. Also update the
"Decomposition" list to mark sub-project 0 as `(merged)` and sub-project 1
as the active candidate for the next brainstorm.

- [ ] **Step 3: Done**

Bootstrap is complete. The next sub-project (Memory Store v0) can now be
brainstormed under the constitution that this work ratified.

---

## Constitution Check

| Principle | How this plan complies |
|---|---|
| **I — Local-First and Sovereign** | Plan creates governance artefacts and a do-nothing CLI binary. The binary makes no network calls. CI runs on hosted runners, but no user data is involved. Trivially compliant. |
| **II — Provider-Agnostic by Contract** | No provider integration in this plan. First relevance is sub-project 3. |
| **III — Open Core with a Stable Boundary** | Every file added is under Apache-2.0. The NOTICE explicitly delineates the open vs. proprietary scope. III.a one-way ratchet becomes mechanically enforceable because the constitution + templates exist in-tree after Task 8. III.b open-side viability is preserved (no proprietary dependency in bootstrap). III.c paid-tier exit is N/A here (no paid tier yet). |
| **V — Composable Library Architecture** | The workspace shape (`Cargo.toml` workspace with `members = ["crates/*"]`) is the carrier for the "thin shells over libraries" pattern. The do-nothing binary at `src/main.rs` will become a thin shell over `crates/singularmem-core` in sub-project 1. No premature crate creation. |
| **VI — Deterministic and Offline-Testable** | The plan adds no tests. The `cargo test` harness runs cleanly with zero tests. Once tests appear in sub-project 1, the spec requires them to pass with networking disabled — a constraint the plan template's `Verification plan` section will surface for every downstream plan. |
| **X — Performance Budgets, Enforced in CI** | Budgets are **defined** in the constitution as part of this plan (Task 4). **Enforcement** is explicitly deferred to sub-project 1, when there is code to measure. The deferral is recorded both in the constitution (Principle X) and in the spec. |

Principles IV (CLI-First), VII (Honest Failure Modes), VIII (Privacy
Telemetry), and IX (Accessible by Default) are not specifically re-checked
here because bootstrap touches none of their surfaces (no non-CLI surface;
no failure modes; no telemetry; no interactive UI beyond `--version`).
Each will be re-checked in every sub-project that does touch them.

---

## Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Clippy `pedantic`/`nursery` lints fire on the do-nothing binary and block CI. | Low | Low | Pedantic and nursery on `fn main() { println!(...) }` have no real complaints. If something does fire, Task 20 step 1 lists the honest fix process. |
| `cargo audit` flags a transitive vulnerability in the empty workspace. | Very low | Low | The bootstrap binary has no dependencies beyond std. If a false positive surfaces, ignore it in `.cargo/audit.toml` with a documented reason. |
| `singularmem.dev` domain is taken. | Low | Medium | If unavailable, fall back to `singularmem.io` or `singularmem.app` and update SECURITY.md before merging. The `security@<domain>` convention is the load-bearing thing; the exact domain is not. |
| Apache or Contributor Covenant URLs are temporarily unreachable. | Very low | Low | Task 9 step 2 names a SPDX mirror fallback for the license; for the Code of Conduct, the Contributor Covenant text is also published on GitHub and Wikipedia and can be sourced from there. |
| The DCO action `tim-actions/dco@master` is yanked or breaks. | Low | Medium | A drop-in replacement (`pradnyalg/dco-action`) exists; if the install fails, swap the action and re-run. |
| Workspace lints `priority = -1` syntax is rejected by older Rust. | Very low | Low | The syntax is stable as of Rust 1.74. `rust-toolchain.toml` pins `stable`, which is well past that. If toolchain version drift occurs, pin a specific stable release. |
| Constitution placeholders re-introduced by accident in a future edit. | Low | High | Spec acceptance criterion 1 has a grep verification command; the CI workflow can be amended in a later sub-project to enforce it on every PR. |

---

## Verification plan

The nine verifications below correspond one-to-one with the spec's nine acceptance criteria.

1. **Placeholders resolved (spec criterion 1):** Task 4 step 3 grep verification, also exercised in Task 23 step 1 just before merge.
2. **Templates present (spec criterion 2):** Task 7 step 2 and Task 8 step 2 verify file existence.
3. **Repo layout (spec criterion 3):** The diff of the bootstrap PR vs. `main` is the verification. PR reviewer checks against the spec's repo-layout diagram.
4. **Governance files present (spec criterion 4):** Task 14 step 4 verifies the commit; Task 23 step 1 re-checks before merge.
5. **`singularmem --version` works (spec criterion 5):** Task 2 step 5 verifies locally; Task 23 step 1 step 4 re-checks on `main` after merge.
6. **CI green (spec criterion 6):** Task 20 step 2.
7. **DCO enforcement live (spec criterion 7):** Task 21 step 4 — an unsigned commit is rejected.
8. **`security@singularmem.dev` resolves (spec criterion 8):** Task 22 step 3 — a real email round-trip.
9. **No `[PLACEHOLDER]` strings remain (spec criterion 9):** Same grep as criterion 1, but applied to the broader committed-file set: `grep -rE '\[OPEN_LICENSE\]|\[COMMERCIAL_LICENSE\]|\[REFERENCE_HARDWARE\]|\[INDEX_QUERY_P95_MS\]|\[INGEST_THROUGHPUT_PER_S\]|\[STARTUP_BUDGET_MS\]|\[BINARY_SIZE_BUDGET_MB\]' .specify/ README.md CONTRIBUTING.md SECURITY.md CODE_OF_CONDUCT.md NOTICE Cargo.toml` should return empty.

Principle X performance budgets are **not measured** in this plan because there is no code to measure. Sub-project 1's plan must include this verification.

---

## Rollback plan

Because this plan is purely additive (no edits to existing files outside the design spec from the initial commit), `git revert <merge-commit>` is sufficient to undo it. The constitution would revert to "not committed", which is the pre-bootstrap state — design spec only on `main`.

If a partial rollback is needed (e.g., revert CI but keep the constitution), the phase commits are independent and can be reverted individually with `git revert <phase-commit>` plus a follow-up to re-stabilise. This should rarely be necessary; the more common path will be a forward-fix PR.

---

## Self-review notes (filled in after writing the plan)

**Spec coverage check:**

- Constitution placeholders resolved (criterion 1) → Task 4.
- Templates committed (criterion 2) → Tasks 5–7.
- Repo layout (criterion 3) → All file-creation tasks combined.
- Governance files committed (criterion 4) → Tasks 9–13.
- Binary works (criterion 5) → Task 2 step 5.
- CI green (criterion 6) → Tasks 16, 20.
- DCO live (criterion 7) → Tasks 17, 21.
- Security email resolves (criterion 8) → Task 22.
- No placeholders remain (criterion 9) → Task 4 step 3 + verification plan item 9.

All nine criteria covered.

**Placeholder scan:** No "TBD", "TODO", "implement later", or vague "add appropriate X" language. Out-of-band steps are explicitly named as out-of-band. PGP key intentionally deferred in SECURITY.md is called out with the reason.

**Type consistency:** No types or methods to be consistent across — this is a config / governance plan. The only Rust code is the three-line `main()`, which is self-consistent.
