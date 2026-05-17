---
title: Provider adapter — Claude (sub-project 3b)
date: 2026-05-18
status: draft
sub-project: 3b-provider-adapter-claude
supersedes: none
---

# Provider adapter — Claude (sub-project 3b)

This sub-project adds the first cloud-provider implementation of the
typed `Adapter` contract Singularmem shipped in sub-project 3a (v0.5.0).
A new crate `singularmem-adapter-claude` exposes `ClaudeAdapter`, a
pure-formatter implementation tailored to Anthropic Claude's preferred
prompt style — element-heavy XML matching the published
prompt-engineering documentation. The CLI's `known_adapters()` registry
gains one line; users invoke it with `singularmem retrieve --adapter
claude <query>`.

## Problem & motivation

The constitution's Principle II requires at-minimum-four provider
integrations (Claude, OpenAI/Codex, Gemini, one local runtime) through
a single typed adapter contract. Sub-project 3a delivered the contract
plus `PlainAdapter` (which doubles as the local-runtime provider). The
three remaining cloud providers are sub-projects 3b/3c/3d, one at a
time, smallest-first.

Claude is first because:

- Anthropic's prompt-engineering format is the most thoroughly
  documented and the most opinionated. Validating the adapter shape
  here de-risks 3c and 3d, which can mostly follow the same template.
- It's the highest-traffic provider for this project's primary user.

Without 3b, the `Adapter` trait has only `PlainAdapter` to test
against, which means the trait's design hasn't been validated by a
real provider with its own format requirements. Landing 3b proves the
contract works for the case it was designed to support, and ratifies
the registry pattern (Cargo.toml dep + `known_adapters()` line) for
3c/3d to follow.

## Goals & non-goals

### Goals

1. New crate `singularmem-adapter-claude` containing `ClaudeAdapter`
   as a pure-formatter `Adapter` implementation.
2. Output shape matches Anthropic's published prompt-engineering
   convention: `<documents>` wrapper, one-indexed `<document
   index="N">` elements, `<source>`/`<tags>` optional sub-elements,
   `<document_content>` body with XML-escaped text.
3. CLI registry integration: `--adapter claude` becomes a valid
   choice; the unknown-adapter error message updates accordingly.
4. CLI integration test confirms `retrieve --adapter claude` produces
   `<documents>` output end-to-end.
5. Workspace version bumps to `v0.6.0` on merge.

### Non-goals

- **No HTTP client, no auth, no streaming.** ClaudeAdapter is a pure
  function. Calling the Anthropic API is the user's job (or a future
  "adapter HTTP helpers" sub-project that does not exist yet).
- **No memory ingest from Claude responses.** Reading direction only,
  per the Adapter trait contract from 3a.
- **No token-budget management.** `max_blocks` caps the input to the
  formatter; the formatter does not measure tokens against any
  Claude context-window limit.
- **No `<context>` outer wrapper, no `<instructions>` block, no
  per-message system-prompt scaffolding.** ClaudeAdapter emits exactly
  the retrieved-documents block. Users compose it into their own
  prompt structure.
- **No support for multiple Claude model variants** with different
  format preferences. Anthropic's `<documents>` convention is
  consistent across all current Claude models; if a future model
  diverges, a `ClaudeOpus5Adapter` (or similar) becomes its own
  registry entry.
- **No CDATA-wrapped content.** Inline XML escaping is strictly
  safe; CDATA isn't (`]]>` in content breaks it). Token-cost
  difference is negligible.
- **No on-disk changes.** Adapter is read-time only.

## Recommended approach

A new crate `crates/singularmem-adapter-claude/` depends only on
`singularmem-retrieve` (for the `Adapter` trait + `RetrievedContext` +
`MemoryBlock` types). `ClaudeAdapter` is a public unit struct.
`name()` returns `"claude"`. `format(&RetrievedContext)` walks the
blocks, emitting:

```
<documents>
<document index="1">
<source>...</source>     # omitted if Item.source is None
<tags>tag1, tag2</tags>  # omitted if Item.tags is empty
<document_content>
<escaped content>
</document_content>
</document>
<document index="2">
...
</document>
</documents>
```

Empty `blocks` → `<documents></documents>\n` (empty container is
unambiguous in XML; no helper comment needed).

