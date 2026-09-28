use super::helpers::*;
use crate::analysis::convention_detector::ConventionDetector;
use crate::model::EntityTier;

#[test]
fn test_detect_test_pattern_priority_directory_over_suffix() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "foo_test.rs",
            Some("tests/foo_test.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "bar_test.rs",
            Some("tests/bar_test.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector
        .detect_test_pattern(&storage.entities, "proj1")
        .unwrap();
    assert!(result.title.contains("tests/ directory"));
    assert_eq!(result.confidence, 1.0);
}

#[test]
fn test_detect_test_pattern_test_directory() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "test_math.rs",
            Some("test/test_math.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "test_utils.rs",
            Some("test/test_utils.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    assert!(result.is_some());
    assert!(result.unwrap().title.contains("test/ directory"));
}

#[test]
fn test_detect_test_pattern_test_prefix() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "test_math.rs",
            Some("src/test_math.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "test_utils.rs",
            Some("src/test_utils.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    assert!(result.is_some());
    let conv = result.unwrap();
    assert!(conv.title.contains("test_ prefix"));
    assert_eq!(conv.confidence, 1.0);
}

#[test]
fn test_detect_test_pattern_test_suffix() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "math_test.rs",
            Some("src/math_test.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "utils_test.rs",
            Some("src/utils_test.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    assert!(result.is_some());
    assert!(result.unwrap().title.contains("_test.rs suffix"));
}

#[test]
fn test_detect_test_pattern_tests_suffix() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "math_tests.rs",
            Some("src/math_tests.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "utils_tests.rs",
            Some("src/utils_tests.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    assert!(result.is_some());
    assert!(result.unwrap().title.contains("_tests.rs suffix"));
}

#[test]
fn test_detect_module_organization_flat_structure() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "lib.rs",
            Some("src/lib.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "utils.rs",
            Some("src/utils.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    assert!(result.is_some());
    let conv = result.unwrap();
    assert_eq!(conv.category, "structure");
    assert!(conv.title.contains("flat"));
    assert_eq!(conv.confidence, 0.95);
}

#[test]
fn test_detect_module_organization_nested_structure() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "login.rs",
            Some("src/features/auth/login.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "register.rs",
            Some("src/features/auth/register.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "permissions.rs",
            Some("src/features/rbac/permissions.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    assert!(result.is_some());
    let conv = result.unwrap();
    assert!(conv.title.contains("nested"));
    assert!(conv.confidence > 0.7);
}

#[test]
fn test_detect_module_organization_very_nested_structure() {
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "a.rs",
            Some("src/a/b/c/d/a.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "b.rs",
            Some("src/a/b/c/d/b.rs"),
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "c.rs",
            Some("src/a/b/c/d/c.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    assert!(result.is_some());
    let conv = result.unwrap();
    assert!(conv.title.contains("nested"));
    assert_eq!(conv.confidence, 0.9);
}

// EDGE CASE TESTS: Restore deleted tests for production code branches

#[test]
fn test_detect_test_pattern_no_test_files() {
    // Edge case: zero test files should return None
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "lib.rs",
            Some("src/lib.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    // Not enough test files to establish a pattern
    assert!(result.is_none());
}

#[test]
fn test_detect_test_pattern_only_one_test_file() {
    // Edge case: exactly one test file should return None (need >=2 to establish pattern)
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "math_test.rs",
            Some("src/math_test.rs"),
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "main.rs",
            Some("src/main.rs"),
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_test_pattern(&storage.entities, "proj1");
    // Not enough test files (only 1, need >= 2)
    assert!(result.is_none());
}

#[test]
fn test_detect_module_organization_mixed_no_pattern() {
    // Edge case: avg depth strictly between 2.0 and 3.0 (mixed structure) returns None
    // depth values: 1 ("src/a.rs"), 2 ("src/features/b.rs"), 2 ("src/utils/c.rs")
    // avg = (1 + 2 + 2) / 3 = 1.666... which is <= 2.0, so we need higher depths
    // Let's use: 1, 3, 3 -> avg = 2.333... which is > 2.0 and < 3.0
    let entities = vec![
        make_entity(
            "f1",
            "proj1",
            EntityTier::File,
            "a.rs",
            Some("src/a.rs"), // depth=1
        ),
        make_entity(
            "f2",
            "proj1",
            EntityTier::File,
            "b.rs",
            Some("src/features/auth/b.rs"), // depth=3
        ),
        make_entity(
            "f3",
            "proj1",
            EntityTier::File,
            "c.rs",
            Some("src/features/rbac/c.rs"), // depth=3
        ),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    // avg_depth = (1 + 3 + 3) / 3 = 2.333, which is > 2.0 and < 3.0, so returns None
    assert!(result.is_none());
}

#[test]
fn test_detect_module_organization_empty_entities() {
    // Edge case: empty entity list should return None
    let entities = vec![];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    assert!(result.is_none());
}

#[test]
fn test_detect_module_organization_no_file_entities() {
    // Edge case: only non-File entities (e.g., functions) should return None
    let entities = vec![
        make_entity(
            "fn1",
            "proj1",
            EntityTier::Function,
            "get_user",
            None, // No path
        ),
        make_entity("fn2", "proj1", EntityTier::Function, "set_config", None),
    ];
    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_module_organization(&storage.entities, "proj1");
    // No File entities, so depths is empty
    assert!(result.is_none());
}
