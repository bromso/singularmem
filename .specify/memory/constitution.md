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
