use crate::extraction::function_preservation::extract_cfg_test_functions;
use crate::extraction::function_preservation::preserve_functions;
use crate::model::{CodeUnit, Entity, EntityTier};
use std::collections::HashSet;

#[test]
fn test_extract_cfg_test_functions_simple() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
pub fn real_function() -> i32 {
    42
}

#[cfg(test)]
fn helper() {
    // Test helper
}

#[cfg(test)]
pub async fn setup() {
    // Async test helpers
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 2);
    assert!(cfg_test_fns.contains("helper"));
    assert!(cfg_test_fns.contains("setup"));
    assert!(!cfg_test_fns.contains("real_function"));
}

#[test]
fn test_extract_cfg_test_functions_with_pub() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
pub fn test_helper() {
    // Public test helper
}

#[cfg(test)]
pub fn create_test_fixture() -> TestData {
    TestData::new()
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 2);
    assert!(cfg_test_fns.contains("test_helper"));
    assert!(cfg_test_fns.contains("create_test_fixture"));
}

#[test]
fn test_extract_cfg_test_functions_no_match() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
pub fn production_function() -> i32 {
    42
}

private fn internal_helper() {
    // Internal helper
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert!(cfg_test_fns.is_empty());
}

#[test]
fn test_extract_cfg_test_functions_non_existent_file() {
    let file_path = "/non/existent/file.rs";
    let repo_root = std::path::Path::new("/");

    let cfg_test_fns = extract_cfg_test_functions(file_path, repo_root);

    assert!(cfg_test_fns.is_empty());
}

#[test]
fn test_extract_cfg_test_functions_multiple_cfg_blocks() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
fn helper1() {}

#[cfg(test)]
pub fn helper2() {}

#[cfg(test)]
async fn helper3() {}

#[cfg(test)]
pub async fn helper4() {}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 4);
    assert!(cfg_test_fns.contains("helper1"));
    assert!(cfg_test_fns.contains("helper2"));
    assert!(cfg_test_fns.contains("helper3"));
    assert!(cfg_test_fns.contains("helper4"));
}

#[test]
fn test_extract_cfg_test_functions_resolves_relative_path() {
    let temp = tempfile::tempdir().unwrap();
    // Create subdirectory structure matching relative path
    let src_dir = temp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let file_path = src_dir.join("module.rs");

    let source = r#"
pub fn production_fn() {}

#[cfg(test)]
fn test_helper() {}
"#;
    std::fs::write(&file_path, source).unwrap();

    // Pass as relative path "src/module.rs" with temp as repo_root
    let cfg_test_fns = extract_cfg_test_functions("src/module.rs", temp.path());

    assert_eq!(cfg_test_fns.len(), 1);
    assert!(cfg_test_fns.contains("test_helper"));
    // production_fn should NOT be in cfg_test_fns
    assert!(!cfg_test_fns.contains("production_fn"));
}

#[test]
fn test_issue_537_cfg_test_functions_excluded_in_production() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("production.rs");

    let source = r#"
pub fn my_function() -> i32 {
    42
}

#[cfg(test)]
fn create_test_memory() -> Memory {
    Memory::new()
}

#[cfg(test)]
fn insert(db: &mut Database, data: &str) -> Result<()> {
    db.insert(data)
}
"#;

    std::fs::write(&file_path, source).unwrap();

    // Create a production file entity
    let file_entity = Entity {
        id: "file-1".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "production.rs".to_string(),
        path: Some(file_path.to_str().unwrap().to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    // Create code units for all three functions
    let code_units = vec![
        CodeUnit {
            name: "my_function".to_string(),
            qualified_name: "my_function".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_str().unwrap().to_string(),
            line: 2,
            end_line: 4,
            language: "Rust".to_string(),
            signature: Some("fn my_function() -> i32".to_string()),
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
        },
        CodeUnit {
            name: "create_test_memory".to_string(),
            qualified_name: "create_test_memory".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_str().unwrap().to_string(),
            line: 7,
            end_line: 9,
            language: "Rust".to_string(),
            signature: Some("fn create_test_memory() -> Memory".to_string()),
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
        },
        CodeUnit {
            name: "insert".to_string(),
            qualified_name: "insert".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_str().unwrap().to_string(),
            line: 11,
            end_line: 13,
            language: "Rust".to_string(),
            signature: Some("fn insert(...)".to_string()),
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
        },
    ];

    // Extract cfg(test) functions (as the orchestration layer would do)
    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());
    assert!(cfg_test_fns.contains("create_test_memory"));
    assert!(cfg_test_fns.contains("insert"));

    let result = preserve_functions(&file_entity, &code_units, &cfg_test_fns);

    // Only my_function should be preserved, cfg(test) helpers excluded
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0.name, "my_function");
}

#[test]
fn test_issue_537_test_file_still_excluded_wholesale() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test_module_test.rs");

    let source = r#"
#[cfg(test)]
fn helper() {}

pub fn also_a_helper() {}
"#;

    std::fs::write(&file_path, source).unwrap();

    // Create a test file entity (ends with _test.rs)
    let test_file = Entity {
        id: "file-1".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "test_module_test.rs".to_string(),
        path: Some(file_path.to_str().unwrap().to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let code_units = vec![
        CodeUnit {
            name: "helper".to_string(),
            qualified_name: "helper".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_str().unwrap().to_string(),
            line: 2,
            end_line: 4,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
        },
        CodeUnit {
            name: "also_a_helper".to_string(),
            qualified_name: "also_a_helper".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_str().unwrap().to_string(),
            line: 6,
            end_line: 8,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
        },
    ];

    let result = preserve_functions(&test_file, &code_units, &HashSet::new());

    // All functions in test files should be excluded (no regression)
    assert!(
        result.is_empty(),
        "Functions in test files should still be excluded wholesale"
    );
}
