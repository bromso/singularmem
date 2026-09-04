use singularmem_core::scope::{ancestors, validate};
use singularmem_core::{Error, ScopeFilter};

#[test]
fn normalises_case_and_slashes() {
    assert_eq!(
        validate("Claude-Code/SingularMem").unwrap(),
        "claude-code/singularmem"
    );
    assert_eq!(validate("/a/b/").unwrap(), "a/b");
}

#[test]
fn rejects_bad_shapes() {
    for bad in ["", "/", "a//b", "a/./b", "a/../b", "a b", "a/b?", "ä"] {
        assert!(
            matches!(validate(bad), Err(Error::Validation { field: "scope", .. })),
            "{bad:?}"
        );
    }
    let nine = ["s"; 9].join("/");
    assert!(validate(&nine).is_err(), "9 segments");
    assert!(validate(&"s".repeat(65)).is_err(), "65-byte segment");
    let eight_ok = ["seg"; 8].join("/");
    assert!(validate(&eight_ok).is_ok());
    assert!(validate(&"s".repeat(64)).is_ok());
    // 8 × 64 = 512 bytes of segments + 7 slashes = 519 > 512 → rejected on total.
    let too_long = vec!["s".repeat(64); 8].join("/");
    assert!(validate(&too_long).is_err(), "total > 512");
}

#[test]
fn ancestors_are_every_prefix_in_order() {
    assert_eq!(ancestors("a/b/c"), vec!["a", "a/b", "a/b/c"]);
    assert_eq!(ancestors("solo"), vec!["solo"]);
}

#[test]
fn filter_constructors_validate() {
    let f = ScopeFilter::descendants("A/B").unwrap();
    assert_eq!(
        f,
        ScopeFilter {
            path: "a/b".into(),
            exact: false
        }
    );
    assert!(ScopeFilter::exact("a//b").is_err());
}
