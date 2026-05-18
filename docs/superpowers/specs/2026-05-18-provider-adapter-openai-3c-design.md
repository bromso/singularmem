---
title: Provider adapter — OpenAI/Codex (sub-project 3c)
date: 2026-05-18
status: draft
sub-project: 3c-provider-adapter-openai
supersedes: none
---

# Provider adapter — OpenAI/Codex (sub-project 3c)

This sub-project adds the second cloud-provider implementation of the
typed `Adapter` contract Singularmem shipped in sub-project 3a (v0.5.0).
A new crate `singularmem-adapter-openai` exposes `OpenAiAdapter`, a
pure-formatter implementation tailored to OpenAI/Codex's preferred
prompt style — bracketed-citation markers (`[N]`) with a leading
directive that primes GPT-family models to cite back by index. The
CLI's `known_adapters()` registry gains one line; users invoke it with
`singularmem retrieve --adapter openai <query>`.

## Problem & motivation

After sub-project 3b shipped `ClaudeAdapter`, three of the four
constitutionally-required providers from Principle II remain
unimplemented as separate cloud adapters (the `plain` adapter from 3a
satisfies the "one fully local runtime" requirement). 3c adds OpenAI/
Codex as the second cloud adapter. 3d (Gemini) follows.

OpenAI is the second most-used provider for this project's primary
user and represents a different prompt-engineering convention than
Anthropic. Where Anthropic's documentation pins down `<documents>`-
style XML, OpenAI doesn't publish a single canonical format. The
community + OpenAI Cookbook examples converge on Markdown with
citation markers — particularly the `[N]` index pattern that pairs
naturally with the "answer with citations" workflow many GPT-4
applications use.

Validating a second cloud adapter on the same trait contract proves
the design generalises beyond Claude's specific shape. After 3c
lands, the only remaining cloud adapter is Gemini (3d), which can
follow the same template with minimal further design work.

## Goals & non-goals

### Goals

1. New crate `singularmem-adapter-openai` containing `OpenAiAdapter`
   as a pure-formatter `Adapter` implementation.
2. Output shape uses bracketed citation markers (`[N]`) and a leading
   directive line that primes the model to cite by index. Same-line
   `source:` metadata in the header where present; own-line `tags:`
   below; blank line; full content. Blank-line block separation; no
   `---` horizontal rules.
3. CLI registry integration: `--adapter openai` becomes a valid
   choice; the unknown-adapter error message extends accordingly.
4. CLI integration test confirms `retrieve --adapter openai` produces
   the citation-marker shape end-to-end.
5. Workspace version bumps to `v0.7.0` on merge.

### Non-goals

- **No HTTP client, no auth, no streaming.** OpenAiAdapter is a pure
  function. Calling the OpenAI API is the user's job (or a future
  HTTP-helpers sub-project that does not exist yet).
- **No memory ingest from OpenAI responses.** Reading direction only,
  per the Adapter trait contract from 3a.
- **No token-budget management.** `max_blocks` caps the input to the
  formatter; the formatter does not measure tokens against any GPT
  context-window limit.
- **No tool-calling JSON shape.** OpenAI's function-calling responses
  use a structured JSON schema, but that's an API-side concern, not a
  retrieval-formatting one. OpenAiAdapter emits plain Markdown-
  compatible text suitable for system or user messages.
- **No support for multiple OpenAI model variants** with different
  format preferences. Bracketed-citation markers work well across the
  GPT-4 family + Codex; if a future model diverges (e.g., wants
  XML-formatted context), a sibling adapter becomes its own registry
  entry rather than a configuration flag on this one.
- **No content escaping or quoting.** Content is emitted verbatim.
  The theoretical collision (memory content containing `[N]` strings)
  is not worth defensive handling — see Recommended Approach.
- **No on-disk changes.** Adapter is read-time only.

## Recommended approach

A new crate `crates/singularmem-adapter-openai/` depends only on
`singularmem-retrieve` for production (dev-deps on
`singularmem-core`/`singularmem-search`/`jiff` for `MemoryBlock`
fixture construction in tests). `OpenAiAdapter` is a public unit
struct. `name()` returns `"openai"`. `format(&RetrievedContext)`
walks the blocks, emitting:

