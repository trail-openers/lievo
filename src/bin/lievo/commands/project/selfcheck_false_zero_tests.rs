// Unit tests for false-0-callers detection (issue #715).

use super::*;
use std::collections::HashMap;

fn no_aliases() -> HashMap<String, String> {
    HashMap::new()
}

#[test]
fn flags_file_with_on_disk_but_no_recorded_importers() {
    let all_files = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |f| {
        if f == "src/b.rs" { 2 } else { 0 }
    });
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].file, "src/b.rs");
    assert_eq!(flagged[0].on_disk_importers, 2);
}

#[test]
fn gated_off_when_unresolved_internal_nonzero() {
    let all_files = vec!["src/b.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 1, &no_aliases(), |_| 5);
    assert!(
        flagged.is_empty(),
        "unresolved_internal != 0 must suppress the false-0 signal entirely"
    );
}

#[test]
fn not_flagged_when_recorded_importer_exists() {
    let all_files = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
    let resolved = vec![("src/a.rs".to_string(), "src/b.rs".to_string())];
    // Only b.rs would show on-disk importers; a.rs is the entry point (0
    // recorded AND 0 on-disk importers, so it must not be flagged either).
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |f| {
        if f == "src/b.rs" { 3 } else { 0 }
    });
    assert!(flagged.is_empty(), "b.rs has a recorded importer");
}

#[test]
fn self_edge_does_not_count_as_importer() {
    let all_files = vec!["src/a.rs".to_string()];
    let resolved = vec![("src/a.rs".to_string(), "src/a.rs".to_string())];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |f| {
        if f == "src/a.rs" { 1 } else { 0 }
    });
    assert_eq!(
        flagged.len(),
        1,
        "self-edges must not count as a recorded importer"
    );
}

// --- #[path] alias exemption (issue #732 round-2 finding #1) ----------------

#[test]
fn path_alias_file_not_flagged() {
    // A #[path]-diverged file in the alias map must not be flagged, even if
    // it has on-disk importers (its only importer is a #[cfg(test)]-gated
    // declaration that the non-test profile never compiles).
    let all_files = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let mut aliases = HashMap::new();
    aliases.insert("src/b.rs".to_string(), "a::b".to_string());
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &aliases, |f| {
        if f == "src/b.rs" { 2 } else { 0 }
    });
    assert!(
        flagged.is_empty(),
        "a #[path]-diverged file in the alias map must not be flagged"
    );
}

// --- test-like file exclusion (issue #725) ---------------------------------

