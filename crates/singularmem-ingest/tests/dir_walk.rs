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
