# Provider Adapter — Claude (Sub-Project 3b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `ClaudeAdapter` as the first cloud-provider implementation of the typed `Adapter` contract from sub-project 3a. A pure formatter that emits Anthropic's `<documents><document index="N">...</document></documents>` XML shape, registered with the CLI so `singularmem retrieve --adapter claude <query>` works end-to-end.

**Architecture:** New crate `crates/singularmem-adapter-claude/` depends only on `singularmem-retrieve`. Single `src/lib.rs` contains the `ClaudeAdapter` unit struct, the `Adapter` impl, and a private `escape_xml` helper. The CLI's `known_adapters()` registry gains one line; one existing CLI test updates to keep its unknown-adapter assertion valid.

**Tech Stack:** Rust 1.80, workspace deps only (no new external crates), `singularmem-retrieve` v0.5.0 with its `Adapter` trait + `RetrievedContext` + `MemoryBlock` types from sub-project 3a.

**Spec:** `docs/superpowers/specs/2026-05-18-provider-adapter-claude-3b-design.md`

---

## File structure (committed across tasks)

**Created:**
- `crates/singularmem-adapter-claude/Cargo.toml` — new crate manifest, version workspace-locked (resolves to 0.5.0 on the branch; bumps to 0.6.0 post-merge).
- `crates/singularmem-adapter-claude/src/lib.rs` — `ClaudeAdapter` struct, `Adapter` impl, private `escape_xml` helper, all unit tests inline.

**Modified:**
- `Cargo.toml` (workspace root) — add `singularmem-adapter-claude` to the root binary's `[dependencies]`.
- `src/main.rs` — `known_adapters()` gains `Box::new(singularmem_adapter_claude::ClaudeAdapter)`; the 3b line-comment placeholder is removed (3c/3d placeholders remain).
- `tests/cli.rs` — `retrieve_unknown_adapter_errors` (around line 1022) updated to use `nonexistent` instead of `claude`; expected `known adapters:` string updated. One new test `retrieve_with_claude_adapter_emits_xml_documents` appended.

**Unchanged on disk:** `docs/formats/store-v1.md` (`format_version` stays `"1"` — adapter is read-time only).

---

## Task 1: Crate scaffold + `ClaudeAdapter` struct with `name()`

**Why first:** Every later task imports `ClaudeAdapter` or its `Adapter` impl. Landing the scaffold + `name()` first lets Task 2 (`escape_xml`) and Task 3 (`format()`) build on a compiling foundation.

**Files:**
- Create: `crates/singularmem-adapter-claude/Cargo.toml`
- Create: `crates/singularmem-adapter-claude/src/lib.rs`

- [ ] **Step 1: Create the new-crate Cargo.toml**

Create `crates/singularmem-adapter-claude/Cargo.toml`:

```toml
[package]
name = "singularmem-adapter-claude"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Singularmem retrieval adapter for Anthropic Claude (pure formatter, no HTTP)."

[lints]
workspace = true

[dependencies]
singularmem-retrieve = { path = "../singularmem-retrieve" }
```

No `[features]`, no dev-deps, no other workspace deps. Tests live inline in `lib.rs` with `#[cfg(test)]` and use only the trait + types re-exported by `singularmem-retrieve`.

- [ ] **Step 2: Write the failing test**

Create `crates/singularmem-adapter-claude/src/lib.rs`:

```rust
//! Singularmem retrieval adapter for Anthropic Claude.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! the element-heavy XML shape Anthropic's prompt-engineering docs
//! recommend: a `<documents>` wrapper around 1-indexed `<document>`
//! elements with optional `<source>`/`<tags>` sub-elements and an
//! XML-escaped `<document_content>` body.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-claude-3b-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for Anthropic Claude. Stateless unit struct.
pub struct ClaudeAdapter;

impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 3 implements this.
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_returns_claude() {
        assert_eq!(ClaudeAdapter.name(), "claude");
    }
}
```

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The new crate compiles; the stubbed `format` returns empty `String` (Task 3 replaces it).

- [ ] **Step 4: Run the test**