#[test]
fn test_file_under_tests_dir_not_flagged() {
    let all_files = vec!["tests/it.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 5);
    assert!(
        flagged.is_empty(),
        "file under a 'tests' path component must not be flagged"
    );
}

#[test]
fn mytests_dir_not_excluded_still_flagged() {
    let all_files = vec!["src/mytests/foo.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 3);
    assert_eq!(
        flagged.len(),
        1,
        "component 'mytests' is not 'tests'; file must still be flagged"
    );
}

#[test]
fn test_suffix_files_not_flagged() {
    let suffixes = ["_tests.rs", "_test.rs", "tests.rs", "test.rs"];
    for suffix in &suffixes {
        let file = format!("src/foo{suffix}");
        let all_files = vec![file.clone()];
        let resolved: Vec<(String, String)> = vec![];
        let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 4);
        assert!(
            flagged.is_empty(),
            "file ending with {suffix} must not be flagged"
        );
    }
}

#[test]
fn my_tests_rs_is_excluded() {
    let all_files = vec!["src/my_tests.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 2);
    assert!(
        flagged.is_empty(),
        "my_tests.rs ends with _tests.rs (suffix match); must not be flagged"
    );
}

#[test]
fn lib_rs_not_flagged() {
    let all_files = vec!["src/lib.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 6);
    assert!(flagged.is_empty(), "lib.rs must not be flagged");
}

#[test]
fn lib2_rs_still_flagged() {
    let all_files = vec!["src/lib2.rs".to_string()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 1);
    assert_eq!(
        flagged.len(),
        1,
        "lib2.rs is not exactly 'lib.rs'; must still be flagged"
    );
}

// --- test-dir with `_tests` suffix (issue #734 follow-up) ----------------

#[test]
fn dir_with_tests_suffix_component_not_flagged() {
    // A path under a `*_tests` directory component (e.g. the
    // `convention_detector_tests/`, `tools_search_validation_tests/`
    // #[path]-declared module dirs) must not be flagged.
    let file = "src/analysis/convention_detector_tests/helpers.rs".to_string();
    let all_files = vec![file.clone()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 4);
    assert!(
        flagged.is_empty(),
        "file under a 'convention_detector_tests' dir component must not be flagged"
    );
}

#[test]
fn tools_search_validation_tests_dir_not_flagged() {
    let file = "src/retrieval/tools_search_validation_tests/basic.rs".to_string();
    let all_files = vec![file.clone()];
    let resolved: Vec<(String, String)> = vec![];
    let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 2);
    assert!(
        flagged.is_empty(),
        "file under a 'tools_search_validation_tests' dir must not be flagged"
    );
}

#[test]
fn filename_with_tests_substring_not_flagged() {
    // Test files named `schema_tests_migrations.rs` / `schema_tests_v7_v8.rs`
    // contain `_tests` in the middle of the filename, not as a suffix.
    for name in ["schema_tests_migrations.rs", "schema_tests_v7_v8.rs"] {
        let file = format!("src/{name}");
        let all_files = vec![file.clone()];
        let resolved: Vec<(String, String)> = vec![];
        let flagged = detect_false_zero_callers(&all_files, &resolved, 0, &no_aliases(), |_| 3);
        assert!(flagged.is_empty(), "file named {name} must not be flagged");
    }
}

// --- on_disk_grep_importer_count (word-boundary matching) ------------------

#[test]
fn on_disk_grep_finds_whole_word_reference() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("target.rs"), "pub fn helper() {}\n").unwrap();
    std::fs::write(
        dir.path().join("caller.rs"),
        "mod target;\nfn f() { target::helper(); }\n",
    )
    .unwrap();
    let all_files = vec!["target.rs".to_string(), "caller.rs".to_string()];
    let count = on_disk_grep_importer_count(dir.path(), "target.rs", &all_files);
    assert_eq!(count, 1);
}

#[test]
fn on_disk_grep_ignores_substring_within_longer_identifier() {
    // "log" must not match inside "catalogue" — word-boundary matching only.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("log.rs"), "pub fn write() {}\n").unwrap();
    std::fs::write(
        dir.path().join("other.rs"),
        "struct Catalogue { items: Vec<String> }\n",
    )
    .unwrap();
    let all_files = vec!["log.rs".to_string(), "other.rs".to_string()];
    let count = on_disk_grep_importer_count(dir.path(), "log.rs", &all_files);
    assert_eq!(
        count, 0,
        "'log' inside 'Catalogue' must not count as a reference"
    );
}

#[test]
fn on_disk_grep_excludes_generic_entry_point_stems() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
    std::fs::write(dir.path().join("helper.go"), "func main() {}\n").unwrap();
    let all_files = vec!["main.go".to_string(), "helper.go".to_string()];
    // "main" is a GENERIC_STEM — must short-circuit to 0 regardless of how
    // many other files happen to contain the word "main".
    let count = on_disk_grep_importer_count(dir.path(), "main.go", &all_files);
    assert_eq!(count, 0);
}

#[test]
fn on_disk_grep_ignores_cross_extension_name_collisions() {
    // "helper" is a common English word; a Go file named helper.go must not
    // be flagged as "referenced" just because an unrelated .rs file happens
    // to contain the word "helper" in prose or an unrelated identifier.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("helper.go"), "package main\n").unwrap();
    std::fs::write(
        dir.path().join("other.rs"),
        "let helper = helper_label();\n",
    )
    .unwrap();
    let all_files = vec!["helper.go".to_string(), "other.rs".to_string()];
    let count = on_disk_grep_importer_count(dir.path(), "helper.go", &all_files);
    assert_eq!(
        count, 0,
        "a .rs file mentioning 'helper' must not count as a Go importer"
    );
}

#[test]
fn on_disk_grep_excludes_target_file_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("target.rs"), "target self-reference\n").unwrap();
    let all_files = vec!["target.rs".to_string()];
    let count = on_disk_grep_importer_count(dir.path(), "target.rs", &all_files);
    assert_eq!(
        count, 0,
        "the target file itself must not count as an importer"
    );
}
