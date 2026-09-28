#[cfg(test)]
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::storage::{Storage, sqlite::SqliteStorage};
use std::sync::{Arc, Mutex};

#[cfg(test)]
pub fn setup_storage() -> (SqliteStorage, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    (storage, project.id)
}

#[cfg(test)]
pub fn make_ctx(storage: SqliteStorage, project_id: String) -> Arc<ToolContext<SqliteStorage>> {
    Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    })
}

#[cfg(test)]
pub fn test_entity(id: &str, name: &str, path: Option<&str>, project_id: &str) -> Entity {
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