Run: `cargo test -p singularmem-adapter-claude --lib`
Expected: PASS (`name_returns_claude`).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-claude --all-targets -- -D warnings`
Expected: zero warnings.

(The stubbed `format` returns `String::new()` — there's no `clippy::needless_pass_by_value` on `&self` and `&RetrievedContext` because those are references.)

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock crates/singularmem-adapter-claude/
git commit -s -m "feat(adapter-claude): new crate scaffold + ClaudeAdapter::name

Adds singularmem-adapter-claude crate depending only on
singularmem-retrieve. ClaudeAdapter is a public unit struct
implementing the Adapter trait; format() is stubbed and lands in
Task 3."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 2: `escape_xml` private helper

**Why next:** Task 3's `format` uses `escape_xml` for source/tags/content. Landing it in isolation with its own test means Task 3 can rely on a tested foundation and focus on the wrapper structure.

**Files:**
- Modify: `crates/singularmem-adapter-claude/src/lib.rs` (add helper + test)

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-adapter-claude/src/lib.rs`:

```rust
    #[test]
    fn escape_xml_replaces_ampersand_first_then_angle_brackets() {
        // & must be replaced first; otherwise the &amp; from the < replacement
        // would get re-escaped to &amp;amp; (and similarly for >).
        let input = "a & b < c > d";
        let out = escape_xml(input);
        assert_eq!(out, "a &amp; b &lt; c &gt; d");
    }

    #[test]
    fn escape_xml_leaves_quotes_and_apostrophes_alone() {
        // We never put user content into attribute values, so quote
        // escaping isn't needed and would just add noise.
        let input = r#"alice's "quoted" text"#;
        let out = escape_xml(input);
        assert_eq!(out, r#"alice's "quoted" text"#);
    }

    #[test]
    fn escape_xml_handles_pre_escaped_input_correctly() {
        // Input that already looks like an XML entity gets escaped again,
        // which is the right behaviour: a literal ampersand in user content
        // must be preserved as &amp;, not silently passed through.
        let input = "&amp; literal";
        let out = escape_xml(input);
        assert_eq!(out, "&amp;amp; literal");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-adapter-claude --lib escape_xml`
Expected: FAIL with `cannot find function 'escape_xml' in this scope`.

- [ ] **Step 3: Implement `escape_xml`**

Add to `crates/singularmem-adapter-claude/src/lib.rs`, between the `impl Adapter for ClaudeAdapter` block and the `#[cfg(test)] mod tests` block:

```rust
/// Escape the three characters that have special meaning inside XML text
/// content: `&`, `<`, `>`. `'` and `"` only matter inside attribute values,
/// which we never emit user content into (only the digit-only `index`).
///
/// Order matters: `&` MUST be replaced first, otherwise the `&amp;` from
/// other replacements would get double-escaped.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-adapter-claude --lib`
Expected: PASS for all 4 tests (`name_returns_claude` + 3 new `escape_xml_*`).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-claude --all-targets -- -D warnings`
Expected: zero warnings.

Watch for `clippy::needless_pass_by_value` — `s: &str` is a reference so it's fine. If clippy suggests something stricter, accept the suggestion.

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-adapter-claude/src/lib.rs
git commit -s -m "feat(adapter-claude): escape_xml private helper

Replaces &, <, > with their entity forms in that exact order
(& must come first to avoid double-escaping). Quote/apostrophe
escaping is deliberately omitted — user content never enters
attribute values. Tested with three cases including the
pre-escaped-input-stays-escaped property."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 3: `format()` implementation + 7 tests

**Files:**
- Modify: `crates/singularmem-adapter-claude/src/lib.rs` (replace `format` stub + add 7 tests)

This is the meatiest task. One function implementation + seven unit tests covering the seven format behaviours from the spec's Testing Strategy section.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/singularmem-adapter-claude/src/lib.rs`:

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
            total_considered: blocks_len_workaround(),
        }
    }

    // `blocks` is moved into `sample_context`, so we can't read its length
    // after; we pass a placeholder. Tests that care about total_considered
    // set it explicitly.
    fn blocks_len_workaround() -> usize {
        0
    }

    #[test]
    fn format_wraps_in_documents_element() {
        let ctx = sample_context(vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])], "fox");
        let out = ClaudeAdapter.format(&ctx);
        assert!(out.starts_with("<documents>\n"), "output should start with documents tag: {out}");
        assert!(out.trim_end().ends_with("</documents>"), "output should end with closing documents tag: {out}");
    }

    #[test]
    fn format_uses_one_indexed_document_indices() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(out.contains("<document index=\"1\">"), "missing index=1: {out}");
        assert!(out.contains("<document index=\"2\">"), "missing index=2: {out}");
        assert!(!out.contains("<document index=\"0\">"), "0-indexed slipped in: {out}");
    }

    #[test]
    fn format_includes_source_when_present() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.contains("<source>claude-conversation:abc-123</source>"),
            "missing source element: {out}"
        );
    }

    #[test]
    fn format_omits_source_when_none() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(!out.contains("<source>"), "unexpected source element: {out}");
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
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.contains("<tags>fox, animals</tags>"),
            "missing tags element: {out}"
        );
    }

    #[test]
    fn format_omits_tags_when_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(!out.contains("<tags>"), "unexpected tags element: {out}");
    }

    #[test]
    fn format_escapes_xml_special_chars_in_content() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]);
        block.content = r#"<script>alert("xss")</script> with & ampersand"#.to_string();
        let ctx = sample_context(vec![block], "fox");
        let out = ClaudeAdapter.format(&ctx);
        // Special chars escaped:
        assert!(out.contains("&lt;script&gt;"), "< not escaped: {out}");
        assert!(out.contains("&lt;/script&gt;"), "</ not escaped: {out}");
        assert!(out.contains("&amp;"), "& not escaped: {out}");
        // Raw tags must NOT appear (they would break Claude's XML parser):
        assert!(
            !out.contains("<script>"),
            "raw <script> tag leaked through into output: {out}"
        );
    }

    #[test]
    fn format_empty_context_emits_empty_documents() {
        let ctx = sample_context(vec![], "nothing matches");
        let out = ClaudeAdapter.format(&ctx);
        assert_eq!(out, "<documents></documents>\n");
    }