XML escaping handles `&`, `<`, `>` (order matters: `&` first to
avoid double-escaping). `"` and `'` are not escaped because user
content never enters attribute values — the only attribute is
`index="<digit>"`.

Module layout is a single `src/lib.rs` (~150 lines including tests).
Splitting into multiple files would be over-engineering for a crate
with one public type and one method.

CLI integration is two edits in two files: one `Cargo.toml` dep line
in the root binary, one line in `src/main.rs::known_adapters()`. The
existing `tests/cli.rs::retrieve_unknown_adapter_errors` test from
sub-project 3a needs an update: change its assertion to use a
different fake adapter name and extend the `known adapters:` substring
assertion to include `claude`.

### Approaches discarded

- **Approach B — attribute-heavy compact XML** (e.g., `<document
  index="1" source="..." tags="...">content</document>`). Rejected
  because (i) it's slightly less idiomatic XML, (ii) commas-in-tags
  become CSV-encoding-ambiguity inside an attribute, (iii) the
  ~30% token saving is meaningless inside Claude's context window.
  Anthropic's published examples use the element-heavy shape.

- **Approach C — Markdown wrapped in `<context>` tag.** Rejected
  because it ignores the most-trained prompt convention. Claude's
  training distribution is heaviest on `<documents>`-style XML for
  retrieved content; deviating from it sacrifices recall for stylistic
  preference.

- **Approach D — CDATA-wrap content** instead of inline escaping.
  Rejected because CDATA can't contain the literal string `]]>`,
  making it almost-but-not-actually-leak-proof for arbitrary user
  input. Inline escaping is strictly safe.

- **Approach E — split into `lib.rs` + `adapter.rs` + `escape.rs`
  modules.** Rejected because the crate has one public type, one
  formatter method, and one helper function. Three files for ~150
  lines is structural ceremony with no payoff.

- **Approach F — bundle all three cloud adapters (3b/3c/3d) into one
  sub-project.** Rejected at the 3a brainstorm; same rationale
  applies. Each adapter sub-project is small (~5-7 tasks), each is
  independently reviewable, each can take user feedback before the
  next begins.

## Architecture

Components:

- **`crates/singularmem-adapter-claude/`** — new crate, version
  `0.6.0` (workspace-locked). Single file under `src/`.
- **`crates/singularmem-adapter-claude/Cargo.toml`** — minimal:
  `name`, workspace inheritance for version/edition/etc., `[lints]
  workspace = true`, single dep on `singularmem-retrieve`. No
  `[features]`. No dev-deps (tests are inline in `lib.rs` with
  `#[cfg(test)]`).
- **`crates/singularmem-adapter-claude/src/lib.rs`** — module docs,
  `ClaudeAdapter` unit struct, `Adapter` impl, private
  `escape_xml(s: &str) -> String` helper, `#[cfg(test)] mod tests`
  with nine unit tests.
- **Root binary `Cargo.toml`** — one new `[dependencies]` line.
- **Root binary `src/main.rs::known_adapters()`** — one new `vec!`
  entry; sub-project 3b's placeholder line-comment is removed; 3c/3d
  placeholders remain.
- **`tests/cli.rs`** — one new integration test, one updated existing
  test.

The layering established by 3a remains: `core ← search ← retrieve ←
adapters`. ClaudeAdapter depends on `singularmem-retrieve` only.
ClaudeAdapter does not depend on `singularmem-core` or
`singularmem-search` directly — every type it touches
(`RetrievedContext`, `MemoryBlock`, `Adapter` trait) is re-exported
from `singularmem-retrieve`.

## Data model

**No changes.** ClaudeAdapter consumes `RetrievedContext` /
`MemoryBlock` from sub-project 3a unchanged. No persistent data,
no on-disk artefacts, no `format_version` bump.

## Interfaces

### CLI

No new flags. `singularmem retrieve` from sub-project 3a gains
`claude` as a valid `--adapter` value. All other flags
(`--limit`, `--min-score`, `--mode`, `--fetch-multiplier`, `--rrf-k`,
`--json`, `--show-elapsed`) compose with `--adapter claude` unchanged.

End-to-end example:

```
$ singularmem retrieve --adapter claude "auth migration"
<documents>
<document index="1">
<source>claude-conversation:abc-123</source>
<tags>auth, decision</tags>
<document_content>
We decided to use Argon2id for password hashing because...
</document_content>
</document>
<document index="2">
<document_content>
Migration plan deadline pushed to next sprint.
</document_content>
</document>
</documents>
```

