use crate::extraction::function_preservation::preserve_functions;
use crate::model::{CodeUnit, Entity, EntityTier};
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

// Tests for issue #534: CamelCase entity search (structs, enums, types)

#[test]
fn test_issue_534_struct_unit_creates_entity() {
    let file = make_file_entity();
    let struct_unit = CodeUnit {
        name: "MemoryStore".to_string(),
        qualified_name: "storage::MemoryStore".to_string(),
        unit_type: "struct".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("struct MemoryStore".to_string()),
        code: None,
        docstring: Some("In-memory storage implementation".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[struct_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "MemoryStore");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_enum_unit_creates_entity() {
    let file = make_file_entity();
    let enum_unit = CodeUnit {
        name: "Permission".to_string(),
        qualified_name: "auth::Permission".to_string(),
        unit_type: "enum".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("enum Permission".to_string()),
        code: None,
        docstring: Some("Permission levels".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[enum_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "Permission");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_class_unit_creates_entity() {
    let file = make_file_entity();
    let class_unit = CodeUnit {
        name: "UserRepository".to_string(),
        qualified_name: "data::UserRepository".to_string(),
        unit_type: "class".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("class UserRepository".to_string()),
        code: None,
        docstring: Some("User data repository".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[class_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "UserRepository");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_interface_unit_creates_entity() {
    let file = make_file_entity();
    let interface_unit = CodeUnit {
        name: "StorageBackend".to_string(),
        qualified_name: "storage::StorageBackend".to_string(),
        unit_type: "interface".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("interface StorageBackend".to_string()),
        code: None,
        docstring: Some("Storage interface".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[interface_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "StorageBackend");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_trait_unit_creates_entity() {
    let file = make_file_entity();
    let trait_unit = CodeUnit {
        name: "Processor".to_string(),
        qualified_name: "pipeline::Processor".to_string(),
        unit_type: "trait".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("trait Processor".to_string()),
        code: None,
        docstring: Some("Data processing trait".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[trait_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "Processor");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_type_unit_creates_entity() {
    let file = make_file_entity();
    let type_unit = CodeUnit {
        name: "UserId".to_string(),
        qualified_name: "auth::UserId".to_string(),
        unit_type: "type".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("type UserId = String".to_string()),
        code: None,
        docstring: Some("User identifier type".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[type_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "UserId");
    assert_eq!(result[0].0.tier, EntityTier::Function);
}

#[test]
fn test_issue_534_mixed_function_and_type_units() {
    let file = make_file_entity();
    let fn_unit = CodeUnit {
        name: "validate".to_string(),
        qualified_name: "validate".to_string(),
        unit_type: "function".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("fn validate() {}".to_string()),
        code: None,
        docstring: Some("Validates validate".to_string()),
        parent_class: None,
        complexity: 3,
        has_branches: true,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };
    let struct_unit = CodeUnit {
        name: "MemoryStore".to_string(),
        qualified_name: "storage::MemoryStore".to_string(),
        unit_type: "struct".to_string(),
        file: "src/auth.rs".to_string(),
        line: 20,
        end_line: 30,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };
    let fn_unit2 = CodeUnit {
        name: "format".to_string(),
        qualified_name: "format".to_string(),
        unit_type: "function".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: Some("fn format() {}".to_string()),
        code: None,
        docstring: Some("Validates format".to_string()),
        parent_class: None,
        complexity: 3,
        has_branches: true,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };
    let code_units = vec![fn_unit, struct_unit, fn_unit2];

    let result = preserve_functions(&file, &code_units, &HashSet::new());

    assert_eq!(result.len(), 3);
    // Should preserve both functions and the struct
    let names: Vec<&str> = result.iter().map(|r| r.0.name.as_str()).collect();
    assert!(names.contains(&"validate"));
    assert!(names.contains(&"MemoryStore"));
    assert!(names.contains(&"format"));
}

#[test]
fn test_issue_534_type_entity_inherits_file_properties() {
    let file = make_file_entity();
    let struct_unit = CodeUnit {
        name: "UserManager".to_string(),
        qualified_name: "auth::UserManager".to_string(),
        unit_type: "class".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: Some("User management class".to_string()),
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[struct_unit], &HashSet::new());
    let type_entity = &result[0].0;

    assert_eq!(type_entity.project_id, file.project_id);
    assert_eq!(type_entity.repo_id, file.repo_id);
    assert_eq!(type_entity.parent_id, Some(file.id.clone()));
    assert_eq!(type_entity.path, file.path);
    assert_eq!(type_entity.language, Some("Rust".to_string()));
    assert_eq!(
        type_entity.summary,
        Some("User management class".to_string())
    );
}

#[test]
fn test_issue_534_type_entity_creates_contains_relationship() {
    let file = make_file_entity();
    let struct_unit = CodeUnit {
        name: "ConfigManager".to_string(),
        qualified_name: "config::ConfigManager".to_string(),
        unit_type: "struct".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[struct_unit], &HashSet::new());
    let (type_entity, relationship) = &result[0];

    assert_eq!(relationship.source_id, file.id);
    assert_eq!(relationship.target_id, type_entity.id);
    assert_eq!(relationship.rel_type, crate::model::RelType::Contains);
    assert_eq!(relationship.weight, 1.0);
}

#[test]
fn test_issue_534_camelcase_name_preserved() {
    let file = make_file_entity();
    let struct_unit = CodeUnit {
        name: "InMemoryCachePool".to_string(),
        qualified_name: "cache::InMemoryCachePool".to_string(),
        unit_type: "struct".to_string(),
        file: "src/auth.rs".to_string(),
        line: 10,
        end_line: 20,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    };

    let result = preserve_functions(&file, &[struct_unit], &HashSet::new());

    assert_eq!(result.len(), 1);
    // CamelCase name should be preserved as-is (not converted to lowercase)
    assert_eq!(result[0].0.name, "InMemoryCachePool");
}
