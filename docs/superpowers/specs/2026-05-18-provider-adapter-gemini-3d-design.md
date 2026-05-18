---
title: Provider adapter — Gemini (sub-project 3d)
date: 2026-05-18
status: draft
sub-project: 3d-provider-adapter-gemini
supersedes: none
---

# Provider adapter — Gemini (sub-project 3d)

This sub-project adds the third and final cloud-provider implementation
of the typed `Adapter` contract Singularmem shipped in sub-project 3a
(v0.5.0). A new crate `singularmem-adapter-gemini` exposes
`GeminiAdapter`, a pure-formatter implementation tailored to Google
Gemini's preferred prompt style — em-dash-separated source headers
with a leading "ground your answer" directive that aligns with Vertex
AI's grounding-API vocabulary.

**Completion milestone**: After 3d merges, all four constitutionally-
required providers ship (`plain` + `claude` + `openai` + `gemini`).
The constitution's Principle II "Provider-Agnostic by Contract" deliverable is fully
discharged for v0.

## Problem & motivation

The constitution's Principle II requires at-minimum-four provider
integrations through a single typed adapter contract. After sub-projects
3a (foundation + `plain`), 3b (Claude), and 3c (OpenAI/Codex), only
Gemini remains. Landing 3d closes out the Principle II requirement.

Gemini is a substantively different provider from Claude and OpenAI:
- Anthropic's docs pin Claude on `<documents>` XML.
- OpenAI converges on bracketed `[N]` citation markers via cookbook
  examples.
- Google's docs don't pin a single canonical format, but Vertex AI's
  grounding API conventions favour fact-statement-style source
  attribution.

