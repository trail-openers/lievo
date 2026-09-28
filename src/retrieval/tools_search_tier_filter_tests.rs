// SearchEntitiesTool tier filtering functionality tests (issue #557).

use super::*;
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use serde_json::json;
use std::sync::{Arc, Mutex};

/// A MockStorage that simulates tier-filtered search for testing
struct TierFilteredMockStorage {
    entities: Vec<Entity>,
    search_tier: std::cell::RefCell<Option<String>>,
}

impl TierFilteredMockStorage {
    fn with_entities(entities: Vec<Entity>) -> Self {
        Self {
            entities,
            search_tier: std::cell::RefCell::new(None),
        }
    }
}

impl Clone for TierFilteredMockStorage {
    fn clone(&self) -> Self {
        Self {
            entities: self.entities.clone(),
            search_tier: std::cell::RefCell::new(self.search_tier.borrow().clone()),
        }
    }
}

impl crate::storage::Storage for TierFilteredMockStorage {
    fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        Ok(self.entities.clone())
    }

    fn search_entities_by_name(
        &self,
        _: &str,
        words: &[&str],
        _: usize,
        tier: Option<&str>,
    ) -> crate::Result<Vec<Entity>> {
        *self.search_tier.borrow_mut() = tier.map(String::from);
        let results: Vec<Entity> = self
            .entities
            .iter()
            .filter(|e| {
                // Filter by tier if specified
                if let Some(t) = tier
                    && e.tier.to_string() != t
                {
                    return false;
                }
                // Then filter by word match
                words.iter().any(|w| {
                    let w_lower = w.to_lowercase();
                    e.name.to_lowercase().contains(&w_lower)
                        || e.path
                            .as_ref()
                            .map(|p| p.to_lowercase().contains(&w_lower))
                            .unwrap_or(false)
                })
            })
            .cloned()
            .collect();
        Ok(results)
    }

    fn entity_by_path(&self, _: &str, _: &str) -> crate::Result<Option<Entity>> {
        Ok(None)
    }

    fn entity_ids_for_paths(
        &self,
        _: &str,
        _: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }

    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }

    fn list_repos(&self, _: &str) -> crate::Result<Vec<crate::model::Repository>> {
        Ok(vec![])
    }

    fn get_entity(&self, _: &str) -> crate::Result<Option<Entity>> {
        Ok(None)
    }
    fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<crate::model::Project> {
        unimplemented!()
    }
    fn get_project(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        Ok(None)
    }
    fn get_project_by_id(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        Ok(None)
    }
    fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
        Ok(vec![])
    }
    fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<crate::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _: &str) -> crate::Result<Option<crate::model::Repository>> {
        Ok(None)
    }
    fn update_repo_index_path(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        Ok(Default::default())
    }
    fn upsert_entity(&self, _: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn entities_by_repo(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_parent(&self, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _: &str) -> crate::Result<u64> {
        Ok(0)
    }
    fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
        Ok(0)
    }
    fn upsert_relationship(&self, _: &crate::model::Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        Ok(vec![])
    }
    fn relationships_to(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        Ok(vec![])
    }
    fn delete_relationships_by_source(&self, _: &str) -> crate::Result<u64> {
        Ok(0)
    }
    fn upsert_insight(&self, _: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        Ok(vec![])
    }
    fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn upsert_convention(&self, _: &crate::model::Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> crate::Result<Vec<crate::model::Convention>> {
        Ok(vec![])
    }
    fn create_analysis_run(&self, _: &str, _: &str) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _: &crate::model::AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
        Ok(None)
    }
    fn upsert_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn persist_analysis_batch(
        &self,
        _: &[&Entity],
        _: &[crate::model::Relationship],
        _: &crate::model::AnalysisRun,
        _: &str,
        _: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        Ok(Default::default())
    }
    fn reconcile_entities(
        &self,
        _: &str,
        _: &str,
        _: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn clear_repo_summaries(&self, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn add_output_dir(&self, _: &str, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn get_output_dirs(&self, _: &str) -> crate::Result<Vec<String>> {
        Ok(vec![])
    }
    fn get_all_file_hashes(
        &self,
        _repo_id: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }
    fn delete_file_hash(&self, _repo_id: &str, _file_path: &str) -> crate::Result<()> {
        Ok(())
    }
    fn count_missing_summaries(&self, _repo_id: &str) -> crate::Result<u64> {
        Ok(0)
    }
}

#[test]
fn test_search_entities_with_tier_function_filter() {
    let entities = vec![
        Entity {
            id: "fn-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: "my_function".to_string(),
            path: Some("src/my_function.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "file-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "my_file".to_string(),
            path: Some("src/my_file.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
    ];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(TierFilteredMockStorage::with_entities(entities))),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Search with tier="function" - should only return function entities
    let input = json!({
        "query": "my",
        "semantic": false,
        "tier": "function"
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1, "Should only return 1 result");
    assert_eq!(results[0]["tier"], "function");
    assert_eq!(results[0]["name"], "my_function");
}

#[test]
fn test_search_entities_with_tier_file_filter() {
    let entities = vec![
        Entity {
            id: "fn-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: "my_function".to_string(),
            path: Some("src/my_function.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "file-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "my_file".to_string(),
            path: Some("src/my_file.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
    ];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(TierFilteredMockStorage::with_entities(entities))),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Search with tier="file" - should only return file entities
    let input = json!({
        "query": "my",
        "semantic": false,
        "tier": "file"
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1, "Should only return 1 result");
    assert_eq!(results[0]["tier"], "file");
    assert_eq!(results[0]["name"], "my_file");
}

#[test]
fn test_search_entities_without_tier_returns_all() {
    let entities = vec![
        Entity {
            id: "fn-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: "my_function".to_string(),
            path: Some("src/my_function.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "file-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "my_file".to_string(),
            path: Some("src/my_file.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
    ];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(TierFilteredMockStorage::with_entities(entities))),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Search without tier - should return all matching entities
    let input = json!({
        "query": "my",
        "semantic": false
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 2, "Should return 2 results");
}

#[test]
fn test_search_entities_with_invalid_tier_returns_error() {
    let entities = vec![];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(TierFilteredMockStorage::with_entities(entities))),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Search with invalid tier - should return error
    let input = json!({
        "query": "test",
        "semantic": false,
        "tier": "invalid_tier"
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed.get("error").is_some(),
        "Should return error for invalid tier"
    );
    let error_msg = parsed["error"].as_str().unwrap();
    assert!(
        error_msg.contains("invalid tier"),
        "Error should mention invalid tier: {error_msg}"
    );
    assert!(
        error_msg.contains("invalid_tier"),
        "Error should include the invalid value: {error_msg}"
    );
}
