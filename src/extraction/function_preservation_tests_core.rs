use crate::extraction::function_preservation::preserve_functions;
use crate::model::{CodeUnit, Entity, EntityTier, RelType, Relationship};
use std::collections::HashSet;

fn make_file_entity() -> Entity {
    Entity {
        id: "file-1".to_string(),
        project_id: "proj-1".to_string(),
        repo_id: Some("repo-1".to_string()),
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "auth.rs".to_string(),
        path: Some("src/auth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("Auth module".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

fn make_function_code_unit(name: &str) -> CodeUnit {
    CodeUnit {
        name: name.to_string(),
        qualified_name: name.to_string(),
        unit_type: "function".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some(format!("fn {}() {{}}", name)),
        code: None,
        docstring: Some(format!("Validates {}", name)),
        parent_class: None,
        complexity: 3,
        has_branches: true,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    }
}

#[test]
fn test_preserve_functions_creates_entities() {
    let file = make_file_entity();
    let code_units = vec![
        make_function_code_unit("validate"),
        make_function_code_unit("format"),
    ];

    let result = preserve_functions(&file, &code_units, &HashSet::new());

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0.name, "validate");
    assert_eq!(result[1].0.name, "format");
}

#[test]
fn test_function_entity_tier_is_function() {
    let file = make_file_entity();
    let code_units = vec![make_function_code_unit("check")];

    let result = preserve_functions(&file, &code_units, &HashSet::new());

    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_function_entity_inherits_file_properties() {
    let file = make_file_entity();
    let code_units = vec![make_function_code_unit("check")];

    let result = preserve_functions(&file, &code_units, &HashSet::new());
    let function_entity = &result[0].0;

    assert_eq!(function_entity.project_id, file.project_id);
    assert_eq!(function_entity.repo_id, file.repo_id);
    assert_eq!(function_entity.parent_id, Some(file.id.clone()));
    assert_eq!(function_entity.path, file.path);
    assert_eq!(function_entity.language, Some("Rust".to_string()));
}

#[test]
fn test_contains_relationship_created() {
    let file = make_file_entity();
    let code_units = vec![make_function_code_unit("verify")];

    let result = preserve_functions(&file, &code_units, &HashSet::new());
    let (function, relationship) = &result[0];

    assert_eq!(relationship.source_id, file.id);
    assert_eq!(relationship.target_id, function.id);
    assert_eq!(relationship.rel_type, RelType::Contains);
    assert_eq!(relationship.weight, 1.0);
}

#[test]
fn test_preserve_functions_ignores_non_functions() {
    let file = make_file_entity();
    let mut unit = make_function_code_unit("my_fn");
    unit.unit_type = "variable".to_string(); // Not a function or type

    let result = preserve_functions(&file, &[unit], &HashSet::new());

    assert!(result.is_empty());
}

#[test]
fn test_preserve_functions_with_empty_code_units() {
    let file = make_file_entity();
    let result = preserve_functions(&file, &[], &HashSet::new());

    assert!(result.is_empty());
}

#[test]
fn test_function_entity_uses_unit_docstring() {
    let file = make_file_entity();
    let code_units = vec![make_function_code_unit("documented")];

    let result = preserve_functions(&file, &code_units, &HashSet::new());
    let function_entity = &result[0].0;

    assert_eq!(
        function_entity.summary,
        Some("Validates documented".to_string())
    );
}

#[test]
fn test_function_entity_serialization_roundtrip() {
    let file = make_file_entity();
    let code_units = vec![make_function_code_unit("roundtrip")];
    let result = preserve_functions(&file, &code_units, &HashSet::new());
    let (entity, rel) = &result[0];

    let json = serde_json::to_string(entity).unwrap();
    let deserialized: Entity = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, entity.id);
    assert_eq!(deserialized.tier, EntityTier::Function);
    assert_eq!(deserialized.name, entity.name);

    let rel_json = serde_json::to_string(rel).unwrap();
    let rel_deser: Relationship = serde_json::from_str(&rel_json).unwrap();

    assert_eq!(rel_deser.rel_type, rel.rel_type);
}

#[test]
fn test_issue_531_excludes_test_functions_in_production_files() {
    let file = make_file_entity();
    let code_units = vec![
        make_function_code_unit("validate"),
        make_function_code_unit("test_setup"), // Should be excluded by naming convention
        make_function_code_unit("test_validate"), // Should be excluded by naming convention
        make_function_code_unit("format"),
    ];

    let result = preserve_functions(&file, &code_units, &HashSet::new());

    // Only non-test functions should be present
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0.name, "validate");
    assert_eq!(result[1].0.name, "format");
}

#[test]
fn test_issue_531_excludes_all_functions_in_test_files() {
    // Test file: *_test.rs pattern
    let test_file = Entity {
        id: "file-1".to_string(),
        project_id: "proj-1".to_string(),
        repo_id: Some("repo-1".to_string()),
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "auth_test.rs".to_string(),
        path: Some("src/auth_test.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("Auth tests".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let code_units = vec![
        make_function_code_unit("test_validate"),
        make_function_code_unit("helper_for_tests"), // Even non-test names should be excluded
    ];

    let result = preserve_functions(&test_file, &code_units, &HashSet::new());

    // All functions in test files should be excluded
    assert!(
        result.is_empty(),
        "Functions in test files should not be preserved"
    );
}

#[test]
fn test_issue_531_excludes_functions_in_tests_directory() {
    // Test file: tests/ directory pattern
    let test_file = Entity {
        id: "file-1".to_string(),
        project_id: "proj-1".to_string(),
        repo_id: Some("repo-1".to_string()),
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "integration.rs".to_string(),
        path: Some("tests/integration.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("Integration tests".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let code_units = vec![
        make_function_code_unit("test_integration"),
        make_function_code_unit("setup"), // Even non-test names should be excluded
    ];

    let result = preserve_functions(&test_file, &code_units, &HashSet::new());

    // All functions in tests/ directory should be excluded
    assert!(
        result.is_empty(),
        "Functions in tests/ directory should not be preserved"
    );
}

#[test]
fn test_issue_531_production_function_in_production_file_included() {
    let file = make_file_entity();
    let code_units = vec![
        make_function_code_unit("process_data"),
        make_function_code_unit("validate_input"),
    ];

    let result = preserve_functions(&file, &code_units, &HashSet::new());

    // Production functions should be preserved
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0.name, "process_data");
    assert_eq!(result[1].0.name, "validate_input");
}