GeminiAdapter picks a format that's distinctly different from the
three existing adapters — em-dash-separated source headers — and
includes a leading directive ("Use the following sources to ground
your answer.") that aligns with Gemini's training distribution on
grounded-response patterns.

Validating a fourth adapter on the same trait contract further proves
the design generalises beyond the first three formats. After 3d
lands, the Principle II surface is complete and the registry pattern
is fully exercised across four independent removable adapters.

## Goals & non-goals

### Goals

1. New crate `singularmem-adapter-gemini` containing `GeminiAdapter`
   as a pure-formatter `Adapter` implementation.
2. Output shape uses em-dash-separated `Source N` headers with a
   leading "Use the following sources to ground your answer." directive.
   Per-block header is `Source N:` when both `source` and `tags` are
   absent, otherwise `Source N — source: ..., tags: ...:` (metadata
   comma-joined in source-then-tags order). Content immediately
   follows on the next line (no blank between header and content).
   Blocks separated by a single blank line; no trailing blank after
   the last block.
3. CLI registry integration: `--adapter gemini` becomes a valid
   choice; the unknown-adapter error message extends to its v0
   final form. The 3d placeholder comment in `known_adapters()`
   is removed.
4. CLI integration test confirms `retrieve --adapter gemini` produces
   the source-header shape end-to-end.
5. Workspace version bumps to `v0.8.0` on merge.
6. Project memory updated to record that **Principle II is fully
   discharged for v0** — all four required providers ship.

### Non-goals

- **No HTTP client, no auth, no streaming.** GeminiAdapter is a pure
  function. Calling the Gemini API is the user's job.
- **No memory ingest from Gemini responses.** Reading direction only.
- **No token-budget management.** `max_blocks` caps input; the
  formatter does not measure tokens against Gemini's context window.
- **No multimodal-aware framing.** GeminiAdapter handles text-only
  memory blocks (Singularmem only stores text content per
  sub-project 1's design). If multimodal memory support arrives in
  a future sub-project, the adapter trait's contract may need
  reconsideration; that's out of scope here.
- **No function-calling / structured-output schema.** Gemini's
  function-calling JSON shape is an API-side concern, not a
  retrieval-formatting one. GeminiAdapter emits plain text suitable
  for system or user message parts.
- **No support for multiple Gemini model variants** with different
  format preferences. The em-dash source-header pattern works well
  across the Gemini family (1.0, 1.5, 2.0 generations); future
  divergence becomes its own sibling adapter.
- **No content escaping or quoting.** Content is emitted verbatim.
- **No on-disk changes.** Adapter is read-time only.

## Recommended approach

A new crate `crates/singularmem-adapter-gemini/` depends only on
`singularmem-retrieve` for production (dev-deps on
`singularmem-core`/`singularmem-search`/`jiff` for `MemoryBlock`
fixture construction in tests). `GeminiAdapter` is a public unit
struct. `name()` returns `"gemini"`. `format(&RetrievedContext)` walks
the blocks, emitting:

```
Use the following sources to ground your answer.

Source 1 — source: claude-conversation:abc-123, tags: fox, animals:
the quick brown fox jumps over the lazy dog

Source 2:
lazy dogs sleep all day
```

The leading directive line is part of the adapter's output, not the
user's responsibility. The em-dash separator (`—`, U+2014) before
metadata distinguishes blocks with metadata from bare blocks (`Source
N:` with nothing else) cleanly. Metadata is `source: ...` and/or
`tags: ...`, comma-joined in source-then-tags order. The header line
ends with a colon; content immediately follows on the next line (no
blank between).

Empty `blocks` → `No grounding sources matched for query: "..."\n`.
The word "grounding" carries from the directive vocabulary, making
the empty-state phrasing distinctly Gemini-flavored even compared
to OpenAI's `"No memories matched for query: ..."`.

Module layout is a single `src/lib.rs` (~250 lines including tests).
Same template as 3b/3c.

CLI integration is two edits in two files: one `Cargo.toml` dep line
in the root binary, one line in `src/main.rs::known_adapters()` (which
replaces the last remaining `// 3X will add:` placeholder). The
existing `tests/cli.rs::retrieve_unknown_adapter_errors` test
tightens its `known adapters:` substring assertion from
`plain, claude, openai` to `plain, claude, openai, gemini` —
this is the registry's v0 final form.

### Approaches discarded

- **Approach B — triple-hyphen Vertex AI grounding-style delimiters**
  (`--- Memory N ---`). Rejected because it's more verbose without
  carrying extra signal; the em-dash header is more compact and
  equally Gemini-flavored.

- **Approach C — Markdown `### Source N` h3 headings with bold
  metadata.** Rejected because it visually overlaps with
  PlainAdapter's `## memory N` h2 headings; the distinguishing
  heading level + bold keys aren't strong enough differentiators.

- **Approach D — JSON-structured output.** Rejected because it's a
  tool-response shape, not a context-injection shape. Users who want
  JSON can use `--json` on the `retrieve` verb (a 3a feature) — that
  bypasses the adapter entirely.

- **Approach E — XML matching Claude's shape.** Rejected because it
  would be redundant with `ClaudeAdapter` and fails to surface
  anything distinctly Gemini-flavored. Gemini does handle XML, but
  it equally handles other formats; choosing the format Google's own
  cookbook examples prefer is a stronger signal.

- **Approach F — ASCII `--` separator instead of em-dash.** Rejected
  because `—` (U+2014) is the natural typographic choice for source
  attribution and renders correctly in all modern terminals + IDEs
  + LLM clients. Sub-project 2c's `--show-ranks` already uses U+2014
  for missing-rank fallback; we have project precedent.

## Architecture

Components:

- **`crates/singularmem-adapter-gemini/`** — new crate, version
  `0.8.0` (workspace-locked). Single file under `src/`.
- **`crates/singularmem-adapter-gemini/Cargo.toml`** — minimal:
  workspace inheritance for version/edition/etc., `[lints] workspace
  = true`, single production dep on `singularmem-retrieve`, three
  dev-deps (`singularmem-core`, `singularmem-search`, `jiff`).
- **`crates/singularmem-adapter-gemini/src/lib.rs`** — module docs,
  `GeminiAdapter` unit struct, `Adapter` impl, `#[cfg(test)] mod
  tests` with twelve unit tests.
- **Root binary `Cargo.toml`** — one new `[dependencies]` line.
- **Root binary `src/main.rs::known_adapters()`** — one new `vec!`
  entry replacing the last `// 3d will add:` placeholder. **After
  this edit, no `// 3X will add:` markers remain** — the registry
  is in its v0 final form.
- **`tests/cli.rs`** — one new integration test, one substring
  assertion tightening on an existing test.

Layering stays clean: `core ← search ← retrieve ← adapters`.
GeminiAdapter depends only on `singularmem-retrieve`.

## Data model

**No changes.** GeminiAdapter consumes `RetrievedContext` /
`MemoryBlock` from sub-project 3a unchanged. No persistent data,
no on-disk artefacts, no `format_version` bump.

## Interfaces

### CLI

No new flags. `singularmem retrieve` from sub-project 3a gains
`gemini` as a valid `--adapter` value. All other flags
(`--limit`, `--min-score`, `--mode`, `--fetch-multiplier`, `--rrf-k`,
`--json`, `--show-elapsed`) compose with `--adapter gemini` unchanged.

End-to-end example:

```
$ singularmem retrieve --adapter gemini "auth migration"
Use the following sources to ground your answer.

Source 1 — source: claude-conversation:abc-123, tags: auth, decision:
We decided to use Argon2id for password hashing because...

Source 2:
Migration plan deadline pushed to next sprint.
```

When the user passes `--json`, the adapter is bypassed entirely (3a
behaviour preserved).

Unknown-adapter error message extends to its v0 final form:

```
$ singularmem retrieve --adapter wat "query"
singularmem: usage: unknown adapter 'wat'; known adapters: plain, claude, openai, gemini
```

Exit code 1 (usage error).

### Library

`crates/singularmem-adapter-gemini/src/lib.rs`:

```rust
pub struct GeminiAdapter;

impl singularmem_retrieve::Adapter for GeminiAdapter {
    fn name(&self) -> &'static str { "gemini" }
    fn format(&self, ctx: &singularmem_retrieve::RetrievedContext) -> String {
        /* algorithm in Recommended Approach */
    }
}
```

No additional public types. No helpers exposed.

### Wire (MCP / HTTP / IPC)

N/A. Sub-project 4 (MCP server) will surface adapters through MCP;
this sub-project ships only the library + CLI registration.

## Error handling

GeminiAdapter has no failure modes. `Adapter::format` returns
`String`, not `Result<String, _>`, by trait contract. Any failure
inside formatting would manifest as a `write!` failure into the
in-memory `String`, which `std::fmt::Write` for `String` makes
impossible (only OOM, which panics globally).

Per Principle VII: there is nothing to surface, because there is
nothing that can fail. The CLI's existing error handling (from
sub-project 3a) covers all upstream failures before the adapter is
invoked.

## Testing strategy

### Unit tests (`crates/singularmem-adapter-gemini/src/lib.rs`)

| Test | What it pins down |
|---|---|
| `name_returns_gemini` | `GeminiAdapter.name() == "gemini"` |
| `format_includes_grounding_instruction_when_non_empty` | Output starts with `"Use the following sources to ground your answer."` line |
| `format_emits_one_indexed_source_headers` | Two-block input → `Source 1` and `Source 2` headers; no `Source 0` |
| `format_header_with_source_only` | Source present, no tags → `Source N — source: ...:` (no `tags:` substring on that line) |
| `format_header_with_tags_only` | Tags non-empty, source absent → `Source N — tags: a, b:` (no `source:` substring on that line) |
| `format_header_with_both_source_and_tags` | Both populated → `Source N — source: X, tags: a, b:` exactly in that order |
| `format_header_bare_when_no_metadata` | Both absent → header is `Source N:` exactly (no em-dash, no dangling separator) |
| `format_separates_blocks_with_blank_line` | Two blocks → blank line between block 1's content and block 2's `Source 2` header |
| `format_does_not_emit_trailing_blank_line_after_last_block` | Two blocks → output ends with last block's content + single `\n`, not `\n\n` |
| `format_empty_context_emits_no_match_line_with_grounding_phrasing` | Empty `blocks` → `No grounding sources matched for query: "..."\n` exactly; output contains the word `grounding` |
| `format_does_not_include_score_or_id_or_created_at` | Output never includes score, ULID, or `created_at` strings |
| `format_preserves_multiline_content_verbatim` | Content `"line one\nline two"` → both lines preserved as-is |

Twelve tests.

**Em-dash character in test source**: assertions use literal `Source N — source: ...` strings with U+2014. Rust source files are UTF-8; tests use `contains(...)` substring matching, which is byte-exact.

### CLI integration test (`tests/cli.rs`)

| Test | Verifies |
|---|---|
| `retrieve_with_gemini_adapter_emits_source_headers` | After ingest, `singularmem retrieve --adapter gemini fox` → exit 0, stdout contains the grounding directive line + `Source 1` header + full content |

### Updated existing test (`tests/cli.rs`)

`retrieve_unknown_adapter_errors` (last tightened in 3c to expect
`known adapters: plain, claude, openai`) tightens once more to
`known adapters: plain, claude, openai, gemini`. The fake-adapter
name (`nonexistent`) and the unknown-adapter substring assertion
stay unchanged. Single-line edit. Same gotcha as 3c: the loose
substring would otherwise pass accidentally; tightening confirms
the registry was actively extended.

After 3d, this is the registry's **v0 final form**. The assertion
stops growing here.

### Perf budget

**No new budget.** GeminiAdapter is pure string concatenation bounded
by `max_blocks × O(content size)`. Microsecond-scale work.

### Offline guarantee

Per Principle VI: GeminiAdapter is a pure function with no I/O. All
tests are unit tests over in-memory `RetrievedContext` values
constructed in the test body — no DB, no network, no file system.

## Open questions

None at spec time. Two notes for the implementation plan:

1. **`known adapters:` assertion churn ends here.** Sub-project 3d
   is the last cloud-adapter sub-project. After this PR merges, the
   v0 Principle II surface is complete and the assertion stops
   growing. No `// 3X will add:` placeholders should remain in
   `known_adapters()` after the registry edit.

2. **Em-dash character in tests.** Tests assert substrings like
   `"Source 1 — source:"` with literal U+2014. Plan should call out
   that Rust source files are UTF-8 and the em-dash compiles fine —
   no need for `\u{2014}` escape sequences. If `cargo test` ever
   surfaces a byte-mismatch issue (very unlikely), check the test
   source file's encoding.

## Acceptance criteria

1. New crate `crates/singularmem-adapter-gemini/` exists with
   `Cargo.toml` (workspace-version, depends only on
   `singularmem-retrieve` for production) and `src/lib.rs`.
2. `singularmem_adapter_gemini::GeminiAdapter` is a public unit
   struct implementing `singularmem_retrieve::Adapter`.
3. `GeminiAdapter::name()` returns `"gemini"`.
4. `GeminiAdapter::format(&RetrievedContext)` produces the em-dash
   source-header shape from the Recommended Approach section.
   Leading directive line on non-empty contexts. Per-block header is
   `Source N:` when both `source` and `tags` are absent, otherwise
   `Source N — <metadata>:` with `source:`/`tags:` parts comma-joined.
   Content immediately follows on the next line. Blank line between
   blocks; no trailing blank after the last block.
5. Em-dash `—` (U+2014) is the separator character, not ASCII `--`
   or `-`.
6. Empty `blocks` → `No grounding sources matched for query: "..."\n`
   exactly. Output contains the word `grounding`.
7. No content escaping; multiline content preserved verbatim.
8. Score, ULID, and `created_at` deliberately omitted from output.
9. Module layout: single `src/lib.rs` (~250 lines including tests).
10. Root binary `Cargo.toml` `[dependencies]` adds
    `singularmem-adapter-gemini = { path = "crates/singularmem-adapter-gemini" }`.
11. `src/main.rs::known_adapters()` registers
    `Box::new(singularmem_adapter_gemini::GeminiAdapter)` after
    `OpenAiAdapter`; the 3d placeholder comment is removed; **no
    `// 3X will add:` markers remain** — registry is in its v0 final
    form.
12. `tests/cli.rs::retrieve_unknown_adapter_errors` tightens its
    `known adapters:` substring assertion from
    `plain, claude, openai` to `plain, claude, openai, gemini`.
13. All 12 unit tests + 1 new CLI integration test from the Testing
    section pass on `ubuntu-latest` and `macos-latest`.
14. No new perf budget; formatter is microsecond-scale.
15. `docs/formats/store-v1.md` unchanged. `format_version` stays
    `"1"`.
16. Tagged on merge as `v0.8.0` (additive MINOR bump per Principle V).
17. **Completion milestone**: post-merge, all four Principle II
    required providers ship (`plain` + `claude` + `openai` +
    `gemini`). Sub-project 3 is complete. Project memory updated to
    reflect this.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. GeminiAdapter is a pure formatter; no API calls to Google, no auth, no streaming. |
| **II — Provider-Agnostic by Contract** | ✅ **COMPLETE after merge.** Fourth and final of four required providers. After 3d merges, the full Principle II surface ships: `plain` (local runtime) + `claude` + `openai` + `gemini`. The "removing any single provider adapter MUST NOT break non-provider features" property now holds for four independent removable adapters — each is a colocated three-edit removal (Cargo.toml dep, registry line, CLI test assertion). The principle is fully discharged for v0. |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (new crate, new registry entry). Nothing removed. III.b: no on-disk changes; `format_version` unchanged. |
| **IV — CLI-First** | `singularmem retrieve --adapter gemini <query>` works end-to-end. All eight existing retrieve flags compose with `--adapter gemini` unchanged. |
| **V — Composable Library Architecture** | Standalone crate with a single public type. Consumers can `cargo add singularmem-adapter-gemini` and use it directly. |
| **VI — Deterministic and Offline-Testable** | All tests are unit tests over in-memory `RetrievedContext` values (no DB, no network). Pure-function trait contract preserved. |
| **VII — Honest Failure Modes** | GeminiAdapter has no failure modes — `format` is infallible by trait contract. The CLI's existing 3a error handling covers all upstream failures before the adapter is invoked. |
| **VIII — Privacy Telemetry** | No telemetry added. No logging in the format path. |
| **IX — Accessible by Default** | Output uses UTF-8 (`—` U+2014) intentionally. Modern terminals/IDEs/LLM clients render it correctly; the em-dash is the natural typographic choice for source attribution. Project precedent: sub-project 2c's `--show-ranks` uses the same character. If a future user reports a Windows-cmd rendering issue, the fix is local (substitute `--` in the format) but not needed for v0. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Formatting cost is `max_blocks × O(content size)` string allocations; microseconds in practice. |
