use std::path::PathBuf;

use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
use singularmem_ingest::{CursorChats, Source};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let d = TempDir::new().unwrap();
    let user = d.path().join("User");
    write_fixture(
        &user,
        &[
            FixtureWorkspace {
                hash: "aaa",
                folder: Some("/home/me/proj"),
                composers: vec![(
                    "c1",
                    "Fix the build",
                    1_741_900_656_013,
                    vec![
                        FixtureBubble {
                            id: "b1",
                            kind: 1,
                            text: "why does the build fail?",
                        },
                        FixtureBubble {
                            id: "b2",
                            kind: 2,
                            text: "Because of X.",
                        },
                        FixtureBubble {
                            id: "b3",
                            kind: 2,
                            text: "",
                        },
                    ],
                )],
            },
            FixtureWorkspace {
                hash: "bbb",
                folder: Some("/home/me/other"),
                composers: vec![(
                    "c2",
                    "Other",
                    1_741_900_700_000,
                    vec![FixtureBubble {
                        id: "b4",
                        kind: 1,
                        text: "hello other",
                    }],
                )],
            },
            FixtureWorkspace {
                hash: "ccc",
                folder: None,
                composers: vec![],
            },
            FixtureWorkspace {
                hash: "ddd",
                // Cursor stores `folder` as a percent-encoded `file://` URI;
                // `write_fixture` writes this string verbatim after
                // prepending `file://`.
                folder: Some("/home/me/My%20Repo"),
                composers: vec![(
                    "c3",
                    "Decoded",
                    1_741_900_800_000,
                    vec![FixtureBubble {
                        id: "b5",
                        kind: 1,
                        text: "hi in my repo",
                    }],
                )],
            },
        ],
    );
    (d, user)
}

#[test]
fn parses_workspaces_composers_and_bubbles() {
    let (_d, user) = fixture();
    let src = CursorChats::open(&user).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i.external_id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "cursor:c1:b1",
            "cursor:c1:b2",
            "cursor:c2:b4",
            "cursor:c3:b5"
        ]
    );
    assert_eq!(items[0].tags, vec!["cursor", "role:user", "transcript"]);
    assert_eq!(items[1].metadata["role"], "assistant");
    assert_eq!(items[1].metadata["title"], "Fix the build");
    assert_eq!(items[1].metadata["workspace"], "/home/me/proj");
    assert_eq!(
        items[1].metadata["composer_created_at"],
        "2025-03-13T21:17:36.013Z"
    );
    assert_eq!(items[1].metadata["index"], 1);
    assert_eq!(src.default_scope(&items[0]).as_deref(), Some("cursor/proj"));
    assert_eq!(
        src.default_scope(&items[2]).as_deref(),
        Some("cursor/other")
    );
    // workspace ddd's folder is a percent-encoded URI ("My%20Repo"); it must
    // be decoded before it ever reaches metadata or scope derivation.
    assert_eq!(items[3].metadata["workspace"], "/home/me/My Repo");
    // "My Repo" has a space, which is not a valid scope segment: correct
    // behaviour, but it must be reached via the *decoded* path, not because
    // decoding never happened.
    assert_eq!(src.default_scope(&items[3]), None);
    // b3 (empty text) filtered; workspace ccc (no workspace.json) counted as filtered.
    assert_eq!(src.filtered_count(), 2);
}

#[test]
fn project_filter_matches_percent_decoded_workspace() {
    let (_d, user) = fixture();
    let mut src = CursorChats::open(&user).unwrap();
    src.project_filter = Some(PathBuf::from("/home/me/My Repo"));
    let items: Vec<_> = src.items().filter_map(Result::ok).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].external_id.as_deref(), Some("cursor:c3:b5"));
}

#[test]
fn skips_workspace_with_unreadable_db_and_counts_it_filtered() {
    let (_d, user) = fixture();
    // Corrupt workspace `aaa`'s state.vscdb with 10 bytes of garbage; it
    // opens fine at the connection level but fails on the first query.
    std::fs::write(user.join("workspaceStorage/aaa/state.vscdb"), b"0123456789").unwrap();
    let src = CursorChats::open(&user).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i.external_id.as_deref().unwrap())
        .collect();
    // Workspace aaa (c1) yields nothing; bbb and ddd are unaffected.
    assert_eq!(ids, vec!["cursor:c2:b4", "cursor:c3:b5"]);
    // ccc (no workspace.json) + the unreadable aaa db, both filtered.
    assert_eq!(src.filtered_count(), 2);
}

#[test]
fn filters_by_project_and_conversation() {
    let (_d, user) = fixture();
    let mut src = CursorChats::open(&user).unwrap();
    src.project_filter = Some(PathBuf::from("/home/me/other"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 1);
    src.project_filter = None;
    src.conversation_filter = Some("c1".into());
    assert_eq!(src.items().filter_map(Result::ok).count(), 2);
}

/// The same composer id listed in two workspaces. Cursor keys the
/// conversation's headers and bubbles by composer id in the *global* DB, so
/// a conversation that has been open in more than one window shows up in
/// both workspaces' `composer.composerData`.
fn fixture_shared_composer() -> (TempDir, PathBuf) {
    let d = TempDir::new().unwrap();
    let user = d.path().join("User");
    let composer = || {
        vec![(
            "shared",
            "Shared",
            1_700_000_000_000_i64,
            vec![FixtureBubble {
                id: "b1",
                kind: 1,
                text: "shared question",
            }],
        )]
    };
    write_fixture(
        &user,
        &[
            FixtureWorkspace {
                hash: "wa",
                folder: Some("/w/a"),
                composers: composer(),
            },
            FixtureWorkspace {
                hash: "wb",
                folder: Some("/w/b"),
                composers: composer(),
            },
        ],
    );
    (d, user)
}

/// A conversation filter alone does not say *which* workspace the hook was
/// fired from, so the hook also passes `cwd` as a project filter. With both
/// set, the item's `workspace` (and hence its derived scope) must be the
/// workspace the hook actually came from.
#[test]
fn conversation_and_project_filter_pick_the_right_workspace() {
    let (_d, user) = fixture_shared_composer();
    let mut src = CursorChats::open(&user).unwrap();
    src.conversation_filter = Some("shared".into());
    src.project_filter = Some(PathBuf::from("/w/b"));
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].metadata["workspace"], "/w/b");
    assert_eq!(src.default_scope(&items[0]).as_deref(), Some("cursor/b"));
}

/// With only a conversation filter, the scan stops at the first workspace
/// that matches instead of re-emitting the same bubbles once per workspace
/// listing the conversation.
#[test]
fn conversation_filter_stops_at_the_first_matching_workspace() {
    let (_d, user) = fixture_shared_composer();
    let mut src = CursorChats::open(&user).unwrap();
    src.conversation_filter = Some("shared".into());
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    let ids: Vec<&str> = items
        .iter()
        .map(|i| i.external_id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["cursor:shared:b1"],
        "one set of items, not one per workspace listing the conversation"
    );
}

#[test]
fn missing_global_db_is_not_found() {
    let d = TempDir::new().unwrap();
    assert!(matches!(
        CursorChats::open(d.path()),
        Err(singularmem_ingest::Error::NotFound { .. })
    ));
}
