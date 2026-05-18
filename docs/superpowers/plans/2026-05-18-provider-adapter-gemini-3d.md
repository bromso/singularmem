# Provider Adapter — Gemini (Sub-Project 3d) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `GeminiAdapter` as the fourth and final cloud-provider implementation of the typed `Adapter` contract from sub-project 3a. A pure formatter emitting em-dash-separated `Source N` headers with a leading "Use the following sources to ground your answer." directive that aligns with Gemini's Vertex AI grounding-API vocabulary. Registered with the CLI so `singularmem retrieve --adapter gemini <query>` works end-to-end. **Completion milestone:** after merge, all four Principle II required providers ship.

**Architecture:** New crate `crates/singularmem-adapter-gemini/` depends only on `singularmem-retrieve` for production (dev-deps on `singularmem-core`/`singularmem-search`/`jiff` for test fixtures). Single `src/lib.rs` with `GeminiAdapter` unit struct + `Adapter` impl. No XML/escape helper needed — content is verbatim. The CLI's `known_adapters()` registry gains one line replacing its last `// 3X will add:` placeholder; one existing CLI test extends its `known adapters:` substring assertion to its v0 final form.

**Tech Stack:** Rust 1.80, workspace deps only (no new external crates), `singularmem-retrieve` v0.5.0 with the `Adapter` trait + `RetrievedContext` + `MemoryBlock` types from sub-project 3a.

**Spec:** `docs/superpowers/specs/2026-05-18-provider-adapter-gemini-3d-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-adapter-gemini/Cargo.toml` — new crate manifest, version workspace-locked.
- `crates/singularmem-adapter-gemini/src/lib.rs` — `GeminiAdapter` struct, `Adapter` impl, twelve unit tests inline.

**Modified:**
- `Cargo.toml` (workspace root) — add `singularmem-adapter-gemini` to the root binary's `[dependencies]`.
- `src/main.rs:393-399` — `known_adapters()` gains `Box::new(singularmem_adapter_gemini::GeminiAdapter)`; the **last** `// 3X will add:` placeholder is removed. Registry is in its v0 final form.
- `tests/cli.rs:1044` — `retrieve_unknown_adapter_errors` substring assertion tightens from `known adapters: plain, claude, openai` to `known adapters: plain, claude, openai, gemini`. One new test `retrieve_with_gemini_adapter_emits_source_headers` appended.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — adapter is read-time only).

---

## Task 1: Crate scaffold + `GeminiAdapter` struct with `name()`

**Why first:** Every later task imports `GeminiAdapter` or its `Adapter` impl. Landing the scaffold + `name()` first means Task 2 can focus purely on the `format` implementation against a compiling foundation.

**Files:**
- Create: `crates/singularmem-adapter-gemini/Cargo.toml`
- Create: `crates/singularmem-adapter-gemini/src/lib.rs`

- [ ] **Step 1: Create the new-crate Cargo.toml**

Create `crates/singularmem-adapter-gemini/Cargo.toml`:

```toml
[package]
name = "singularmem-adapter-gemini"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Singularmem retrieval adapter for Google Gemini (pure formatter, no HTTP)."

[lints]
workspace = true

[dependencies]
singularmem-retrieve = { path = "../singularmem-retrieve" }

[dev-dependencies]
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }
jiff = { workspace = true }
```

Dev-deps are added up-front (same pattern as 3c) because Task 2's tests will construct `MemoryBlock` literals immediately. No production-side `[features]`.

- [ ] **Step 2: Write the failing test (as part of the seed code)**

Create `crates/singularmem-adapter-gemini/src/lib.rs`:

```rust
//! Singularmem retrieval adapter for Google Gemini.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! em-dash-separated `Source N` headers with a leading directive line
//! that primes Gemini to ground its answer in the listed sources.
//! Per-block header is `Source N:` when both `source` and `tags` are
//! absent, otherwise `Source N — source: ..., tags: ...:` with the
//! metadata comma-joined. Content immediately follows on the next line.
//!
//! The em-dash separator is U+2014 (UTF-8); Rust source files are UTF-8
//! so the literal compiles fine. Project precedent: sub-project 2c's
//! `--show-ranks` output uses the same character.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-gemini-3d-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for Google Gemini. Stateless unit struct.
pub struct GeminiAdapter;

impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
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
    fn name_returns_gemini() {
        assert_eq!(GeminiAdapter.name(), "gemini");
    }
}
```

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The new crate compiles; the stubbed `format` returns empty `String` (Task 2 replaces it).