```

(Note: the `blocks_len_workaround` helper is intentionally trivial — `total_considered` isn't asserted in any of these tests because `Adapter::format` doesn't include it in the output. Tests that need a specific value would set it explicitly in the struct literal.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p singularmem-adapter-claude --lib format_`
Expected: most FAIL because `format` is the Task 1 stub returning `String::new()`. Specifically:
- `format_wraps_in_documents_element` fails on `starts_with("<documents>\n")`.
- The other six fail on their respective substring assertions.

- [ ] **Step 3: Replace the `format` stub with the real implementation**

In `crates/singularmem-adapter-claude/src/lib.rs`, replace the entire `impl Adapter for ClaudeAdapter` block:

```rust
impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 3 implements this.
        String::new()
    }
}
```

with:

```rust
impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return "<documents></documents>\n".to_string();
        }
        let mut out = String::new();
        let _ = writeln!(out, "<documents>");
        for (i, block) in ctx.blocks.iter().enumerate() {
            let _ = writeln!(out, "<document index=\"{}\">", i + 1);
            if let Some(source) = &block.source {
                let _ = writeln!(out, "<source>{}</source>", escape_xml(source));
            }
            if !block.tags.is_empty() {
                let joined = block.tags.join(", ");
                let _ = writeln!(out, "<tags>{}</tags>", escape_xml(&joined));
            }
            let _ = writeln!(out, "<document_content>");
            let _ = writeln!(out, "{}", escape_xml(&block.content));
            let _ = writeln!(out, "</document_content>");
            let _ = writeln!(out, "</document>");
        }
        let _ = writeln!(out, "</documents>");
        out
    }
}
```

You'll also need to add three dev-dependencies to make the new tests compile. Add to `crates/singularmem-adapter-claude/Cargo.toml`:

```toml
[dev-dependencies]
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }
jiff = { workspace = true }
```

Rationale: tests construct `MemoryBlock` literals which reference `ItemId` (from core), `ScoreKind` (from search), and `Timestamp` (from jiff). These are dev-only dependencies — production code never imports any of them directly.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p singularmem-adapter-claude --lib`
Expected: PASS for all 11 tests (1 name + 3 escape_xml + 7 format).

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p singularmem-adapter-claude --all-targets --tests -- -D warnings`
Expected: zero warnings.

