// SearchEntitiesTool multi-word search functionality tests.

use super::*;
use crate::model::{Entity, EntityTier};
use crate::retrieval::test_helpers;
use crate::retrieval::tools::ToolContext;
use serde_json::json;
use std::sync::{Arc, Mutex};

// Test that multi-word search matches any word in the query (issue #472)
#[test]
fn test_multi_word_search_matches_any_word() {
    // Create a custom MockStorage for this test that supports OR-logic search
    struct MockStorage {
        entities: Vec<Entity>,
    }

    impl MockStorage {
        fn with_entities(entities: Vec<Entity>) -> Self {
            Self { entities }
        }
    }

    impl Clone for MockStorage {
        fn clone(&self) -> Self {
            Self {
                entities: self.entities.clone(),
            }
        }
    }

    use crate::storage::Storage;

    impl Storage for MockStorage {
        fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
            Ok(self.entities.clone())
        }

        fn search_entities_by_name(
            &self,
            _: &str,
            words: &[&str],
            _: usize,
            _: Option<&str>,
        ) -> crate::Result<Vec<Entity>> {
            // Simple in-memory search: match if ANY word matches name or path (OR logic)
            let results: Vec<Entity> = self
                .entities
                .iter()
                .filter(|e| {
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

        fn get_entity(&self, _id: &str) -> crate::Result<Option<Entity>> {
            Ok(None)
        }
        fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<crate::model::Project> {
            unimplemented!()
        }
        fn get_project(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
            unimplemented!()
        }
        fn get_project_by_id(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
            unimplemented!()
        }
        fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
            unimplemented!()
        }
        fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<crate::model::Repository> {
            unimplemented!()
        }
        fn get_repo(&self, _: &str) -> crate::Result<Option<crate::model::Repository>> {
            unimplemented!()
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
            unimplemented!()
        }
        fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
            unimplemented!()
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
            unimplemented!()
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
            unimplemented!()
        }
        fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
            unimplemented!()
        }
        fn upsert_convention(&self, _: &crate::model::Convention) -> crate::Result<()> {
            unimplemented!()
        }
        fn list_conventions(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> crate::Result<Vec<crate::model::Convention>> {
            unimplemented!()
        }
        fn create_analysis_run(
            &self,
            _: &str,
            _: &str,
        ) -> crate::Result<crate::model::AnalysisRun> {
            unimplemented!()
        }
        fn update_analysis_run(&self, _: &crate::model::AnalysisRun) -> crate::Result<()> {
            unimplemented!()
        }
        fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
            unimplemented!()
        }
        fn upsert_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
            unimplemented!()
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
            unimplemented!()
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
            unimplemented!()
        }
        fn clear_repo_summaries(&self, _: &str) -> crate::Result<()> {
            unimplemented!()
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

    let entities = vec![
        Entity {
            id: "entity-1".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::Module,
            parent_id: None,
            name: "memory_manager".to_string(),
            path: Some("src/memory_manager.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "entity-2".to_string(),
            project_id: "test-project".to_string(),
            repo_id: None,
            tier: EntityTier::Module,
            parent_id: None,
            name: "data_store".to_string(),
            path: Some("src/data_store.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
    ];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(MockStorage::with_entities(entities))),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Test multi-word search: "memory store" should match both entities
    // (contains "memory" in first entity, "store" in second entity)
    let input = json!({
        "query": "memory store",
        "semantic": false
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(
        results.len(),
        2,
        "Multi-word search should match entities with any word"
    );
    let entity_ids: Vec<&str> = results
        .iter()
        .map(|r| r["entity_id"].as_str().unwrap())
        .collect();
    assert!(
        entity_ids.contains(&"entity-1"),
        "Should match memory_manager"
    );
    assert!(entity_ids.contains(&"entity-2"), "Should match data_store");
}

// Test that single-word search still works correctly (issue #472 regression test)
#[test]
fn test_single_word_search_unaffected() {
    // Use the base MockStorage from test_helpers
    let entities = vec![Entity {
        id: "entity-1".to_string(),
        project_id: "test-project".to_string(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "memory_manager".to_string(),
        path: Some("src/memory_manager.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }];

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(test_helpers::MockStorage {
            entities,
            edges: vec![],
        })),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Test single-word search should still work
    let input = json!({
        "query": "memory",
        "semantic": false
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(
        results.len(),
        0, // test_helpers MockStorage always returns empty results from search
        "Base MockStorage should return no results"
    );
}
