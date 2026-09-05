//! `--help` must be byte-identical across refactors of the CLI binary.
//! Regenerate with `UPDATE_HELP_SNAPSHOTS=1 cargo test --test help_snapshots`.
use assert_cmd::Command;
use std::path::PathBuf;

const SUBCOMMANDS: &[&[&str]] = &[
    &[],
    &["ingest"],
    &["ingest-transcript"],
    &["ingest-codex"],
    &["ingest-cursor"],
    &["ingest-dir"],
    &["get"],
    &["list"],
    &["revisions"],
    &["export"],
    &["search"],
    &["reindex"],
    &["retrieve"],
    &["semantic-search"],
    &["scope"],
    &["scope", "list"],
    &["scope", "move"],
    &["wake-up"],
    &["hook"],
    &["hooks"],
    &["hooks", "install"],
    &["hooks", "uninstall"],
    &["hooks", "status"],
    &["graph"],
    &["graph", "add"],
    &["graph", "query"],
    &["graph", "predicate"],
    &["graph", "invalidate"],
    &["graph", "supersede"],
    &["graph", "timeline"],
    &["graph", "stats"],
    &["graph", "entities"],
    &["graph", "history"],
];

fn snapshot_path(args: &[&str]) -> PathBuf {
    let name = if args.is_empty() {
        "root".to_string()
    } else {
        args.join("-")
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/help")
        .join(format!("{name}.txt"))
}

#[test]
fn help_output_matches_snapshots() {
    let update = std::env::var_os("UPDATE_HELP_SNAPSHOTS").is_some();
    let mut failures = Vec::new();
    for args in SUBCOMMANDS {
        let mut cmd = Command::cargo_bin("singularmem").unwrap();
        cmd.args(*args).arg("--help");
        // Fixed width so clap's wrapping is deterministic.
        cmd.env("COLUMNS", "100").env("NO_COLOR", "1");
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "--help failed for {args:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        let path = snapshot_path(args);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing snapshot {}", path.display()));
        if expected != text {
            failures.push(format!(
                "{args:?}: --help changed (run with UPDATE_HELP_SNAPSHOTS=1 to accept)"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
