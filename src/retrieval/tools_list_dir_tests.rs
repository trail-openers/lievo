// ListDirectoryTool tests — split from tools_tests.rs to keep all test files under 500 lines.
// Included via `#[path = "tools_list_dir_tests.rs"] mod list_dir_tests;` in tools_impl.rs.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

use super::super::{ListDirectoryTool, ToolContext};

fn setup_storage() -> (SqliteStorage, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project-ld", None).unwrap();
    (storage, project.id)
}

fn make_ctx(storage: SqliteStorage, project_id: String) -> Arc<ToolContext<SqliteStorage>> {
    Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    })
}

fn make_ctx_with_repo(
    storage: SqliteStorage,
    project_id: String,
    repo_path: std::path::PathBuf,
) -> Arc<ToolContext<SqliteStorage>> {
    Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path,
        output_dir: None,
        zero_repo_guidance: None,
    })
}

#[test]
fn test_list_directory_returns_entries() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("alpha.rb"), "").unwrap();
    fs::write(tmp.path().join("beta.rb"), "").unwrap();
    fs::create_dir(tmp.path().join("subdir")).unwrap();

    let (storage, project_id) = setup_storage();
    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": ""})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed.get("error").is_none(), "unexpected error: {parsed}");
    assert_eq!(parsed["path"], "");
    let entries = parsed["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .filter_map(|v| v.get("path").and_then(|p| p.as_str()))
        .collect();
    assert!(names.contains(&"alpha.rb"), "missing alpha.rb in {names:?}");
    assert!(names.contains(&"beta.rb"), "missing beta.rb in {names:?}");
    assert!(names.contains(&"subdir/"), "missing subdir/ in {names:?}");
    assert_eq!(parsed["count"], 3);
}

#[test]
fn test_list_directory_subdirs_have_trailing_slash() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("mydir")).unwrap();
    fs::write(tmp.path().join("myfile.txt"), "").unwrap();

    let (storage, project_id) = setup_storage();
    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": ""})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    let entries = parsed["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .filter_map(|v| v.get("path").and_then(|p| p.as_str()))
        .collect();

    assert!(
        names.contains(&"mydir/"),
        "directory entry must end with '/': {names:?}"
    );
    assert!(
        names.contains(&"myfile.txt"),
        "file entry must not end with '/': {names:?}"
    );
}

#[test]
fn test_list_directory_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": "nonexistent_dir_xyz"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("path not found"),
        "expected 'path not found', got: {err}"
    );
}

#[test]
fn test_list_directory_path_traversal_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": "../secret"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("path traversal not allowed"),
        "expected traversal rejection, got: {err}"
    );
}

#[test]
fn test_list_directory_path_is_file_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("routes.rb"),
        "Rails.application.routes.draw do end",
    )
    .unwrap();

    let (storage, project_id) = setup_storage();
    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": "routes.rb"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("path is a file"),
        "expected 'path is a file', got: {err}"
    );
}

#[test]
fn test_list_directory_empty_repo_path() {
    let (storage, project_id) = setup_storage();
    // make_ctx() uses PathBuf::new() — the empty path triggers "not configured".
    let ctx = make_ctx(storage, project_id);
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": "src"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("repo path not configured"),
        "expected 'repo path not configured', got: {err}"
    );
}

// Test that root-level files match entity_id with "./" prefix fallback (issue #473)
#[test]
fn test_list_directory_root_file_entity_id_fallback() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("README.md"), "# Readme").unwrap();

    let (storage, project_id) = setup_storage();

    // Add a repo and an entity with "./" prefix path
    let repo = storage
        .add_repo(&project_id, "test-repo", tmp.path().to_str().unwrap())
        .unwrap();
    let repo_id = &repo.id;

    // Create an entity with "./README.md" path (common in indexed files)
    let entity = crate::model::Entity {
        id: "entity-readme".to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: crate::model::EntityTier::File,
        parent_id: None,
        name: "README.md".to_string(),
        path: Some("./README.md".to_string()),
        language: Some("Markdown".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    // List the root directory (empty path)
    let result = tool.call(json!({"path": ""})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed.get("error").is_none(), "unexpected error: {parsed}");

    // Find the README.md entry
    let entries = parsed["entries"].as_array().unwrap();
    let readme_entry = entries
        .iter()
        .find(|e| e["path"] == "README.md")
        .expect("README.md entry should exist");

    // Verify entity_id is populated via fallback
    assert_eq!(
        readme_entry["entity_id"], "entity-readme",
        "entity_id should be populated via './' fallback for root-level files"
    );
}

// Test that entries have indexed field set correctly (issue #533)
#[test]
fn test_list_directory_indexed_field() {
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("README.md"), "# Readme").unwrap();
    fs::write(tmp.path().join("LICENSE"), "MIT").unwrap();
    fs::create_dir(tmp.path().join("src")).unwrap();

    let (storage, project_id) = setup_storage();

    // Add a repo
    let repo = storage
        .add_repo(&project_id, "test-repo", tmp.path().to_str().unwrap())
        .unwrap();
    let repo_id = &repo.id;

    // Only create an entity for README.md (LICENSE intentionally excluded)
    let entity = crate::model::Entity {
        id: "entity-readme".to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: crate::model::EntityTier::File,
        parent_id: None,
        name: "README.md".to_string(),
        path: Some("README.md".to_string()),
        language: Some("Markdown".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx_with_repo(storage, project_id, tmp.path().to_path_buf());
    let tool = ListDirectoryTool { ctx };

    let result = tool.call(json!({"path": ""})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(parsed.get("error").is_none(), "unexpected error: {parsed}");

    let entries = parsed["entries"].as_array().unwrap();

    // Find README.md entry - should have indexed: true
    let readme_entry = entries
        .iter()
        .find(|e| e["path"] == "README.md")
        .expect("README.md entry should exist");
    assert_eq!(
        readme_entry["entity_id"], "entity-readme",
        "README.md should have entity_id"
    );
    assert_eq!(
        readme_entry["indexed"], true,
        "README.md should have indexed: true"
    );

    // Find LICENSE entry - should have indexed: false (no entity_id)
    let license_entry = entries
        .iter()
        .find(|e| e["path"] == "LICENSE")
        .expect("LICENSE entry should exist");
    assert_eq!(
        license_entry["entity_id"],
        serde_json::Value::Null,
        "LICENSE should have entity_id: null"
    );
    assert_eq!(
        license_entry["indexed"], false,
        "LICENSE should have indexed: false"
    );

    // Find src/ directory - should have indexed: false (directories not indexed)
    let src_entry = entries
        .iter()
        .find(|e| e["path"] == "src/")
        .expect("src/ entry should exist");
    assert_eq!(
        src_entry["indexed"], false,
        "src/ directory should have indexed: false"
    );
}