When the user passes `--json`, the adapter is bypassed entirely (3a
behaviour preserved): `RetrievedContext` serialises via serde, and
`--adapter` is irrelevant.

Unknown-adapter error message updates to include `claude`:

```
$ singularmem retrieve --adapter wat "query"
singularmem: usage: unknown adapter 'wat'; known adapters: plain, claude
```

Exit code 1 (usage error).

### Library

`crates/singularmem-adapter-claude/src/lib.rs`:

```rust
pub struct ClaudeAdapter;

impl singularmem_retrieve::Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str { "claude" }
    fn format(&self, ctx: &singularmem_retrieve::RetrievedContext) -> String {
        /* algorithm in Recommended Approach */
    }
}
```

`escape_xml` is private to the crate. No additional public types.

### Wire (MCP / HTTP / IPC)

N/A. Sub-project 4 (MCP server) will surface adapters through MCP;
this sub-project ships only the library + CLI registration.

## Error handling

ClaudeAdapter has no failure modes. `Adapter::format` returns
`String`, not `Result<String, _>`, by trait contract. Any failure
inside formatting would manifest as a `write!` failure into the
in-memory `String`, which `std::fmt::Write` for `String` makes
impossible (only OOM, which panics globally).

Per Principle VII: there is nothing to surface, because there is
nothing that can fail. The Constitution Check section addresses this
explicitly.

The CLI's existing error handling (from sub-project 3a) covers all
upstream failures (missing index, missing query, etc.) before the
adapter is invoked.

## Testing strategy

### Unit tests (`crates/singularmem-adapter-claude/src/lib.rs`)

| Test | What it pins down |
|---|---|
| `name_returns_claude` | `ClaudeAdapter.name() == "claude"` |
| `format_wraps_in_documents_element` | Output begins with `<documents>` line and ends with `</documents>` line |
| `format_uses_one_indexed_document_indices` | Two-block input → `<document index="1">` and `<document index="2">` present; no `index="0"` |
| `format_includes_source_when_present` | Block with `Some("claude-conversation:abc-123")` → `<source>claude-conversation:abc-123</source>` line in that document |
| `format_omits_source_when_none` | Block with `source: None` → NO `<source>` element in that document |
| `format_includes_tags_when_non_empty` | `tags: vec!["fox", "animals"]` → `<tags>fox, animals</tags>` line present |
| `format_omits_tags_when_empty` | `tags: vec![]` → NO `<tags>` element |
| `format_escapes_xml_special_chars_in_content` | Content `<script>foo & "bar"</script>` → output contains `&lt;script&gt;foo &amp; "bar"&lt;/script&gt;`; no raw `<script>` substring |
| `format_empty_context_emits_empty_documents` | `blocks: vec![]` → `<documents></documents>\n` exactly; no `<document>` element |

Nine tests.

### CLI integration test (`tests/cli.rs`)

| Test | Verifies |
|---|---|
| `retrieve_with_claude_adapter_emits_xml_documents` | After ingest, `singularmem retrieve --adapter claude fox` → exit 0, stdout contains `<documents>`, `<document index="1">`, `<document_content>`, full content, `</document>`, `</documents>` |

### Updated existing test (`tests/cli.rs`)

`retrieve_unknown_adapter_errors` (from sub-project 3a) currently
asserts `--adapter claude` is unknown. After 3b lands, `claude`
becomes valid. Change to:
- Unknown adapter name: `nonexistent` (or any other obviously-fake string)
- Expected stderr substring: `known adapters: plain, claude` (was: `known adapters: plain`)

Sub-projects 3c and 3d will each extend this assertion further
(`plain, claude, openai`, then `plain, claude, openai, gemini`).
The plan should call this pattern out so each implementer doesn't
miss it.

### Perf budget

**No new budget.** ClaudeAdapter is pure string concatenation
bounded by `max_blocks × O(content size)`. Microsecond-scale work.
A budget would be noise-dominated.

### Offline guarantee

Per Principle VI: ClaudeAdapter is a pure function with no I/O. All
tests are unit tests over in-memory `RetrievedContext` values
constructed in the test body — no DB, no network, no file system.
The `tests-offline` advisory CI job picks up the new tests with no
configuration changes.