```
Use the following retrieved memories. Cite by [N] index.

[1] source: claude-conversation:abc-123
tags: fox, animals

the quick brown fox jumps over the lazy dog

[2]
lazy dogs sleep all day
```

The leading directive line is part of the adapter's output, not the
user's responsibility. Bracket markers are decoration without the
directive; with it, they're a contract the model honours. Users who
want different prompt scaffolding can layer their own around the
adapter's output.

Empty `blocks` → `No memories matched for query: "..."\n`. Note the
absence of `[`/`]` characters — brackets are reserved for citation
markers in this adapter's output, and an empty-state message like
`[no memories...]` could confuse a downstream parser scanning for
`[N]` references.

Module layout is a single `src/lib.rs` (~200 lines including tests).
Same call as 3b: one struct + one method + tests = one file.

CLI integration is two edits in two files: one `Cargo.toml` dep line
in the root binary, one line in `src/main.rs::known_adapters()`. The
existing `tests/cli.rs::retrieve_unknown_adapter_errors` test from
sub-project 3a (updated in 3b) extends its expected `known adapters:`
substring once more.

### Approaches discarded

- **Approach B — Markdown numbered headings** (`# Document N`).
  Rejected because it overlaps significantly with PlainAdapter's
  existing Markdown shape — less distinctly "OpenAI-flavored" and
  loses the citation-back-by-index affordance.

- **Approach C — code-fenced blocks** (triple-backtick wrappers).
  Rejected because it's atypical for OpenAI RAG patterns (more
  typical for showing actual code) and the bulletproofing against
  Markdown-in-content collisions isn't worth the visual noise.

- **Approach D — JSON-structured output** (`{"documents": [...]}`).
  Rejected because it's a tool-response shape, not a context-
  injection shape. Users who want JSON can use `--json` on the
  `retrieve` verb (a 3a feature) — that bypasses the adapter
  entirely and emits `RetrievedContext` directly.

- **Approach E — no leading directive line; bracket markers
  decorative only.** Rejected because the markers without the
  directive don't reliably trigger citation behaviour. The
  directive-plus-markers combo is the point.

## Architecture

Components:

- **`crates/singularmem-adapter-openai/`** — new crate, version
  `0.7.0` (workspace-locked). Single file under `src/`.
- **`crates/singularmem-adapter-openai/Cargo.toml`** — minimal:
  workspace inheritance for version/edition/etc., `[lints] workspace
  = true`, single production dep on `singularmem-retrieve`, three
  dev-deps (`singularmem-core`, `singularmem-search`, `jiff`).
- **`crates/singularmem-adapter-openai/src/lib.rs`** — module docs,
  `OpenAiAdapter` unit struct, `Adapter` impl, `#[cfg(test)] mod
  tests` with twelve unit tests.
- **Root binary `Cargo.toml`** — one new `[dependencies]` line.
- **Root binary `src/main.rs::known_adapters()`** — one new `vec!`
  entry; sub-project 3c's placeholder line-comment is removed; only
  3d's placeholder remains.
- **`tests/cli.rs`** — one new integration test, one substring
  assertion extension on an existing test.

Layering stays clean: `core ← search ← retrieve ← adapters`.
OpenAiAdapter depends only on `singularmem-retrieve`. It does not
depend on `singularmem-core` or `singularmem-search` directly —
every type it references (`RetrievedContext`, `MemoryBlock`,
`Adapter` trait) is re-exported from `singularmem-retrieve`.

## Data model

**No changes.** OpenAiAdapter consumes `RetrievedContext` /
`MemoryBlock` from sub-project 3a unchanged. No persistent data,
no on-disk artefacts, no `format_version` bump.

## Interfaces

### CLI

No new flags. `singularmem retrieve` from sub-project 3a gains
`openai` as a valid `--adapter` value. All other flags
(`--limit`, `--min-score`, `--mode`, `--fetch-multiplier`, `--rrf-k`,
`--json`, `--show-elapsed`) compose with `--adapter openai` unchanged.

End-to-end example:

```
$ singularmem retrieve --adapter openai "auth migration"
Use the following retrieved memories. Cite by [N] index.

[1] source: claude-conversation:abc-123
tags: auth, decision

We decided to use Argon2id for password hashing because...

[2]
Migration plan deadline pushed to next sprint.
```

When the user passes `--json`, the adapter is bypassed entirely (3a
behaviour preserved).

