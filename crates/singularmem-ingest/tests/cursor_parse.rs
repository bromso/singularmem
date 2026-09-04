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
    assert_eq!(ids, vec!["cursor:c1:b1", "cursor:c1:b2", "cursor:c2:b4"]);
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
    // b3 (empty text) filtered; workspace ccc (no workspace.json) counted as filtered.
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

#[test]
fn missing_global_db_is_not_found() {
    let d = TempDir::new().unwrap();
    assert!(matches!(
        CursorChats::open(d.path()),
        Err(singularmem_ingest::Error::NotFound { .. })
    ));
}
