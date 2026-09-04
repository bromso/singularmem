use std::path::Path;

use singularmem_core::{NewItem, ScopeFilter, Store};
use singularmem_retrieve::wakeup::{build, header, render, ScopeSet, WakeupOptions};
use singularmem_retrieve::PlainAdapter;
use tempfile::TempDir;

fn store_with(items: &[(&str, &str)]) -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (c, sc) in items {
        let mut n = NewItem::text(*c);
        n.scope = Some((*sc).into());
        s.ingest(n).unwrap();
    }
    (d, s)
}

/// A `Clock` that always returns the same instant, so ingests made through
/// it tie on `created_at`. `std::env::set_current_dir` is process-global, so
/// callers wanting a `.`/`..`-relative path build it directly against a
/// `TempDir` instead of changing the process cwd.
#[derive(Debug, Clone, Copy)]
struct FixedClock(jiff::Timestamp);

impl singularmem_core::Clock for FixedClock {
    fn now(&self) -> jiff::Timestamp {
        self.0
    }
}

fn store_with_fixed_clock(items: &[(&str, &str)]) -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let clock = FixedClock("2026-01-01T00:00:00Z".parse().unwrap());
    let s = Store::open_with(
        d.path().join("s.db"),
        Box::new(clock),
        Box::new(singularmem_core::OsRng),
    )
    .unwrap();
    for (c, sc) in items {
        let mut n = NewItem::text(*c);
        n.scope = Some((*sc).into());
        s.ingest(n).unwrap();
    }
    (d, s)
}

#[test]
fn scope_set_for_project_unions_editor_scopes() {
    let set = ScopeSet::for_project(Path::new("/tmp/My Repo"), false);
    // basename "My Repo" is not a valid segment (space) → normalised? No: invalid → dropped.
    assert!(set.0.is_empty());
    let set = ScopeSet::for_project(Path::new("/home/me/singularmem"), false);
    assert_eq!(
        set.names(),
        vec![
            "claude-code/singularmem",
            "codex/singularmem",
            "cursor/singularmem"
        ]
    );
    let set = ScopeSet::for_project(Path::new("/home/me/singularmem"), true);
    assert_eq!(
        set.names().last().map(String::as_str),
        Some("files/singularmem")
    );
}

#[test]
fn scope_set_for_project_canonicalizes_dot_and_dot_dot() {
    let d = TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("proj/sub")).unwrap();

    // `dir.file_name()` is `None` for both `.` and `..` themselves, so
    // `for_project` must canonicalise first (resolving them away) rather
    // than reading the basename off the un-resolved path.
    let dot = d.path().join("proj").join(".");
    let set = ScopeSet::for_project(&dot, false);
    assert_eq!(
        set.names(),
        vec!["claude-code/proj", "codex/proj", "cursor/proj"]
    );

    let dot_dot = d.path().join("proj").join("sub").join("..");
    let set = ScopeSet::for_project(&dot_dot, false);
    assert_eq!(
        set.names(),
        vec!["claude-code/proj", "codex/proj", "cursor/proj"]
    );
}

#[test]
fn build_returns_recent_items_oldest_to_newest_across_scopes() {
    let (_d, s) = store_with(&[
        ("c1", "claude-code/p"),
        ("x", "cursor/other"),
        ("k1", "codex/p"),
        ("c2", "claude-code/p"),
    ]);
    let set = ScopeSet::for_project(Path::new("/w/p"), false);
    let w = build(
        &s,
        &set,
        &WakeupOptions {
            limit: 2,
            max_bytes: 8192,
        },
    )
    .unwrap();
    assert_eq!(w.total, 3);
    assert_eq!(w.shown, 2);
    let contents: Vec<&str> = w
        .context
        .blocks
        .iter()
        .map(|b| b.content.as_str())
        .collect();
    assert_eq!(contents, vec!["k1", "c2"]);
    assert_eq!(w.context.query, "wake-up:claude-code/p,codex/p,cursor/p");
    assert!(w.context.blocks.iter().all(|b| b.score == 0.0));
}

#[test]
fn render_has_header_and_budget_drops_oldest_first() {
    let (_d, s) = store_with(&[
        ("aaaa aaaa aaaa", "codex/p"),
        ("bbbb bbbb bbbb", "codex/p"),
        ("cccc cccc cccc", "codex/p"),
    ]);
    let set = ScopeSet(vec![ScopeFilter::descendants("codex/p").unwrap()]);
    let w = build(&s, &set, &WakeupOptions::default()).unwrap();
    let full = render(&w, &PlainAdapter, 100_000);
    assert!(full.starts_with("# Singularmem wake-up — codex/p — 3 items, showing last 3\n"));
    assert!(full.contains("aaaa") && full.contains("cccc"));
    let small = render(&w, &PlainAdapter, full.len() - 20);
    assert!(small.starts_with(&header(&w)));
    assert!(!small.contains("aaaa"), "oldest dropped first");
    assert!(small.contains("cccc"));
    assert!(small.len() <= full.len() - 20);
    let tiny = render(&w, &PlainAdapter, 10);
    assert_eq!(
        tiny.trim_end(),
        header(&w).trim_end(),
        "header always survives"
    );
}

#[test]
fn build_dedups_items_seen_via_overlapping_scope_filters() {
    // A real `Store::open` clock (`SystemClock`) makes it likely, but not
    // guaranteed, that two fast-consecutive ingests land on distinct
    // `created_at` values — which is exactly the case an adjacent-only
    // `dedup_by` (the pre-fix implementation) glosses over. `store_with_fixed_clock`
    // pins both ingests to the same instant so the two items always tie on
    // `created_at`, forcing the discriminating case: after `sort_by_key`
    // with a tie, the two instances of an id are not guaranteed adjacent
    // (or even ordered id-then-id), so `dedup_by` on adjacents would have
    // let 3 or 4 blocks (duplicates from one or both scope filters) through
    // instead of 2. `build`'s `HashSet`-based dedup collapses by id
    // regardless of position or ties.
    let (_d, s) = store_with_fixed_clock(&[("c1", "a/b"), ("c2", "a/b")]);
    let set = ScopeSet(vec![
        ScopeFilter::descendants("a").unwrap(),
        ScopeFilter::descendants("a/b").unwrap(),
    ]);
    let w = build(
        &s,
        &set,
        &WakeupOptions {
            limit: 10,
            max_bytes: 8192,
        },
    )
    .unwrap();
    // Both filters match both items; each item must appear exactly once.
    let ids: Vec<_> = w.context.blocks.iter().map(|b| b.id).collect();
    let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "no duplicate ids: {ids:?}");
    assert_eq!(unique.len(), 2);
    assert_eq!(w.context.blocks.len(), 2, "exactly 2 blocks, not 3 or 4");
    // `total` unions the two overlapping filters rather than summing
    // `count_scoped` per filter (which would double count both items:
    // 2 (via `a`) + 2 (via `a/b`) = 4).
    assert_eq!(w.total, 2);
}

#[test]
fn empty_store_gives_header_only() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let set = ScopeSet::for_project(Path::new("/w/p"), false);
    let w = build(&s, &set, &WakeupOptions::default()).unwrap();
    assert_eq!(w.total, 0);
    let out = render(&w, &PlainAdapter, 8192);
    assert_eq!(
        out,
        "# Singularmem wake-up — claude-code/p, codex/p, cursor/p — 0 items, showing last 0\n"
    );
}