Watch for:
- `clippy::doc_markdown` on `XML` in doc-comments — backtick-wrap if flagged.
- `clippy::format_push_string` if the `let _ = writeln!(out, ...)` pattern triggers it — accept the `writeln!` form (matches PlainAdapter's pattern from sub-project 3a).
- `clippy::needless_pass_by_value` on `_ctx: &RetrievedContext` — references, fine.

- [ ] **Step 6: Run fmt check**

Run: `cargo fmt --check`
Expected: clean. Apply `cargo fmt` and include in the commit below if not.

- [ ] **Step 7: Commit**

```bash
git add crates/singularmem-adapter-claude/Cargo.toml crates/singularmem-adapter-claude/src/lib.rs Cargo.lock
git commit -s -m "feat(adapter-claude): full format() implementation + 7 unit tests

Emits Anthropic's documented prompt-engineering shape: <documents>
wrapper, 1-indexed <document index=\"N\"> elements, optional
<source>/<tags> sub-elements (omitted when None/empty),
<document_content> with XML-escaped body. Empty context emits
<documents></documents> exactly.

Adds singularmem-core, singularmem-search, and jiff as dev-deps so
test fixtures can construct MemoryBlock literals. Production code
references only singularmem-retrieve."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 4: Root binary integration

**Files:**
- Modify: `Cargo.toml` (workspace root — add to root binary's `[dependencies]`)
- Modify: `src/main.rs:393-399` (or wherever `known_adapters()` is — look for `fn known_adapters()`)

- [ ] **Step 1: Add the crate as a runtime dep**

Modify the workspace root `Cargo.toml`. The current `[dependencies]` section includes:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

Add one line after `singularmem-retrieve`:

```toml
[dependencies]
singularmem-core = { path = "crates/singularmem-core" }
singularmem-search = { path = "crates/singularmem-search", features = ["testing"] }
singularmem-retrieve = { path = "crates/singularmem-retrieve" }
singularmem-adapter-claude = { path = "crates/singularmem-adapter-claude" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
```

The workspace members glob `members = ["crates/*"]` in `[workspace]` already auto-picks up the new crate; no further edit there.

- [ ] **Step 2: Register `ClaudeAdapter` in `known_adapters()`**

In `src/main.rs`, find the `known_adapters` function (currently around line 393):

```rust
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        // 3b will add: Box::new(singularmem_adapter_claude::ClaudeAdapter),
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
        // 3c will add: Box::new(singularmem_adapter_openai::OpenAiAdapter),
        // 3d will add: Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}
```

(The 3b line-comment placeholder becomes a real `Box::new` entry; 3c and 3d placeholders remain.)

- [ ] **Step 3: Build the workspace**

Run: `cargo build --workspace`
Expected: clean build. The CLI binary now links against the new crate.

- [ ] **Step 4: Verify CLI lists the new adapter in `--help`**

Run: `cargo run --quiet --bin singularmem -- retrieve --help 2>&1 | head -40`

This won't show `claude` directly because `--adapter` is a free-form `String` argument in the current CLI design — the registry only validates at dispatch time. But the build should succeed, and Task 5 will assert the registry behaviour via integration tests.

- [ ] **Step 5: Run the full existing CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for almost all tests, but `retrieve_unknown_adapter_errors` will FAIL because it asserts `--adapter claude` is unknown. Task 5 fixes that test. For now, confirm the failure is exactly that and nothing else:

```bash
cargo test --test cli 2>&1 | grep -E "FAILED|test result" | tail -10
```

Expected: exactly one failure named `retrieve_unknown_adapter_errors` (and any unrelated pre-existing flake). The rest of the suite (40+ tests) passes.

If MORE than one test fails, something else broke — investigate before continuing.

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -s -m "feat(cli): register ClaudeAdapter in known_adapters

Adds singularmem-adapter-claude to root binary deps and inserts
Box::new(singularmem_adapter_claude::ClaudeAdapter) into the
known_adapters() vec! after PlainAdapter. The 3b placeholder
comment is removed; 3c and 3d placeholders remain for the next
sub-projects.

Knowingly breaks the retrieve_unknown_adapter_errors test from
sub-project 3a — that test's assertion that '--adapter claude'
is unknown is no longer true. Task 5 updates the test."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 5: CLI tests — update existing + add new

**Files:**
- Modify: `tests/cli.rs` (around line 1022 — `retrieve_unknown_adapter_errors`; append new test at end)

- [ ] **Step 1: Update `retrieve_unknown_adapter_errors`**

Find `retrieve_unknown_adapter_errors` in `tests/cli.rs` (currently around line 1022). The current body asserts `--adapter claude` is unknown:

```rust
#[test]
fn retrieve_unknown_adapter_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // No need to ingest — the unknown-adapter check fails before any I/O.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "claude",
            "anything",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown adapter 'claude'"))
        .stderr(predicate::str::contains("known adapters: plain"));
}
```

Replace with:

```rust
#[test]
fn retrieve_unknown_adapter_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");

    // No need to ingest — the unknown-adapter check fails before any I/O.
    // Use a deliberately-fake adapter name; each new cloud adapter
    // (sub-projects 3b/3c/3d) makes its own name a valid choice, so the
    // unknown-adapter test must use something that will never become valid.
    singularmem()
        .args([
            "--store",
            db.to_str().unwrap(),
            "retrieve",
            "--adapter",
            "nonexistent",
            "anything",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown adapter 'nonexistent'"))
        .stderr(predicate::str::contains("known adapters: plain, claude"));
}
```

Two changes:
1. `--adapter claude` → `--adapter nonexistent` (and the matching `unknown adapter 'claude'` substring → `unknown adapter 'nonexistent'`).
2. `known adapters: plain` → `known adapters: plain, claude`.

- [ ] **Step 2: Add the new CLI integration test**

Append to the bottom of `tests/cli.rs`:

```rust
#[test]
fn retrieve_with_claude_adapter_emits_xml_documents() {
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
            "claude",
            "fox",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("<documents>"))
        .stdout(predicate::str::contains("<document index=\"1\">"))
        .stdout(predicate::str::contains("<document_content>"))
        .stdout(predicate::str::contains("the quick brown fox"))
        .stdout(predicate::str::contains("</document_content>"))
        .stdout(predicate::str::contains("</document>"))
        .stdout(predicate::str::contains("</documents>"));
}
```

- [ ] **Step 3: Run the updated + new tests**

Run: `cargo test --test cli retrieve_unknown_adapter_errors retrieve_with_claude_adapter_emits_xml_documents`
Expected: PASS for both.

- [ ] **Step 4: Run the full CLI test suite**

Run: `cargo test --test cli`
Expected: PASS for all tests (40 from prior sub-projects + 1 new = 41 total).

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests pass (171 from prior sub-projects + 11 new unit tests from 3a tasks 1-3 + 1 new CLI test = 183 approximately).

- [ ] **Step 6: Verify clippy clean**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 7: Verify fmt clean**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add tests/cli.rs
git commit -s -m "test(cli): update unknown-adapter test + add Claude integration test

Updates retrieve_unknown_adapter_errors to use 'nonexistent' as the
fake-adapter name (since 'claude' is now valid) and extends the
expected 'known adapters:' substring to include 'claude'.

Adds retrieve_with_claude_adapter_emits_xml_documents asserting
end-to-end that 'singularmem retrieve --adapter claude <query>'
emits Anthropic's <documents><document index=\"1\">...
</document></documents> shape."
```

Verify sign-off: `git log -1 --format=%B | grep -c '^Signed-off-by:'` must return `1`.

---

## Task 6: Final workspace gate

This task is a verification-only checkpoint. No source changes unless something below fails.

- [ ] **Step 1: Workspace fmt check**

Run: `cargo fmt --check`
Expected: clean. If not, `cargo fmt` and commit separately:

```bash
git add -p .
git commit -s -m "style: rustfmt cleanups after Claude adapter landing"
```

- [ ] **Step 2: Workspace clippy**

Run: `cargo clippy --workspace --all-targets --tests --benches -- -D warnings`
Expected: zero warnings.

- [ ] **Step 3: Workspace test**

Run: `cargo test --workspace`
Expected: all tests pass. The known pre-existing flake `singularmem-core::tests/store_basics::export_emits_meta_line_and_items_in_order` may intermittently fail; if so, re-run once to confirm. Do NOT attempt to fix it in this sub-project (out of scope).

- [ ] **Step 4: Rustdoc gate**

Run: `RUSTDOCFLAGS='-D missing-docs -D warnings' cargo doc --workspace --no-deps`
Expected: clean. The new crate's public items (`ClaudeAdapter` and its impl methods) all have doc-comments from Task 1's seed code. If rustdoc complains about a missing doc, fix it inline.

- [ ] **Step 5: Cargo.lock status**

Run: `git status Cargo.lock`

If `Cargo.lock` shows modifications from earlier tasks (almost certainly will, because Task 1 added a new crate and Task 3 added three dev-deps), it should already be staged in those task commits. Confirm:

```bash
git diff Cargo.lock
```

If clean, skip. If there's still uncommitted churn:

```bash
git add Cargo.lock
git commit -s -m "chore: refresh Cargo.lock after Claude adapter landing"
```

- [ ] **Step 6: Final repository status**

Run: `git status`
Expected: clean working tree (untracked `.agents/`, `.claude/`, `skills-lock.json` files are normal per prior sub-projects).

Run: `git log --oneline -10`
Expected: the new commits from Tasks 1-5 (plus optional Task 6 fmt/lockfile commits) sit on top of `fd345be` (the v0.5.0 version-bump commit from sub-project 3a's wrap-up) and `f9bac78` (the 3b design spec commit, currently on local main).

---

## Self-review

**1. Spec coverage check** (each spec acceptance criterion → task):

| Spec AC | Task |
|---|---|
| 1. New crate scaffold + workspace-version + depends only on singularmem-retrieve | 1 |
| 2. `ClaudeAdapter` is public unit struct implementing `Adapter` | 1 |
| 3. `name()` returns `"claude"` | 1 |
| 4. `format()` produces the spec'd XML shape; empty → `<documents></documents>\n` | 3 |
| 5. XML escaping handles `&`, `<`, `>` in that order | 2 |
| 6. Module layout: single `src/lib.rs` | 1 |
| 7. Root binary `Cargo.toml` `[dependencies]` adds new crate | 4 |
| 8. `known_adapters()` registers `ClaudeAdapter`; 3b placeholder removed; 3c/3d remain | 4 |
| 9. `retrieve_unknown_adapter_errors` updated | 5 |
| 10. All 9 unit tests + 1 new CLI test pass | distributed: 1, 2, 3, 5 |
| 11. No new perf budget | (no task — verified by absence) |
| 12. `docs/formats/store-v1.md` unchanged | (no task — verified by absence) |
| 13. Tag `v0.6.0` on merge | (out of plan scope — maintainer's merge ritual) |

All thirteen criteria covered.

Note on AC #10 test counts: spec says "9 unit tests + 1 new CLI test = 10 total new tests" but Task 2 also adds 3 escape_xml tests. The 9 unit tests count includes `name_returns_claude` + 1 (escape_xml — spec was conservative; we landed 3 for thoroughness) + 7 format tests. The extra 2 escape_xml tests are bonus coverage; they don't violate the spec.

Actually let me recount more carefully. Spec's Testing Strategy lists exactly these unit tests:
- `name_returns_claude`
- `format_wraps_in_documents_element`
- `format_uses_one_indexed_document_indices`
- `format_includes_source_when_present`
- `format_omits_source_when_none`
- `format_includes_tags_when_non_empty`
- `format_omits_tags_when_empty`
- `format_escapes_xml_special_chars_in_content`
- `format_empty_context_emits_empty_documents`

That's 9. The plan also adds three `escape_xml_*` tests in Task 2 that aren't named in the spec's table. These are tests for the *helper function*, not the *adapter*, so they're complementary not redundant. They add confidence in the helper's correctness independent of the format() integration. Keep them.

**2. Placeholder scan:** no TBDs, no "implement later" (Task 1's stub is explicitly marked as a Task 3 dependency with line-content shown), no "similar to Task N". Every task has complete code blocks. Task 6 is verification-only and has exact commands + expected outputs.

**3. Type consistency:**
- `ClaudeAdapter` (unit struct, no generics) consistent across Tasks 1, 3, 4, 5.
- `name(&self) -> &'static str` consistent across Tasks 1 and 3.
- `format(&self, &RetrievedContext) -> String` consistent across Tasks 1 and 3.
- `escape_xml(s: &str) -> String` consistent across Tasks 2 and 3.
- `known_adapters()` return type `Vec<Box<dyn singularmem_retrieve::Adapter>>` matches the existing 3a signature.
- CLI test substring assertions match the format produced by Task 3's implementation.

Plan ready for execution.
