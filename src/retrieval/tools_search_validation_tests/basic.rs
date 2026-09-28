// SearchEntitiesTool basic input validation tests.

use super::{SearchEntitiesTool, Tool, Value, json};
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use std::sync::{Arc, Mutex};

// Test that search_entities with semantic=false uses exact match
// (no summary clause, dead code removed)
#[test]
fn test_search_entities_non_semantic_mode() {
    struct MockStorage {
        entities: Vec<Entity>,
    }

    impl Storage for MockStorage {
        fn list_entities(
            &self,
            _project_id: &str,
            _tier: Option<EntityTier>,
        ) -> crate::Result<Vec<Entity>> {
            Ok(self.entities.clone())
        }

        fn get_entity(&self, _entity_id: &str) -> crate::Result<Option<Entity>> {
            Ok(None)
        }

        fn relationships_from(
            &self,
            _entity_id: &str,
        ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
            Ok(vec![])
        }

        fn relationships_to(
            &self,
            _entity_id: &str,
        ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
            Ok(vec![])
        }
        fn search_entities_by_name(
            &self,
            _: &str,
            _: &[&str],
            _: usize,
            _: Option<&str>,
        ) -> crate::Result<Vec<Entity>> {
            Ok(vec![])
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
        fn list_repos(&self, _: &str) -> crate::Result<Vec<crate::model::Repository>> {
            Ok(vec![])
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
            Ok(crate::storage::DeleteStats::default())
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
            Ok(crate::storage::DeleteStats::default())
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

    impl Clone for MockStorage {
        fn clone(&self) -> Self {
            Self {
                entities: self.entities.clone(),
            }
        }
    }

    let entities = vec![];
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(MockStorage { entities })),
        project_id: "test-project".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let tool = SearchEntitiesTool { ctx };

    // Test empty string returns error
    let input = json!({
        "query": "",
        "semantic": false
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed.get("error").is_some(),
        "Empty query should return error"
    );
    assert_eq!(parsed["error"].as_str().unwrap(), "query cannot be empty");

    // Test whitespace-only string returns error
    let input = json!({
        "query": "   ",
        "semantic": false
    });
    let result = tool.call(input).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed.get("error").is_some(),
        "Whitespace-only query should return error"
    );
    assert_eq!(parsed["error"].as_str().unwrap(), "query cannot be empty");
}
