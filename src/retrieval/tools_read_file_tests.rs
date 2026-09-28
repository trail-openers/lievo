// ReadFileTool tests — split from tools_tests.rs to keep both files under 500 lines.
// Included via `#[path = "tools_read_file_tests.rs"] mod read_file_tests;` in tools_impl.rs.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

use super::super::{ReadFileTool, ToolContext};

fn setup_storage() -> (SqliteStorage, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project-rf", None).unwrap();
    (storage, project.id)
}

fn test_file_entity(id: &str, name: &str, path: Option<&str>, project_id: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: path.map(|s| s.to_string()),
        language: Some("Rust".to_string()),
        summary: Some(format!("Summary of {name}")),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
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

fn make_ctx_with_repo_path(
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
fn test_read_file_entity_not_found() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tool = ReadFileTool { ctx };

    let result = tool.call(json!({"entity_id": "missing-id"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(
        parsed["error"]
            .as_str()
            .unwrap()
            .contains("entity not found"),
        "expected 'entity not found' in error, got: {parsed}"
    );
}

#[test]
fn test_read_file_non_file_tier() {
    let (storage, project_id) = setup_storage();
    let module_entity = Entity {
        tier: EntityTier::Module,
        ..test_file_entity("mod1", "my_module", Some("src/my_module"), &project_id)
    };
    storage.upsert_entity(&module_entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = ReadFileTool { ctx };

    let result = tool.call(json!({"entity_id": "mod1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(err.contains("is not a file"), "got: {err}");
    assert!(err.contains("tier: module"), "got: {err}");
}

#[test]
fn test_read_file_returns_content() {
    use std::io::Write;

    let (storage, project_id) = setup_storage();

    let tmp_dir = tempfile::tempdir().unwrap();
    let rel_path = "hello.rs";
    let mut f = std::fs::File::create(tmp_dir.path().join(rel_path)).unwrap();
    writeln!(f, "fn main() {{ println!(\"hello\"); }}").unwrap();

    storage
        .upsert_entity(&test_file_entity(
            "fe1",
            "hello",
            Some(rel_path),
            &project_id,
        ))
        .unwrap();

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: tmp_dir.path().to_path_buf(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ReadFileTool { ctx };

    let result = tool.call(json!({"entity_id": "fe1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["entity_id"], "fe1");
    assert_eq!(parsed["name"], "hello");
    assert_eq!(parsed["path"], rel_path);
    assert!(
        parsed["content"].as_str().unwrap().contains("fn main()"),
        "expected file content in result"
    );
}

#[test]
fn test_read_file_file_not_on_disk() {
    // Entity exists in storage with a path that doesn't correspond to a real file on disk.
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_file_entity(
            "ghost1",
            "ghost_file",
            Some("definitely_does_not_exist_12345.rs"),
            &project_id,
        ))
        .unwrap();

    // Use /tmp as repo_path so canonicalize() on it succeeds, but the joined
    // path won't exist on disk.
    let ctx = make_ctx_with_repo_path(storage, project_id, std::path::PathBuf::from("/tmp"));
    let tool = ReadFileTool { ctx };

    let result = tool.call(json!({"entity_id": "ghost1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("file not found on disk"),
        "expected 'file not found on disk', got: {}",
        err
    );
}

#[test]
fn test_read_file_empty_repo_path_returns_not_configured() {
    // When repo_path is empty (chat mode), canonicalize() fails and we return
    // a "not configured" error rather than panicking or leaking filesystem info.
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_file_entity(
            "f1",
            "some_file",
            Some("src/lib.rs"),
            &project_id,
        ))
        .unwrap();

    // make_ctx() uses PathBuf::new() — the empty path exercises the chat-mode guard.
    let ctx = make_ctx(storage, project_id);
    let tool = ReadFileTool { ctx };

    let result = tool.call(json!({"entity_id": "f1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let err = parsed["error"].as_str().unwrap();
    assert!(
        err.contains("repo path not configured"),
        "expected 'repo path not configured', got: {err}"
    );
}