- [ ] **Step 4: Run the test**

Run: `cargo test -p singularmem-adapter-gemini --lib`
Expected: PASS (`name_returns_gemini`).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-gemini --all-targets -- -D warnings`
Expected: zero warnings.

Watch for `clippy::doc_markdown` on `Gemini`, `Vertex AI`, `UTF-8`, `XML` in the module-level doc-comment — backtick-wrap if flagged.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/singularmem-adapter-gemini/
git commit -s -m "feat(adapter-gemini): new crate scaffold + GeminiAdapter::name

Adds singularmem-adapter-gemini crate depending only on
singularmem-retrieve (production) plus singularmem-core/search/jiff
as dev-deps for test fixtures. GeminiAdapter is a public unit struct
implementing the Adapter trait; format() is stubbed and lands in
Task 2.

Fourth and final cloud-provider adapter for the Principle II surface."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: `format()` implementation + 11 format tests

**Files:**
- Modify: `crates/singularmem-adapter-gemini/src/lib.rs` (replace `format` stub + add 11 tests)

This is the meatiest task. Single function implementation + eleven unit tests covering all the format behaviours from the spec's Testing Strategy section (plus the existing `name_returns_gemini` from Task 1 = 12 total).

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-adapter-gemini/src/lib.rs`:

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
    fn format_includes_grounding_instruction_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.contains("Use the following sources to ground your answer."),
            "missing grounding instruction: {out}"
        );
    }

    #[test]
    fn format_emits_one_indexed_source_headers() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        assert!(
            out.lines().any(|l| l.starts_with("Source 1")),
            "missing Source 1 header: {out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("Source 2")),
            "missing Source 2 header: {out}"
        );
        assert!(
            !out.lines().any(|l| l.starts_with("Source 0")),
            "0-indexed slipped in: {out}"
        );
    }

    #[test]
    fn format_header_with_source_only() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        // Header: "Source 1 — source: claude-conversation:abc-123:"
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert!(
            header_line.contains("— source: claude-conversation:abc-123"),
            "missing source segment: {header_line}"
        );
        assert!(
            !header_line.contains("tags:"),
            "unexpected tags segment in source-only header: {header_line}"
        );
        assert!(
            header_line.ends_with(':'),
            "header should end with colon: {header_line}"
        );
    }

    #[test]
    fn format_header_with_tags_only() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                None,
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert!(
            header_line.contains("— tags: fox, animals"),
            "missing tags segment: {header_line}"
        );
        assert!(
            !header_line.contains("source:"),
            "unexpected source segment in tags-only header: {header_line}"
        );
        assert!(
            header_line.ends_with(':'),
            "header should end with colon: {header_line}"
        );
    }

    #[test]
    fn format_header_with_both_source_and_tags() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        // Exact expected header: "Source 1 — source: claude-conversation:abc-123, tags: fox, animals:"
        assert!(
            out.contains(
                "Source 1 — source: claude-conversation:abc-123, tags: fox, animals:"
            ),
            "missing combined source+tags header: {out}"
        );
    }

    #[test]
    fn format_header_bare_when_no_metadata() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = GeminiAdapter.format(&ctx);
        // Header must be exactly "Source 1:" — no em-dash, no dangling separator.
        let header_line = out
            .lines()
            .find(|l| l.starts_with("Source 1"))
            .expect("Source 1 header should exist");
        assert_eq!(
            header_line, "Source 1:",
            "bare header should be 'Source 1:': got {header_line:?}"
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
        let out = GeminiAdapter.format(&ctx);
        let lines: Vec<&str> = out.lines().collect();
        let idx_2 = lines
            .iter()
            .position(|l| l.starts_with("Source 2"))
            .expect("Source 2 header should exist");
        assert!(idx_2 > 0, "Source 2 should not be the first line");
        assert!(
            lines[idx_2 - 1].is_empty(),
            "expected blank line immediately before Source 2 header; got: {:?}",
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
        let out = GeminiAdapter.format(&ctx);
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
    fn format_empty_context_emits_no_match_line_with_grounding_phrasing() {
        let ctx = sample_context(vec![], "nothing here");
        let out = GeminiAdapter.format(&ctx);
        assert_eq!(
            out,
            "No grounding sources matched for query: \"nothing here\"\n"
        );
        // Vocabulary check: the empty-state message uses 'grounding'
        // consistent with the directive vocabulary.
        assert!(
            out.contains("grounding"),
            "empty state should use grounding vocabulary: {out}"
        );
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
        let out = GeminiAdapter.format(&ctx);
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
        let out = GeminiAdapter.format(&ctx);
        assert!(out.contains("line one"), "missing line one: {out}");
        assert!(out.contains("line two"), "missing line two: {out}");
        assert!(out.contains("line three"), "missing line three: {out}");
    }
```

That's eleven `format_*` tests + the existing `name_returns_gemini` = 12 unit tests total.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-adapter-gemini --lib format_`
Expected: most FAIL because `format` is the Task 1 stub returning `String::new()`.

- [ ] **Step 3: Replace the `format` stub**

In `crates/singularmem-adapter-gemini/src/lib.rs`, replace the entire `impl Adapter for GeminiAdapter` block. The current stub is:

```rust
impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 2 implements this.
        String::new()
    }
}
```

Replace with:

```rust
impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return format!(
                "No grounding sources matched for query: {:?}\n",
                ctx.query
            );
        }
        let mut out = String::new();
        let _ = writeln!(out, "Use the following sources to ground your answer.");
        let _ = writeln!(out);
        for (i, block) in ctx.blocks.iter().enumerate() {
            // Build the per-block header. Four cases:
            //   "Source N:"                                   (no source, no tags)
            //   "Source N — source: X:"                       (source only)
            //   "Source N — tags: a, b:"                      (tags only)
            //   "Source N — source: X, tags: a, b:"           (both)
            let mut parts: Vec<String> = Vec::new();
            if let Some(s) = &block.source {
                parts.push(format!("source: {s}"));
            }
            if !block.tags.is_empty() {
                parts.push(format!("tags: {}", block.tags.join(", ")));
            }
            if parts.is_empty() {
                let _ = writeln!(out, "Source {}:", i + 1);
            } else {
                let _ = writeln!(out, "Source {} — {}:", i + 1, parts.join(", "));
            }
            // Content immediately follows the header on the next line.
            let _ = writeln!(out, "{}", block.content);
            // Blank line between blocks; no trailing blank after the last.
            if i + 1 < ctx.blocks.len() {
                let _ = writeln!(out);
            }
        }
        out
    }
}
```

The `—` character in the writeln format string is U+2014. Rust source files are UTF-8; this compiles directly without escape sequences.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-adapter-gemini --lib`
Expected: PASS for all 12 tests (1 name + 11 format).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-gemini --all-targets --tests -- -D warnings`
Expected: zero warnings.

Watch for:
- `clippy::doc_markdown` on `Gemini`/`Vertex AI`/`UTF-8`/`XML`/etc. — backtick-wrap if flagged.
- `clippy::format_push_string` if the `let _ = writeln!(out, ...)` pattern triggers it — keep the `writeln!` form (matches PlainAdapter + ClaudeAdapter + OpenAiAdapter pattern from sub-projects 3a/3b/3c).
- `clippy::needless_pass_by_value` on `_ctx: &RetrievedContext` — references, fine.

- [ ] **Step 6: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 7: Commit**

```bash
git add crates/singularmem-adapter-gemini/src/lib.rs
git commit -s -m "feat(adapter-gemini): full format() implementation + 11 format tests

Emits em-dash source-header shape: leading 'Use the following sources
to ground your answer.' directive, then 'Source N:' (bare) or
'Source N — source: X, tags: a, b:' (with metadata, source-then-tags
order, comma-joined). Content immediately follows header on next line;
blank-line block separation; no trailing blank after last block.

Empty context emits 'No grounding sources matched for query: \"...\"'
with grounding vocabulary continued from the directive.

Em-dash separator is U+2014 (UTF-8); Rust source files are UTF-8 so
the literal compiles directly. Project precedent: sub-project 2c's
--show-ranks output uses the same character."
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
singularmem-adapter-openai = { path = "crates/singularmem-adapter-openai" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

Add one line after `singularmem-adapter-openai`:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
singularmem-adapter-claude = { path = "crates/singularmem-adapter-claude" }
singularmem-adapter-openai = { path = "crates/singularmem-adapter-openai" }
singularmem-adapter-gemini = { path = "crates/singularmem-adapter-gemini" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

- [ ] **Step 2: Register `GeminiAdapter` in `known_adapters()`** (registry's v0 final form)

In `src/main.rs`, find the `known_adapters` function (currently around line 393):

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

Replace with (note: this is the last `// 3X will add:` placeholder — after this edit, **no `// 3X will add:` markers remain** in the file):

```rust
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        Box::new(singularmem_adapter_claude::ClaudeAdapter),
        Box::new(singularmem_adapter_openai::OpenAiAdapter),
        Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}
```

The registry is now in its v0 final form. All four Principle II required providers ship.

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The CLI binary now links against all four adapter crates.

- [ ] **Step 4: Confirm no `// 3X will add:` placeholders remain**

Run: `grep -n "3. will add" src/main.rs`
Expected: no output (all four placeholders are now real registry entries).

- [ ] **Step 5: Run the full existing CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for all tests, BUT note the same gotcha from 3c: the existing `retrieve_unknown_adapter_errors` test asserts `"known adapters: plain, claude, openai"` which is a substring of the new `"known adapters: plain, claude, openai, gemini"` — so it will still pass at this point. Task 4 tightens that assertion. For now, confirm everything passes:

```bash
cargo test --test cli 2>&1 | grep -E "FAILED|test result" | tail -10
```

Expected: ALL tests pass at this point (no expected failure since the existing substring is still contained in the new output).

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -s -m "feat(cli): register GeminiAdapter in known_adapters (v0 final form)

Adds singularmem-adapter-gemini to root binary deps and inserts
Box::new(singularmem_adapter_gemini::GeminiAdapter) into the
known_adapters() vec! after OpenAiAdapter. This replaces the last
'// 3X will add:' placeholder comment — the registry is now in its
v0 final form with all four Principle II required providers shipping
(plain + claude + openai + gemini).

The existing retrieve_unknown_adapter_errors test still passes
because 'plain, claude, openai' is a substring of the new
'plain, claude, openai, gemini' output. Task 4 tightens that
assertion to actively confirm the registry was extended."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 4: CLI tests — tighten existing + add new

**Files:**
- Modify: `tests/cli.rs:1044` (the `known adapters:` substring assertion in `retrieve_unknown_adapter_errors`)
- Modify: `tests/cli.rs` (append new test at end)

- [ ] **Step 1: Tighten `retrieve_unknown_adapter_errors`'s `known adapters:` assertion**

Find `retrieve_unknown_adapter_errors` in `tests/cli.rs` (currently around line 1022). The current body ends with:

```rust
        .stderr(predicate::str::contains(
            "known adapters: plain, claude, openai",
        ));
}
```

Replace that assertion line with:

```rust
        .stderr(predicate::str::contains(
            "known adapters: plain, claude, openai, gemini",
        ));
}
```

That's the only change to this test — the fake-adapter name (`nonexistent`) stays, the `unknown adapter 'nonexistent'` substring stays. Single-line edit. Same gotcha pattern as 3c — the tighter assertion actively confirms `gemini` is now registered.

**This is the final tightening.** After 3d, the v0 Principle II surface is complete; the assertion stops growing.

- [ ] **Step 2: Add the new CLI integration test**

Append to the bottom of `tests/cli.rs`:

```rust
#[test]
fn retrieve_with_gemini_adapter_emits_source_headers() {
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
            "gemini",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Use the following sources to ground your answer.",
        ))
        .stdout(predicate::str::contains("Source 1"))
        .stdout(predicate::str::contains("the quick brown fox"));
}
```

- [ ] **Step 3: Run the updated + new tests**

Run: `cargo test --test cli retrieve_unknown_adapter_errors retrieve_with_gemini_adapter_emits_source_headers`
Expected: PASS for both.

- [ ] **Step 4: Run the full CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for all tests (42 from prior sub-projects + 1 new = 43 total).

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
git commit -s -m "test(cli): tighten unknown-adapter assertion + add Gemini integration test

Tightens retrieve_unknown_adapter_errors's 'known adapters:' substring
assertion from 'plain, claude, openai' to 'plain, claude, openai, gemini'
so it actively confirms the registry has been extended (the previous
looser assertion would have accidentally passed even without the
registry change since the old string was a substring of the new
output). This is the final tightening — after 3d, the v0 Principle II
surface is complete and the assertion stops growing.

Adds retrieve_with_gemini_adapter_emits_source_headers asserting
end-to-end that 'singularmem retrieve --adapter gemini <query>' emits
the grounding-instruction line, 'Source 1' header, and full content."
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
git commit -s -m "style: rustfmt cleanups after Gemini adapter landing"
```

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 3: Workspace test**

Run: `cargo test --workspace`
Expected: all tests pass. The known pre-existing flake `singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order` may intermittently fail; if so, re-run once to confirm. Do NOT attempt to fix it in this sub-project (out of scope).

- [ ] **Step 4: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean. The new crate's public items (`GeminiAdapter` and its impl methods) all have doc-comments from Task 1's seed code. If rustdoc complains about a missing doc, fix it inline.

- [ ] **Step 5: Cargo.lock status**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications from Task 1 (almost certainly will because a new crate was added), it should already be staged in Task 1's commit. Confirm:

```bash
git diff Cargo.lock
```

If clean, skip. If there's still uncommitted churn:

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after Gemini adapter landing"
```

- [ ] **Step 6: Final placeholder check**

Run: `grep -rn "3. will add" src/ docs/ 2>/dev/null | head -5`

Expected: no `// 3X will add:` placeholders in `src/`. If `docs/` shows hits, those are the historical specs/plans for 3a/3b/3c/3d which legitimately contain `3b will add` / `3c will add` / `3d will add` in their narrative text — that's fine, those are documentation artefacts.

- [ ] **Step 7: Final repository status**

Run: `git status`
Expected: clean working tree (untracked `.agents/`, `.claude/`, `skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -10`
Expected: the new commits from Tasks 1-4 (plus optional Task 5 fmt/lockfile commits) sit on top of `62b2070` (the v0.7.0 version-bump commit from sub-project 3c's wrap-up) and `057220b` (the 3d design spec commit, currently on local main).

---

## Self-review

**1. Spec coverage check** (each spec acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. New crate scaffold + workspace-version + depends on singularmem-retrieve | 1 |
| 2. `GeminiAdapter` is public unit struct implementing `Adapter` | 1 |
| 3. `name()` returns `"gemini"` | 1 |
| 4. `format()` produces em-dash source-header shape with four header cases + leading directive + immediate content + blank-line separation | 2 |
| 5. Em-dash separator is U+2014, not ASCII `--` or `-` | 2 |
| 6. Empty `blocks` → `"No grounding sources matched for query: ..."\n` with "grounding" word | 2 |
| 7. No content escaping; multiline preserved verbatim | 2 |
| 8. Score, ULID, created_at omitted | 2 |
| 9. Module layout: single `src/lib.rs` | 1 |
| 10. Root binary `Cargo.toml` `[dependencies]` adds new crate | 3 |
| 11. `known_adapters()` registers `GeminiAdapter`; **no `// 3X will add:` placeholders remain** | 3 (verified in step 4) |
| 12. `retrieve_unknown_adapter_errors` tightens substring to `plain, claude, openai, gemini` | 4 |
| 13. All 12 unit tests + 1 new CLI test pass | distributed: 1, 2, 4 |
| 14. No new perf budget | (no task — verified by absence) |
| 15. `docs/formats/store-v1.md` unchanged | (no task — verified by absence) |
| 16. Tag `v0.8.0` on merge | (out of plan scope — maintainer's merge ritual) |
| 17. Completion milestone: project memory updated post-merge | (out of plan scope — same merge ritual) |

All seventeen criteria covered. Notable additional check in Task 5 Step 6 confirms the registry is in its v0 final form.

**2. Placeholder scan:** no TBDs, no "implement later" (Task 1's stub is explicitly marked as a Task 2 dependency with full replacement code in Task 2 Step 3), no "similar to Task N". Every task has complete code blocks. Task 5 is verification-only with exact commands + expected outputs.

**3. Type consistency:**
- `GeminiAdapter` (unit struct, no generics) consistent across Tasks 1, 2, 3, 4.
- `name(&self) -> &'static str` consistent across Tasks 1 and 2.
- `format(&self, &RetrievedContext) -> String` consistent across Tasks 1 and 2.
- `known_adapters()` return type `Vec<Box<dyn singularmem_retrieve::Adapter>>` matches the existing 3a signature.
- CLI test substring assertions match the format produced by Task 2's implementation:
  - `"Use the following sources to ground your answer."` directive (Tasks 2 + 4)
  - `Source 1` header (Tasks 2 + 4)
  - `known adapters: plain, claude, openai, gemini` registry list (Task 4 matches Task 3's registry order)
- Em-dash `—` (U+2014) consistent across Task 1 (doc comment), Task 2 (implementation + tests), and spec.

Plan ready for execution.