Unknown-adapter error message extends to include `openai`:

```
$ singularmem retrieve --adapter wat "query"
singularmem: usage: unknown adapter 'wat'; known adapters: plain, claude, openai
```

Exit code 1 (usage error).

### Library

`crates/singularmem-adapter-openai/src/lib.rs`:

```rust
pub struct OpenAiAdapter;

impl singularmem_retrieve::Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str { "openai" }
    fn format(&self, ctx: &singularmem_retrieve::RetrievedContext) -> String {
        /* algorithm in Recommended Approach */
    }
}
```

No additional public types. No helpers exposed (no `escape_xml`
analogue needed; content is emitted verbatim).

### Wire (MCP / HTTP / IPC)

N/A. Sub-project 4 (MCP server) will surface adapters through MCP;
this sub-project ships only the library + CLI registration.

## Error handling

OpenAiAdapter has no failure modes. `Adapter::format` returns
`String`, not `Result<String, _>`, by trait contract. Any failure
inside formatting would manifest as a `write!` failure into the
in-memory `String`, which `std::fmt::Write` for `String` makes
impossible (only OOM, which panics globally).

Per Principle VII: there is nothing to surface, because there is
nothing that can fail. The CLI's existing error handling (from
sub-project 3a) covers all upstream failures (missing index, missing
query, etc.) before the adapter is invoked.

## Testing strategy

### Unit tests (`crates/singularmem-adapter-openai/src/lib.rs`)

| Test | What it pins down |
|---|---|
| `name_returns_openai` | `OpenAiAdapter.name() == "openai"` |
| `format_includes_citation_instruction_when_non_empty` | Output starts with `"Use the following retrieved memories. Cite by [N] index."` line |
| `format_emits_one_indexed_bracket_markers` | Two-block input → `[1]` and `[2]` on their own block headers; no `[0]` |
| `format_includes_source_on_header_line_when_present` | Block with `Some("claude-conversation:abc-123")` → `[1] source: claude-conversation:abc-123` line |
| `format_omits_source_keyword_when_none` | Block with `source: None` → header is `[N]` exactly (no `source:` substring on that line) |
| `format_includes_tags_when_non_empty` | `tags: vec!["fox", "animals"]` → `tags: fox, animals` line on its own |
| `format_omits_tags_when_empty` | `tags: vec![]` → no `tags:` line in that block |
| `format_separates_blocks_with_blank_line` | Two blocks → blank line between block 1's content and block 2's `[2]` header |
| `format_does_not_emit_trailing_blank_line_after_last_block` | Two blocks → no extra blank line after last block's content |
| `format_empty_context_emits_no_match_line_without_brackets` | Empty `blocks` → `No memories matched for query: "..."\n` exactly; output contains no `[` or `]` characters |
| `format_does_not_include_score_or_id_or_created_at` | Output never includes score, ULID, or `created_at` strings |
| `format_preserves_multiline_content_verbatim` | Content `"line one\nline two"` → both lines preserved in output as-is |

Twelve tests.

### CLI integration test (`tests/cli.rs`)

| Test | Verifies |
|---|---|
| `retrieve_with_openai_adapter_emits_bracket_citations` | After ingest, `singularmem retrieve --adapter openai fox` → exit 0, stdout contains the citation-instruction line + `[1]` marker + full content |

### Updated existing test (`tests/cli.rs`)

`retrieve_unknown_adapter_errors` (last updated in 3b to expect
`known adapters: plain, claude`) extends once more to
`known adapters: plain, claude, openai`. The fake-adapter name
(`nonexistent`) and the unknown-adapter substring assertion stay
unchanged. Single-line edit.

### Perf budget

**No new budget.** OpenAiAdapter is pure string concatenation bounded
by `max_blocks × O(content size)`. Microsecond-scale work. A budget
would be noise-dominated.

### Offline guarantee

Per Principle VI: OpenAiAdapter is a pure function with no I/O. All
tests are unit tests over in-memory `RetrievedContext` values
constructed in the test body — no DB, no network, no file system.
The `tests-offline` advisory CI job picks up the new tests with no
configuration changes.

## Open questions

None at spec time. Two notes for the implementation plan:

