# Provider Adapter — OpenAI/Codex (Sub-Project 3c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `OpenAiAdapter` as the second cloud-provider implementation of the typed `Adapter` contract from sub-project 3a. A pure formatter emitting bracketed-citation markers (`[N]`) with a leading directive line that primes GPT-family models to cite by index. Registered with the CLI so `singularmem retrieve --adapter openai <query>` works end-to-end.

**Architecture:** New crate `crates/singularmem-adapter-openai/` depends only on `singularmem-retrieve` for production (dev-deps on `singularmem-core`/`singularmem-search`/`jiff` for test fixtures). Single `src/lib.rs` with the `OpenAiAdapter` unit struct + `Adapter` impl. No XML/escape helper needed — content is emitted verbatim. The CLI's `known_adapters()` registry gains one line; one existing CLI test extends its `known adapters:` substring assertion.

**Tech Stack:** Rust 1.80, workspace deps only (no new external crates), `singularmem-retrieve` v0.5.0 with the `Adapter` trait + `RetrievedContext` + `MemoryBlock` types from sub-project 3a.

**Spec:** `docs/superpowers/specs/2026-05-18-provider-adapter-openai-3c-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-adapter-openai/Cargo.toml` — new crate manifest, version workspace-locked.
- `crates/singularmem-adapter-openai/src/lib.rs` — `OpenAiAdapter` struct, `Adapter` impl, twelve unit tests inline.

**Modified:**
- `Cargo.toml` (workspace root) — add `singularmem-adapter-openai` to the root binary's `[dependencies]`.
- `src/main.rs:393-399` — `known_adapters()` gains `Box::new(singularmem_adapter_openai::OpenAiAdapter)`; the 3c line-comment placeholder is removed (3d placeholder remains).
- `tests/cli.rs:1043` — `retrieve_unknown_adapter_errors` substring assertion extends from `known adapters: plain, claude` to `known adapters: plain, claude, openai`. One new test `retrieve_with_openai_adapter_emits_bracket_citations` appended.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — adapter is read-time only).

---

## Task 1: Crate scaffold + `OpenAiAdapter` struct with `name()`

**Why first:** Every later task imports `OpenAiAdapter` or its `Adapter` impl. Landing the scaffold + `name()` first means Task 2 can focus purely on the `format` implementation against a compiling foundation.

**Files:**
- Create: `crates/singularmem-adapter-openai/Cargo.toml`
- Create: `crates/singularmem-adapter-openai/src/lib.rs`

- [ ] **Step 1: Create the new-crate Cargo.toml**

Create `crates/singularmem-adapter-openai/Cargo.toml`:

```toml
[package]
name = "singularmem-adapter-openai"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Singularmem retrieval adapter for OpenAI/Codex (pure formatter, no HTTP)."

[lints]
workspace = true

[dependencies]
singularmem-retrieve = { path = "../singularmem-retrieve" }

[dev-dependencies]
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }
jiff = { workspace = true }
```

The dev-deps are needed up-front (unlike 3b which added them in Task 3) because Task 2's tests will construct `MemoryBlock` literals immediately. No production-side `[features]`.

- [ ] **Step 2: Write the failing test (as part of the seed code)**

Create `crates/singularmem-adapter-openai/src/lib.rs`:

```rust
//! Singularmem retrieval adapter for OpenAI/Codex.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! bracketed citation markers (`[N]`) with a leading directive line
//! that primes GPT-family models to cite back by index. Same-line
//! `source:` in the `[N]` header when present; own-line `tags:` below;
//! blank line; full content emitted verbatim (no escaping).
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-openai-3c-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for OpenAI / OpenAI Codex. Stateless unit struct.
pub struct OpenAiAdapter;

impl Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 2 implements this.
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_returns_openai() {
        assert_eq!(OpenAiAdapter.name(), "openai");
    }
}
```

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The new crate compiles; the stubbed `format` returns empty `String` (Task 2 replaces it).

- [ ] **Step 4: Run the test**

