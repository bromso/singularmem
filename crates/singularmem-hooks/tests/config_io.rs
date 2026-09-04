use serde_json::json;
use singularmem_hooks::{read_config, write_config, Error};

#[test]
fn read_config_missing_file_returns_empty_object() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("does-not-exist.json");
    let value = read_config(&path).unwrap();
    assert_eq!(value, json!({}));
}

#[test]
fn read_config_invalid_json_is_reported() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, "not json").unwrap();
    match read_config(&path) {
        Err(Error::InvalidJson { path: p, .. }) => assert_eq!(p, path),
        other => panic!("expected Error::InvalidJson, got {other:?}"),
    }
}

#[test]
fn write_config_then_read_config_round_trips() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    let value = json!({"b": 1, "a": [1, 2, 3]});
    write_config(&path, &value).unwrap();
    let read_back = read_config(&path).unwrap();
    assert_eq!(read_back, value);
}

#[test]
fn write_config_uses_two_space_indent_trailing_newline_and_no_tmp_sibling() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    let value = json!({"a": 1, "b": 2});
    write_config(&path, &value).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'));
    let second_line = text.lines().nth(1).expect("at least two lines");
    assert!(
        second_line.starts_with("  \""),
        "expected two-space indent, got {second_line:?}"
    );

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    assert!(!std::path::Path::new(&tmp_name).exists());
}

#[test]
fn write_config_creates_missing_nested_directories() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("a/b/c/settings.json");
    let value = json!({"x": true});
    write_config(&path, &value).unwrap();
    assert_eq!(read_config(&path).unwrap(), value);
}