1. **`known adapters:` assertion churn (continued).** Each cloud-
   adapter sub-project (3b, 3c, 3d) extends the
   `retrieve_unknown_adapter_errors` test in `tests/cli.rs`. 3c
   extends `plain, claude` to `plain, claude, openai`. 3d will
   extend to `plain, claude, openai, gemini`. Worth flagging in the
   plan task that does this edit.

2. **CLI integration test placement.** The new
   `retrieve_with_openai_adapter_emits_bracket_citations` test belongs
   alongside the other `retrieve_*` tests at the bottom of
   `tests/cli.rs`, not in the new adapter crate. Same call as 3b:
   adapter-crate tests are pure unit tests over `RetrievedContext`
   values; the CLI test exercises the full registry → dispatch →
   formatter path.

## Acceptance criteria

1. New crate `crates/singularmem-adapter-openai/` exists with
   `Cargo.toml` (workspace-version, depends only on
   `singularmem-retrieve` for production) and `src/lib.rs`.
2. `singularmem_adapter_openai::OpenAiAdapter` is a public unit
   struct implementing `singularmem_retrieve::Adapter`.
3. `OpenAiAdapter::name()` returns `"openai"`.
4. `OpenAiAdapter::format(&RetrievedContext)` produces the bracketed-
   citation shape from the Recommended Approach section. Leading
   directive line on non-empty contexts. Same-line `source:` on the
   `[N]` header when present; own-line `tags:` when non-empty;
   blank line; full content. Blocks separated by a single blank
   line; no trailing blank after the last block.
5. Empty `blocks` → `No memories matched for query: "..."\n` exactly;
   output contains no `[` or `]` characters (brackets reserved for
   citation markers).
6. No content escaping; multiline content preserved verbatim.
7. Score, ULID, and `created_at` deliberately omitted from output.
8. Module layout: single `src/lib.rs` (~200 lines including tests).
9. Root binary `Cargo.toml` `[dependencies]` adds
   `singularmem-adapter-openai = { path = "crates/singularmem-adapter-openai" }`.
10. `src/main.rs::known_adapters()` registers
    `Box::new(singularmem_adapter_openai::OpenAiAdapter)` after
    `ClaudeAdapter`; the 3c line-comment placeholder is removed;
    only the 3d placeholder remains.
11. `tests/cli.rs::retrieve_unknown_adapter_errors` extends its
    `known adapters:` substring assertion from `plain, claude` to
    `plain, claude, openai`.
12. All 12 unit tests + 1 new CLI integration test from the Testing
    section pass on `ubuntu-latest` and `macos-latest`.
13. No new perf budget; formatter is microsecond-scale.
14. `docs/formats/store-v1.md` unchanged. `format_version` stays
    `"1"`.
15. Tagged on merge as `v0.7.0` (additive MINOR bump per Principle V).

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. OpenAiAdapter is a pure formatter; no API calls to OpenAI, no auth, no streaming. |
| **II — Provider-Agnostic by Contract** | Third of four required providers. After 3c merges: `plain` + `claude` + `openai` ship; only `gemini` (3d) remains. The "remove an adapter doesn't break non-provider features" property is now testable against three independent removable adapters (each is a colocated three-edit removal). |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (new crate, new registry entry). Nothing removed. III.b: no on-disk changes; `format_version` unchanged. |
| **IV — CLI-First** | `singularmem retrieve --adapter openai <query>` works end-to-end. All eight existing retrieve flags compose with `--adapter openai` unchanged. |
| **V — Composable Library Architecture** | Standalone crate with a single public type. Consumers can `cargo add singularmem-adapter-openai` and use it with their own `Retriever` instance — no CLI dependency. |
| **VI — Deterministic and Offline-Testable** | All tests are unit tests over in-memory `RetrievedContext` values (no DB, no network). Pure-function trait contract preserved. |
| **VII — Honest Failure Modes** | OpenAiAdapter has no failure modes — `format` is infallible by trait contract (`String` return, not `Result`). The CLI's existing 3a error handling covers all upstream failures before the adapter is invoked. |
| **VIII — Privacy Telemetry** | No telemetry added. No logging in the format path. |
| **IX — Accessible by Default** | Plain ASCII output. Citation-marker format is machine-parseable for screen readers + programmatic consumers. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Formatting cost is `max_blocks × O(content size)` string allocations; microseconds in practice. |
