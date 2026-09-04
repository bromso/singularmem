use std::path::{Path, PathBuf};

use singularmem_core::{NewItem, Store};
use singularmem_ingest::{ingest_source, ClaudeTranscript, DirectoryWalker, Result, Source};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session.jsonl")
}

#[test]
fn transcript_default_scope_is_claude_code_slash_cwd_basename() {
    let t = ClaudeTranscript::open(fixture()).unwrap();
    let first = t.items().find_map(Result::ok).unwrap();
    assert_eq!(first.scope, None, "sources leave scope for the driver");
    assert_eq!(t.default_scope(&first).as_deref(), Some("claude-code/proj"));
}

#[test]
fn transcript_override_wins() {
    let mut t = ClaudeTranscript::open(fixture()).unwrap();
    t.scope_override = Some("Team/Alpha".into());
    let first = t.items().find_map(Result::ok).unwrap();
    assert_eq!(t.default_scope(&first).as_deref(), Some("team/alpha"));
}

#[test]
fn directory_default_scope_is_files_slash_root_basename() {
    let d = TempDir::new().unwrap();
    let root = d.path().join("MyRepo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "hi").unwrap();
    let w = DirectoryWalker::new(&root).unwrap();
    let item = w.items().next().unwrap().unwrap();
    assert_eq!(w.default_scope(&item).as_deref(), Some("files/myrepo"));
}

#[test]
fn invalid_basename_yields_no_scope() {
    let d = TempDir::new().unwrap();
    let root = d.path().join("bad name");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "hi").unwrap();
    let w = DirectoryWalker::new(&root).unwrap();
    let item = w.items().next().unwrap().unwrap();
    assert_eq!(w.default_scope(&item), None);
}

#[test]
fn driver_applies_default_scope_and_preserves_explicit() {
    let d = TempDir::new().unwrap();
    let root = d.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "hi").unwrap();
    std::fs::write(root.join(".gitignore"), "s.db*\n").unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let w = DirectoryWalker::new(&root).unwrap();
    ingest_source(&s, &w, false).unwrap();
    let item = s.list().unwrap().next().unwrap().unwrap();
    assert_eq!(item.scope.as_deref(), Some("files/repo"));
    assert_eq!(s.scopes().unwrap(), vec![("files/repo".to_string(), 1)]);
}

/// A fixed, in-memory `Source` whose items are handed in directly, to
/// exercise the driver's "explicit scope wins over default" rule without a
/// filesystem fixture.
struct Fixed(Vec<NewItem>);

impl Source for Fixed {
    fn name(&self) -> String {
        "fixed".to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        Box::new(self.0.iter().cloned().map(Ok))
    }

    fn default_scope(&self, _item: &NewItem) -> Option<String> {
        Some("default/x".to_string())
    }
}

#[test]
fn driver_applies_default_scope_only_when_item_has_none() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();

    let mut explicit = NewItem::text("has its own scope");
    explicit.scope = Some("explicit/y".into());
    let unset = NewItem::text("no scope of its own");

    let source = Fixed(vec![explicit, unset]);
    ingest_source(&s, &source, false).unwrap();

    assert_eq!(
        s.scopes().unwrap(),
        vec![("default/x".to_string(), 1), ("explicit/y".to_string(), 1),]
    );
}
