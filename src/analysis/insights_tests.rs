// Tests for insights.rs - InsightDetector coverage gap detection and helpers

use crate::analysis::insights::{InsightDetector, is_test_file};
use crate::model::{Entity, EntityTier};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

fn make_entity(id: &str, path: Option<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: Some("repo".to_string()),
        tier: EntityTier::File,
        parent_id: Some("mod".to_string()),
        name: id.to_string(),
        path: path.map(|s| s.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

fn make_detector<'a>(storage: &'a SqliteStorage, project_id: &'a str) -> InsightDetector<'a> {
    InsightDetector::new(storage, project_id)
}

fn test_entity(id: &str, path: &str, language: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "test".to_string(),
        path: Some(path.to_string()),
        language: Some(language.to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_is_test_file_non_rust_file_affected() {
    // Non-Rust files should use filename pattern matching
    let entity = make_entity("test_foo.py", Some("tests/test_foo.py"));
    assert!(
        is_test_file(&entity),
        "Python test file should be recognized by filename pattern"
    );

    let entity = make_entity("foo.py", Some("src/foo.py"));
    assert!(
        !is_test_file(&entity),
        "Non-test Python file should not be recognized"
    );
}

#[test]
fn test_is_test_file_rust_test_file_by_name() {
    // Rust test files detected by name should still work
    let entity = make_entity("lib_test", Some("tests/lib_test.rs"));
    assert!(
        is_test_file(&entity),
        "Rust test file detected by name pattern"
    );
}

#[test]
fn test_is_test_file_common_test_directories() {
    // Files in common test directories are recognized
    let entity = make_entity("integration", Some("/project/tests/integration.rs"));
    assert!(
        is_test_file(&entity),
        "File in /tests/ should be recognized as test file"
    );

    let entity = make_entity("spec_helper", Some("/project/spec/spec_helper.rb"));
    assert!(
        is_test_file(&entity),
        "File in /spec/ should be recognized as test file"
    );

    let entity = make_entity("unit", Some("/project/__tests__/unit.rs"));
    assert!(
        is_test_file(&entity),
        "File in /__tests__/ should be recognized as test file"
    );
}

#[test]
fn test_is_test_file_spec_patterns() {
    let entity = make_entity("foo_spec.rb", Some("spec/models/foo_spec.rb"));
    assert!(
        is_test_file(&entity),
        "foo_spec.rb should be recognized as test file"
    );

    let entity = make_entity("spec_foo.rb", Some("spec/models/spec_foo.rb"));
    assert!(
        is_test_file(&entity),
        "spec_foo.rb should be recognized as test file"
    );
}

#[test]
fn file_has_inline_tests_returns_false_when_no_repo_root() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let detector = make_detector(&storage, &project.id);
    let entity = test_entity("e1", "src/lib.rs", "Rust");
    assert!(!detector.file_has_inline_tests(&entity));
}

#[test]
fn file_has_inline_tests_returns_false_for_non_rust_file() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test2", None).unwrap();
    let detector =
        make_detector(&storage, &project.id).with_repo_root(std::path::Path::new("/some/path"));
    let entity = test_entity("e2", "app.py", "Python");
    assert!(!detector.file_has_inline_tests(&entity));
}

/// Regression test for issue #564: modules containing only non-testable files
/// (YAML/TOML/Markdown) should NOT get a coverage gap insight.
#[test]
fn detect_coverage_gaps_skips_non_testable_language_modules() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test3", None).unwrap();

    // Create a module entity
    let module = Entity {
        id: "mod-github".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: ".github".to_string(),
        path: Some(".github".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    // Create 3 YAML file children (simulates .github/workflows/*.yml)
    let yaml_file1 = Entity {
        id: "f-yaml-1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-github".to_string()),
        name: "ci.yml".to_string(),
        path: Some(".github/workflows/ci.yml".to_string()),
        language: Some("YAML".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let yaml_file2 = Entity {
        id: "f-yaml-2".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-github".to_string()),
        name: "cd.yml".to_string(),
        path: Some(".github/workflows/cd.yml".to_string()),
        language: Some("yaml".to_string()), // lowercase variant
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let yaml_file3 = Entity {
        id: "f-yaml-3".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-github".to_string()),
        name: "issue.yml".to_string(),
        path: Some(".github/ISSUE_TEMPLATES/issue.yml".to_string()),
        language: Some("YAML".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&yaml_file1).unwrap();
    storage.upsert_entity(&yaml_file2).unwrap();
    storage.upsert_entity(&yaml_file3).unwrap();

    let detector = make_detector(&storage, &project.id);
    let insights = detector.detect().unwrap();

    // .github module with only YAML files should NOT produce a coverage gap insight
    let coverage_gaps: Vec<_> = insights
        .into_iter()
        .filter(|i| i.category == "coverage_gap")
        .collect();

    assert!(
        coverage_gaps.is_empty(),
        "YAML-only module '.github' should not get a coverage gap insight, but got: {coverage_gaps:?}"
    );
}

/// Regression test for issue #569: modules where all files have language `None`
/// should also be skipped, since we can't determine testability.
#[test]
fn detect_coverage_gaps_skips_none_language_modules() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test4", None).unwrap();

    // Create a module entity
    let module = Entity {
        id: "mod-unknown".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "unknown-stuff".to_string(),
        path: Some("unknown-stuff".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    // Create 2 file children with language None (simulates files the extractor couldn't classify)
    let file1 = Entity {
        id: "f-none-1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-unknown".to_string()),
        name: "weird-file.abc".to_string(),
        path: Some("unknown-stuff/weird-file.abc".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let file2 = Entity {
        id: "f-none-2".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-unknown".to_string()),
        name: "another.xyz".to_string(),
        path: Some("unknown-stuff/another.xyz".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file1).unwrap();
    storage.upsert_entity(&file2).unwrap();

    let detector = make_detector(&storage, &project.id);
    let insights = detector.detect().unwrap();

    // Module with only None-language files should NOT produce a coverage gap insight
    let coverage_gaps: Vec<_> = insights
        .into_iter()
        .filter(|i| i.category == "coverage_gap")
        .collect();

    assert!(
        coverage_gaps.is_empty(),
        "None-language module 'unknown-stuff' should not get a coverage gap insight, but got: {coverage_gaps:?}"
    );
}
