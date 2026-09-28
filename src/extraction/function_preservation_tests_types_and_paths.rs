use crate::extraction::function_preservation::is_test_file_path;

// Tests for issue #547: Test file path detection fixes

#[test]
fn test_issue_547_nested_tests_directory_detected() {
    // Bug fix: src/memory/tests/ingest.rs should be detected as a test file
    assert!(is_test_file_path("src/memory/tests/ingest.rs"));
    assert!(is_test_file_path("src/storage/test/main.rs"));
    assert!(is_test_file_path("lib/api/spec/endpoints_test.py"));
}

#[test]
fn test_issue_547_plural_tests_filename_detected() {
    // Bug fix: batch_tests.rs should be detected as a test file
    assert!(is_test_file_path("src/memory/batch_tests.rs"));
    assert!(is_test_file_path("src/db/integration_tests.rs"));
    assert!(is_test_file_path("lib/unit_tests.py"));
}

#[test]
fn test_issue_547_production_file_not_false_positive() {
    // Ensure no false positives for production files
    assert!(!is_test_file_path("src/memory/batch.rs"));
    assert!(!is_test_file_path("src/storage/database.rs"));
    assert!(!is_test_file_path("src/utilities/helpers.rs"));
    assert!(!is_test_file_path("latest/version.json"));
    assert!(!is_test_file_path("contest/scoring.rs"));
}

#[test]
fn test_issue_547_existing_tests_directory_still_detected() {
    // Regression test: existing tests/ detection should still work
    assert!(is_test_file_path("tests/lib_integration.rs"));
    assert!(is_test_file_path("tests/integration.rs"));
    assert!(is_test_file_path("test/main.rs"));
    assert!(is_test_file_path("spec/helpers.rb"));
    assert!(is_test_file_path("__tests__/utils.test.js"));
}

#[test]
fn test_issue_547_multiple_bug_fixes_together() {
    // Test all acceptance criteria from issue #547
    assert!(
        is_test_file_path("src/memory/tests/ingest.rs"),
        "Nested tests/ dir should be detected"
    );
    assert!(
        is_test_file_path("src/memory/batch_tests.rs"),
        "Plural _tests.rs filename should be detected"
    );
    assert!(
        !is_test_file_path("src/utilities/helpers.rs"),
        "Production file should not be false positive"
    );
    assert!(
        is_test_file_path("tests/lib_integration.rs"),
        "Existing tests/ detection should still work"
    );
}

// Tests for issue #562: tests_ prefix (plural) not matched by starts_with

#[test]
fn test_issue_562_tests_prefix_not_matched() {
    // Bug fix: tests_utils.rs starts with "tests_" (plural), not "test_" (singular)
    // The original code only checked starts_with("test_") but NOT starts_with("tests_")
    assert!(
        is_test_file_path("src/config/tests_utils.rs"),
        "tests_utils.rs should be detected (starts with tests_)"
    );
    assert!(
        is_test_file_path("src/tests_helpers.rs"),
        "tests_helpers.rs should be detected (starts with tests_)"
    );
    assert!(
        is_test_file_path("tests_common.rs"),
        "tests_common.rs should be detected (starts with tests_)"
    );
}

// Tests for issue #549 Part B: Test file path detection for "tests.rs", "test.rs", "spec.rs"

#[test]
fn test_issue_549_part_b_tests_rs_detected() {
    // Bug fix: src/mcp/tests.rs should be detected as a test file
    assert!(
        is_test_file_path("src/mcp/tests.rs"),
        "File named 'tests.rs' should be detected as test file"
    );
    assert!(
        is_test_file_path("src/sqlite/tests.rs"),
        "File named 'tests.rs' in nested path should be detected"
    );
    assert!(
        is_test_file_path("lib/foo/tests.rs"),
        "File named 'tests.rs' in lib/ should be detected"
    );
}

#[test]
fn test_issue_549_part_b_test_rs_detected() {
    // Bug fix: test.rs detection
    assert!(
        is_test_file_path("src/mcp/test.rs"),
        "File named 'test.rs' should be detected as test file"
    );
    assert!(
        is_test_file_path("src/sqlite/test.rs"),
        "File named 'test.rs' in nested path should be detected"
    );
    assert!(
        is_test_file_path("lib/foo/test.rs"),
        "File named 'test.rs' in lib/ should be detected"
    );
}

#[test]
fn test_issue_549_part_b_spec_rs_detected() {
    // Bug fix: spec.rs detection
    assert!(
        is_test_file_path("src/mcp/spec.rs"),
        "File named 'spec.rs' should be detected as test file"
    );
    assert!(
        is_test_file_path("src/sqlite/spec.rs"),
        "File named 'spec.rs' in nested path should be detected"
    );
    assert!(
        is_test_file_path("lib/foo/spec.rs"),
        "File named 'spec.rs' in lib/ should be detected"
    );
}

#[test]
fn test_issue_549_part_b_no_false_positives() {
    // Ensure production files with "test" in path are not false positives
    assert!(
        !is_test_file_path("src/contest/scoring.rs"),
        "Production file with 'contest' in name should not be detected"
    );
    assert!(
        !is_test_file_path("src/attestation/verify.rs"),
        "Production file with 'attestation' in name should not be detected"
    );
    assert!(
        !is_test_file_path("src/testimonials/list.rs"),
        "Production file with 'testimonials' in name should not be detected"
    );
}

#[test]
fn test_issue_549_part_b_stem_stripping_works_correctly() {
    // Verify the stem stripping logic works by testing with different extensions
    assert!(
        is_test_file_path("src/mcp/tests.rs"),
        "Path with .rs extension should match 'tests'"
    );
    assert!(
        is_test_file_path("src/mcp/tests.py"),
        "Path with .py extension should match 'tests'"
    );
    assert!(
        is_test_file_path("src/mcp/spec.js"),
        "Path with .js extension should match 'spec'"
    );
    assert!(
        is_test_file_path("src/mcp/test.go"),
        "Path with .go extension should match 'test'"
    );
}

#[test]
fn test_issue_549_part_b_double_extension_handling() {
    // Test that the split logic handles extensions correctly
    assert!(
        is_test_file_path("src/mcp/tests.test.js"),
        "Double extension should work: tests.test.js -> tests"
    );
    assert!(
        is_test_file_path("src/mcp/test.spec.rs"),
        "Double extension should work: test.spec.rs -> test"
    );
    assert!(
        !is_test_file_path("src/mcp/helpers.prod.rs"),
        "Non-test stem should not match: helpers.prod.rs -> helpers"
    );
}