## Open questions

None at spec time. Two notes for the implementation plan:

1. **`known adapters:` assertion churn.** Each cloud-adapter
   sub-project (3b, 3c, 3d) needs to update the
   `retrieve_unknown_adapter_errors` test in `tests/cli.rs`. The plan
   should call this out explicitly in the relevant task. Skipping it
   = test failure.

2. **CLI integration test placement.** The new
   `retrieve_with_claude_adapter_emits_xml_documents` test belongs
   alongside the other `retrieve_*` tests at the bottom of
   `tests/cli.rs`, not in the new adapter crate. Adapter-crate tests
   are pure unit tests over `RetrievedContext` values; the CLI test
   exercises the full registry → dispatch → formatter path and lives
   with the other end-to-end CLI integration tests.

## Acceptance criteria

1. New crate `crates/singularmem-adapter-claude/` exists with
   `Cargo.toml` (workspace-version, depends only on
   `singularmem-retrieve`) and `src/lib.rs`.
2. `singularmem_adapter_claude::ClaudeAdapter` is a public unit
   struct implementing `singularmem_retrieve::Adapter`.
3. `ClaudeAdapter::name()` returns `"claude"`.
4. `ClaudeAdapter::format(&RetrievedContext)` produces the XML shape
   from the Recommended Approach section. Empty `blocks` →
   `<documents></documents>\n` exactly.
5. XML escaping handles `&`, `<`, `>` in source/tags/content. `&` is
   replaced first (no double-escape). `"` and `'` are not escaped
   (no user content in attribute values).
6. Module layout: single `src/lib.rs` (~150 lines including tests).
7. Root binary `Cargo.toml` `[dependencies]` adds
   `singularmem-adapter-claude = { path = "crates/singularmem-adapter-claude" }`.
8. `src/main.rs::known_adapters()` registers
   `Box::new(singularmem_adapter_claude::ClaudeAdapter)` after
   `PlainAdapter`; the 3b line-comment placeholder is removed; 3c
   and 3d placeholders remain.
9. `tests/cli.rs::retrieve_unknown_adapter_errors` updated: unknown
   adapter name changed away from `claude`; `known adapters:`
   assertion updated to include `claude`.
10. All 9 unit tests + 1 new CLI integration test from the Testing
    section pass on `ubuntu-latest` and `macos-latest`.
11. No new perf budget; formatter is microsecond-scale.
12. `docs/formats/store-v1.md` unchanged. `format_version` stays
    `"1"`.
13. Tagged on merge as `v0.6.0` (additive MINOR bump per Principle V).

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. ClaudeAdapter is a pure formatter; no API calls to Anthropic, no auth, no streaming. |
| **II — Provider-Agnostic by Contract** | Implements the typed `Adapter` contract from 3a verbatim. Removing this adapter is a three-edit change (Cargo.toml dep, registry line, one CLI test assertion) — colocated and obvious. Two of the four required providers (Plain = local-runtime, Claude) now ship; 3c (OpenAI/Codex) and 3d (Gemini) remain. |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (new crate, new registry entry). Nothing removed. III.b: no on-disk changes; `format_version` unchanged. |
| **IV — CLI-First** | `singularmem retrieve --adapter claude <query>` works end-to-end. All eight existing retrieve flags compose with `--adapter claude` unchanged. |
| **V — Composable Library Architecture** | Standalone crate with a single public type. Consumers can `cargo add singularmem-adapter-claude` and use it with their own `Retriever` instance — no CLI dependency. Single-file module layout matches the "one clear purpose" principle. |
| **VI — Deterministic and Offline-Testable** | All tests are unit tests over in-memory `RetrievedContext` values (no DB, no network). Pure-function trait contract is preserved. |
| **VII — Honest Failure Modes** | ClaudeAdapter has no failure modes — `format` is infallible by trait contract (`String` return, not `Result`). The CLI's existing 3a error handling covers all upstream failures before the adapter is invoked. |
| **VIII — Privacy Telemetry** | No telemetry added. No logging in the format path. |
| **IX — Accessible by Default** | XML output is plain ASCII (all special chars escaped). Programmatic consumers can parse with any XML library. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Formatting cost is `max_blocks × O(content size)` string allocations; microseconds in practice. |
