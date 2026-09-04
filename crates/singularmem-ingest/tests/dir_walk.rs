use std::fs;

use singularmem_ingest::{DirectoryWalker, Source};
use tempfile::TempDir;

fn tree() -> TempDir {
    let d = TempDir::new().unwrap();
    fs::create_dir_all(d.path().join("src")).unwrap();
    fs::create_dir_all(d.path().join("target")).unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
    fs::write(d.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(d.path().join("README.md"), "# hi\n").unwrap();
    fs::write(d.path().join("target/out.txt"), "ignored").unwrap();
    fs::write(d.path().join(".git/HEAD"), "ref: x").unwrap();
    fs::write(d.path().join("blob.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    fs::write(d.path().join("big.txt"), "x".repeat(2000)).unwrap();
    d
}

#[test]
fn walks_text_files_respecting_gitignore_and_hidden() {
    let d = tree();
    let mut w = DirectoryWalker::new(d.path()).unwrap();
    w.max_file_bytes = 1000;
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    let mut rels: Vec<String> = items
        .iter()
        .map(|i| i.metadata["rel_path"].as_str().unwrap().to_string())
        .collect();
    rels.sort();
    assert_eq!(rels, vec!["README.md", "src/main.rs"]);
    assert_eq!(
        w.filtered_count(),
        2,
        "blob.bin (binary) and big.txt (too large)"
    );
}

#[test]
fn item_shape() {
    let d = tree();
    let w = DirectoryWalker::new(d.path()).unwrap();
    let item = w
        .items()
        .map(Result::unwrap)
        .find(|i| i.metadata["rel_path"] == "src/main.rs")
        .unwrap();
    let root = d.path().canonicalize().unwrap();
    let abs = root.join("src/main.rs");
    assert_eq!(
        item.external_id.as_deref(),
        Some(format!("file:{}", abs.display()).as_str())
    );
    assert_eq!(
        item.source.as_deref(),
        Some(format!("dir:{}", root.display()).as_str())
    );
    assert_eq!(item.tags, vec!["ext:rs", "file"]);
    assert_eq!(item.content, "fn main() {}");
    assert_eq!(item.metadata["path"], abs.display().to_string());
    assert_eq!(item.metadata["size_bytes"], 13);
    assert_eq!(item.metadata["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(item.metadata["chunk_count"], 1);
}

#[test]
fn oversized_text_is_chunked() {
    let d = TempDir::new().unwrap();
    fs::write(
        d.path().join("a.txt"),
        format!("{}\n\n{}", "p".repeat(30), "q".repeat(30)),
    )
    .unwrap();
    let mut w = DirectoryWalker::new(d.path()).unwrap();
    w.chunk_bytes = 40;
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 2);
    assert!(items[0]
        .external_id
        .as_deref()
        .unwrap()
        .ends_with("a.txt#0"));
    assert!(items[1]
        .external_id
        .as_deref()
        .unwrap()
        .ends_with("a.txt#1"));
}

#[test]
fn missing_root_is_not_found() {
    assert!(matches!(
        DirectoryWalker::new("/definitely/missing"),
        Err(singularmem_ingest::Error::NotFound { .. })
    ));
}

#[cfg(unix)]
#[test]
fn walk_error_reports_failing_path() {
    use std::os::unix::fs::PermissionsExt;

    let d = TempDir::new().unwrap();
    let locked = d.path().join("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::write(locked.join("x.txt"), "hi").unwrap();

    let mut perms = fs::metadata(&locked).unwrap().permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&locked, perms).unwrap();

    if fs::read_dir(&locked).is_ok() {
        eprintln!("skipping walk_error_reports_failing_path: running as root, permissions ignored");
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&locked, perms).unwrap();
        return;
    }

    let w = DirectoryWalker::new(d.path()).unwrap();
    let errors: Vec<_> = w.items().filter_map(std::result::Result::err).collect();

    let mut perms = fs::metadata(&locked).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&locked, perms).unwrap();

    assert!(
        errors.iter().any(|e| matches!(
            e,
            singularmem_ingest::Error::Io { path, .. }
                if path.components().any(|c| c.as_os_str() == "locked")
        )),
        "expected an Io error under `locked`, got: {errors:?}"
    );
}

/// Build a directory chain under `root` until the absolute path of a file
/// placed inside it exceeds `target` bytes. Returns that file's path.
fn deep_file(root: &std::path::Path, target: usize) -> std::path::PathBuf {
    let mut dir = root.to_path_buf();
    while dir.join("f.txt").as_os_str().len() <= target {
        dir = dir.join("a".repeat(100));
    }
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("f.txt");
    fs::write(&f, "deep").unwrap();
    f
}

#[test]
fn over_long_path_is_an_error_not_a_dropped_run() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join("ok.txt"), "fine").unwrap();
    let long = deep_file(d.path(), 520);
    assert!(long.as_os_str().len() > 520);

    let w = DirectoryWalker::new(d.path()).unwrap();
    let (oks, errs): (Vec<_>, Vec<_>) = w.items().partition(Result::is_ok);
    assert_eq!(oks.len(), 1, "the normal file still yields an item");
    assert_eq!(errs.len(), 1, "the over-long path yields one error");
    assert!(
        matches!(
            errs.into_iter().map(Result::unwrap_err).next().unwrap(),
            singularmem_ingest::Error::Unsupported { ref reason, .. }
                if reason.contains("512")
        ),
        "expected an Unsupported error naming the 512-byte cap"
    );
}

#[test]
fn parent_gitignore_is_not_consulted() {
    let d = TempDir::new().unwrap();
    fs::write(d.path().join(".gitignore"), "sub/keep.txt\n").unwrap();
    fs::create_dir_all(d.path().join("sub")).unwrap();
    fs::write(d.path().join("sub/keep.txt"), "kept").unwrap();

    let w = DirectoryWalker::new(d.path().join("sub")).unwrap();
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    let rels: Vec<String> = items
        .iter()
        .map(|i| i.metadata["rel_path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        rels,
        vec!["keep.txt"],
        "a .gitignore in the parent of the walked root must not apply"
    );
}

#[test]
fn over_long_extension_drops_the_ext_tag_only() {
    let d = TempDir::new().unwrap();
    let name = format!("x.{}", "e".repeat(80));
    fs::write(d.path().join(&name), "body").unwrap();
    let w = DirectoryWalker::new(d.path()).unwrap();
    let items: Vec<_> = w.items().map(Result::unwrap).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].tags, vec!["file"], "ext: tag omitted, not fatal");
}
