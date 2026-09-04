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