Run: `cargo test -p singularmem-adapter-openai --lib`
Expected: PASS (`name_returns_openai`).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-openai --all-targets -- -D warnings`
Expected: zero warnings.

Watch for `clippy::doc_markdown` on `XML`, `GPT`, `Codex`, `OpenAI` in the module-level doc-comment — backtick-wrap if flagged. The seed code uses `[\`singularmem_retrieve::Adapter\`]` which is already wrapped.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/singularmem-adapter-openai/
git commit -s -m "feat(adapter-openai): new crate scaffold + OpenAiAdapter::name

Adds singularmem-adapter-openai crate depending only on
singularmem-retrieve (production) plus singularmem-core/search/jiff
as dev-deps for test fixtures. OpenAiAdapter is a public unit struct
implementing the Adapter trait; format() is stubbed and lands in
Task 2."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: `format()` implementation + 12 unit tests

**Files:**
- Modify: `crates/singularmem-adapter-openai/src/lib.rs` (replace `format` stub + add 12 tests)

This is the meatiest task. Single function implementation + twelve unit tests covering all the format behaviours from the spec's Testing Strategy section.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-adapter-openai/src/lib.rs`:

```rust
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_retrieve::MemoryBlock;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    fn sample_block(id_str: &str, source: Option<&str>, tags: Vec<&str>) -> MemoryBlock {
        MemoryBlock {
            id: ItemId::from_str(id_str).unwrap(),
            content: "the quick brown fox jumps over the lazy dog".to_string(),
            score: 0.5,
            score_kind: ScoreKind::Rrf,
            source: source.map(String::from),
            tags: tags.into_iter().map(String::from).collect(),
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        }
    }

    fn sample_context(blocks: Vec<MemoryBlock>, query: &str) -> RetrievedContext {
        RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 0,
        }
    }

    #[test]
    fn format_includes_citation_instruction_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.contains("Use the following retrieved memories. Cite by [N] index."),
            "missing citation instruction: {out}"
        );
    }

    #[test]
    fn format_emits_one_indexed_bracket_markers() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        // [1] appears on its own line (header), as does [2]; [0] never appears.
        assert!(
            out.lines().any(|l| l.starts_with("[1]")),
            "missing [1] header: {out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("[2]")),
            "missing [2] header: {out}"
        );
        assert!(
            !out.lines().any(|l| l.starts_with("[0]")),
            "0-indexed slipped in: {out}"
        );
    }

    #[test]
    fn format_includes_source_on_header_line_when_present() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.contains("[1] source: claude-conversation:abc-123"),
            "missing source on header: {out}"
        );
    }

    #[test]
    fn format_omits_source_keyword_when_none() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        // The [1] header line must NOT contain "source:" when block.source is None.
        let header_line = out
            .lines()
            .find(|l| l.starts_with("[1]"))
            .expect("[1] header should exist");
        assert!(
            !header_line.contains("source:"),
            "unexpected source on bare header: {header_line}"
        );
    }

    #[test]
    fn format_includes_tags_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                None,
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            out.lines().any(|l| l == "tags: fox, animals"),
            "missing tags line: {out}"
        );
    }

    #[test]
    fn format_omits_tags_when_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        assert!(
            !out.lines().any(|l| l.starts_with("tags:")),
            "unexpected tags line: {out}"
        );
    }

    #[test]
    fn format_separates_blocks_with_blank_line() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        // Find the [2] header; the line immediately before it must be blank.
        let lines: Vec<&str> = out.lines().collect();
        let idx_2 = lines
            .iter()
            .position(|l| l.starts_with("[2]"))
            .expect("[2] header should exist");
        assert!(idx_2 > 0, "[2] should not be the first line");
        assert!(
            lines[idx_2 - 1].is_empty(),
            "expected blank line immediately before [2] header; got: {:?}",
            lines[idx_2 - 1]
        );
    }

    #[test]
    fn format_does_not_emit_trailing_blank_line_after_last_block() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        // Output ends with the last block's content followed by a single
        // newline — NOT a blank line after that.
        assert!(
            out.ends_with("lazy dog\n"),
            "expected trailing 'lazy dog\\n' but got: {out:?}"
        );
        assert!(
            !out.ends_with("\n\n"),
            "trailing blank line after last block: {out:?}"
        );
    }

    #[test]
    fn format_empty_context_emits_no_match_line_without_brackets() {
        let ctx = sample_context(vec![], "nothing here");
        let out = OpenAiAdapter.format(&ctx);
        assert_eq!(out, "No memories matched for query: \"nothing here\"\n");
        // Crucial property: brackets are reserved for citation markers.
        assert!(!out.contains('['), "[ leaked into empty-state output: {out}");
        assert!(!out.contains(']'), "] leaked into empty-state output: {out}");
    }

    #[test]
    fn format_does_not_include_score_or_id_or_created_at() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("some-source"),
                vec!["t1"],
            )],
            "fox",
        );
        let out = OpenAiAdapter.format(&ctx);
        // Score (0.5 → "0.5" or "0.5000"), ID (the ULID), created_at
        // (2026-05-12T14:30:00Z), none should appear.
        assert!(
            !out.contains("0.5000"),
            "score appeared in output: {out}"
        );
        assert!(
            !out.contains("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "ULID appeared in output: {out}"
        );
        assert!(
            !out.contains("2026-05-12"),
            "created_at appeared in output: {out}"
        );
    }

    #[test]
    fn format_preserves_multiline_content_verbatim() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]);
        block.content = "line one\nline two\nline three".to_string();
        let ctx = sample_context(vec![block], "x");
        let out = OpenAiAdapter.format(&ctx);
        assert!(out.contains("line one"), "missing line one: {out}");
        assert!(out.contains("line two"), "missing line two: {out}");
        assert!(out.contains("line three"), "missing line three: {out}");
    }

    #[test]
    fn format_full_block_has_blank_line_between_metadata_and_content() {
        // Header lines (`[1]`, `tags:`) and the content body are separated
        // by exactly one blank line. This is the canonical OpenAI Cookbook
        // RAG shape.
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("src"),
                vec!["t1"],
            )],
            "x",
        );
        let out = OpenAiAdapter.format(&ctx);
        let lines: Vec<&str> = out.lines().collect();
        // Find the content line ("the quick brown fox...") and verify
        // the line before it is blank.
        let content_idx = lines
            .iter()
            .position(|l| l.starts_with("the quick brown fox"))
            .expect("content line should exist");
        assert!(content_idx > 0, "content should not be the first line");
        assert!(
            lines[content_idx - 1].is_empty(),
            "expected blank line before content; got: {:?}",
            lines[content_idx - 1]
        );
    }
```

That's eleven `format_*` tests + the existing `name_returns_openai` = 12 unit tests total.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-adapter-openai --lib format_`
Expected: most FAIL because `format` is the Task 1 stub returning `String::new()`.

- [ ] **Step 3: Replace the `format` stub**

In `crates/singularmem-adapter-openai/src/lib.rs`, replace the entire `impl Adapter for OpenAiAdapter` block. The current stub is:

```rust
impl Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 2 implements this.
        String::new()
    }
}
```

Replace with:

```rust
impl Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!("No memories matched for query: {:?}\n", ctx.query);
        }
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Use the following retrieved memories. Cite by [N] index."
        );
        let _ = writeln!(out);
        for (i, block) in ctx.blocks.iter().enumerate() {
            // Per-block header: bracket marker + optional source on same line.
            if let Some(source) = &block.source {
                let _ = writeln!(out, "[{}] source: {}", i + 1, source);
            } else {
                let _ = writeln!(out, "[{}]", i + 1);
            }
            if !block.tags.is_empty() {
                let _ = writeln!(out, "tags: {}", block.tags.join(", "));
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", block.content);
            // Separator between blocks: blank line. No trailing blank
            // after the last block.
            if i + 1 < ctx.blocks.len() {
                let _ = writeln!(out);
            }
        }
        out
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-adapter-openai --lib`
Expected: PASS for all 12 tests (1 name + 11 format).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-openai --all-targets --tests -- -D warnings`
Expected: zero warnings.

Watch for:
- `clippy::doc_markdown` on `XML`/`GPT`/`OpenAI`/etc. in doc-comments — backtick-wrap if flagged.
- `clippy::format_push_string` if the `let _ = writeln!(out, ...)` pattern triggers it — keep the `writeln!` form (matches PlainAdapter + ClaudeAdapter pattern from sub-projects 3a/3b).
- `clippy::needless_pass_by_value` on `_ctx: &RetrievedContext` — references, fine.

- [ ] **Step 6: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 7: Commit**

```bash
git add crates/singularmem-adapter-openai/src/lib.rs
git commit -s -m "feat(adapter-openai): full format() implementation + 11 format tests

Emits bracketed-citation marker shape: leading directive line, 1-indexed
[N] headers with optional same-line 'source:' metadata, optional
own-line 'tags:' line, blank line, full content emitted verbatim,
blank-line block separation (no trailing blank). Empty context emits
'No memories matched for query: \"...\"' with no brackets (reserved
for citation markers)."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 3: Root binary integration

**Files:**
- Modify: `Cargo.toml` (workspace root — add to root binary's `[dependencies]`)
- Modify: `src/main.rs:393-399` (or wherever `known_adapters()` lives — look for `fn known_adapters()`)

- [ ] **Step 1: Add the crate as a runtime dep**

Modify the workspace root `Cargo.toml`. The current `[dependencies]` section includes:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
singularmem-adapter-claude = { path = "crates/singularmem-adapter-claude" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

Add one line after `singularmem-adapter-claude`:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
singularmem-adapter-claude = { path = "crates/singularmem-adapter-claude" }
singularmem-adapter-openai = { path = "crates/singularmem-adapter-openai" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

- [ ] **Step 2: Register `OpenAiAdapter` in `known_adapters()`**

In `src/main.rs`, find the `known_adapters` function (currently around line 393):

```rust
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        Box::new(singularmem_adapter_claude::ClaudeAdapter),
        // 3c will add: Box::new(singularmem_adapter_openai::OpenAiAdapter),
        // 3d will add: Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}
```

Replace with:

```rust
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        Box::new(singularmem_adapter_claude::ClaudeAdapter),
        Box::new(singularmem_adapter_openai::OpenAiAdapter),
        // 3d will add: Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}
```

(The 3c line-comment placeholder becomes a real `Box::new` entry; only the 3d placeholder remains.)

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The CLI binary now links against the new crate.

- [ ] **Step 4: Run the full existing CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for almost all tests, but `retrieve_unknown_adapter_errors` will FAIL because it asserts `known adapters: plain, claude` and the registry now produces `plain, claude, openai`. Task 4 fixes that test. For now, confirm the failure is exactly that and nothing else:

```bash
cargo test --test cli 2>&1 | grep -E "FAILED|test result" | tail -10
```

Expected: exactly one test failure named `retrieve_unknown_adapter_errors`. The rest of the suite (40+ tests) passes.

If MORE than one CLI test fails, something else broke — investigate before continuing.

- [ ] **Step 5: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -s -m "feat(cli): register OpenAiAdapter in known_adapters

Adds singularmem-adapter-openai to root binary deps and inserts
Box::new(singularmem_adapter_openai::OpenAiAdapter) into the
known_adapters() vec! after ClaudeAdapter. The 3c placeholder
comment is removed; only the 3d placeholder remains.

Knowingly breaks the retrieve_unknown_adapter_errors test from
sub-projects 3a/3b — its 'known adapters: plain, claude'
substring assertion no longer matches. Task 4 extends it."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 4: CLI tests — extend existing + add new

**Files:**
- Modify: `tests/cli.rs:1043` (the `known adapters:` substring assertion in `retrieve_unknown_adapter_errors`)
- Modify: `tests/cli.rs` (append new test at end)

- [ ] **Step 1: Extend `retrieve_unknown_adapter_errors`'s `known adapters:` assertion**

Find `retrieve_unknown_adapter_errors` in `tests/cli.rs` (currently around line 1022). The current body ends with:

```rust
        .stderr(predicate::str::contains("known adapters: plain, claude"));
}
```

Replace that single line with:

```rust
        .stderr(predicate::str::contains("known adapters: plain, claude, openai"));
}
```

That's the only change to this test — the fake-adapter name (`nonexistent`) stays, the `unknown adapter 'nonexistent'` substring stays. Single-line edit.

- [ ] **Step 2: Add the new CLI integration test**

Append to the bottom of `tests/cli.rs`:

```rust
#[test]
fn retrieve_with_openai_adapter_emits_bracket_citations() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "ingest",
            "--content",
            "the quick brown fox jumps",
        ])
        .assert()
        .success();
    std::thread::sleep(std::time::Duration::from_millis(200));

    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "openai",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Use the following retrieved memories. Cite by [N] index.",
        ))
        .stdout(predicate::str::contains("[1]"))
        .stdout(predicate::str::contains("the quick brown fox"));
}
```

- [ ] **Step 3: Run the updated + new tests**

Run: `cargo test --test cli retrieve_unknown_adapter_errors retrieve_with_openai_adapter_emits_bracket_citations`
Expected: PASS for both.

- [ ] **Step 4: Run the full CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for all tests (41 from prior sub-projects + 1 new = 42 total).

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Verify fmt clean**

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 8: Commit**

```bash
git add tests/cli.rs
git commit -s -m "test(cli): extend unknown-adapter test + add OpenAI integration test

Extends retrieve_unknown_adapter_errors's 'known adapters:' substring
assertion from 'plain, claude' to 'plain, claude, openai' to match the
new registry.

Adds retrieve_with_openai_adapter_emits_bracket_citations asserting
end-to-end that 'singularmem retrieve --adapter openai <query>' emits
the citation-instruction line, '[1]' marker, and full content."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 5: Final workspace gate

This task is a verification-only checkpoint. No source changes unless something below fails.

- [ ] **Step 1: Workspace fmt check**

Run: `cargo fmt --check`
Expected: clean. If not, `cargo fmt` and commit separately:

```bash
git add -p .
git commit -s -m "style: rustfmt cleanups after OpenAI adapter landing"
```

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 3: Workspace test**

Run: `cargo test --workspace`
Expected: all tests pass. The known pre-existing flake `singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order` may intermittently fail; if so, re-run once to confirm. Do NOT attempt to fix it in this sub-project (out of scope).

- [ ] **Step 4: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean. The new crate's public items (`OpenAiAdapter` and its impl methods) all have doc-comments from Task 1's seed code. If rustdoc complains about a missing doc, fix it inline.

- [ ] **Step 5: Cargo.lock status**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications from Task 1 (almost certainly will because a new crate was added), it should already be staged in Task 1's commit. Confirm:

```bash
git diff Cargo.lock
```

If clean, skip. If there's still uncommitted churn:

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after OpenAI adapter landing"
```

- [ ] **Step 6: Final repository status**

Run: `git status`
Expected: clean working tree (untracked `.agents/`, `.claude/`, `skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -10`
Expected: the new commits from Tasks 1-4 (plus optional Task 5 fmt/lockfile commits) sit on top of `dd7dc22` (the v0.6.0 version-bump commit from sub-project 3b's wrap-up) and `49c953e` (the 3c design spec commit, currently on local main).

---

## Self-review

**1. Spec coverage check** (each spec acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. New crate scaffold + workspace-version + depends on singularmem-retrieve | 1 |
| 2. `OpenAiAdapter` is public unit struct implementing `Adapter` | 1 |
| 3. `name()` returns `"openai"` | 1 |
| 4. `format()` produces the spec'd shape with leading directive + `[N]` markers + same-line source + own-line tags + blank line + content + blank-line separation, no trailing blank | 2 |
| 5. Empty `blocks` → `No memories matched for query: "..."\n` with no `[`/`]` | 2 |
| 6. No content escaping; multiline preserved | 2 |
| 7. Score, ULID, created_at omitted | 2 |
| 8. Module layout: single `src/lib.rs` | 1 |
| 9. Root binary `Cargo.toml` `[dependencies]` adds new crate | 3 |
| 10. `known_adapters()` registers `OpenAiAdapter`; 3c placeholder removed; 3d remains | 3 |
| 11. `retrieve_unknown_adapter_errors` substring extended to `plain, claude, openai` | 4 |
| 12. All 12 unit tests + 1 new CLI test pass | distributed: 1, 2, 4 |
| 13. No new perf budget | (no task — verified by absence) |
| 14. `docs/formats/store-v1.md` unchanged | (no task — verified by absence) |
| 15. Tag `v0.7.0` on merge | (out of plan scope — maintainer's merge ritual) |

All fifteen criteria covered.

**2. Placeholder scan:** no TBDs, no "implement later" (Task 1's stub is explicitly marked as a Task 2 dependency with full replacement code in Task 2 Step 3), no "similar to Task N". Every task has complete code blocks. Task 5 is verification-only with exact commands + expected outputs.

**3. Type consistency:**
- `OpenAiAdapter` (unit struct, no generics) consistent across Tasks 1, 2, 3, 4.
- `name(&self) -> &'static str` consistent across Tasks 1 and 2.
- `format(&self, &RetrievedContext) -> String` consistent across Tasks 1 and 2.
- `known_adapters()` return type `Vec<Box<dyn singularmem_retrieve::Adapter>>` matches the existing 3a signature.
- CLI test substring assertions match the format produced by Task 2's implementation:
  - `"Use the following retrieved memories. Cite by [N] index."` directive (Tasks 2 + 4)
  - `[1]` bracket marker (Tasks 2 + 4)
  - `known adapters: plain, claude, openai` registry list (Task 4 matches Task 3's registry order)

Plan ready for execution.
