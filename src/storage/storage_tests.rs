// Test helpers for the Storage trait: the MockStorage adapter used by
// storage_tests and the object-safety check.
use super::*;

#[test]
fn test_storage_trait_is_object_safe() {
    // This test ensures the Storage trait can be used as a trait object
    // (useful for dependency injection and testing)
    let mock: Box<dyn Storage> = Box::new(MockStorage);
    fn accept_storage(_: &dyn Storage) {}
    accept_storage(&*mock);
}

// Mock implementation for testing
pub struct MockStorage;

impl Storage for MockStorage {
    fn create_project(&self, _name: &str, _description: Option<&str>) -> crate::Result<Project> {
        unimplemented!()
    }
    fn get_project(&self, _name: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _project_id: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<Project>> {
        unimplemented!()
    }
    fn add_repo(
        &self,
        _project_id: &str,
        _name: &str,
        _local_path: &str,
    ) -> crate::Result<Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _repo_id: &str) -> crate::Result<Option<Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _project_id: &str) -> crate::Result<Vec<Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _repo_id: &str, _path: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _repo_id: &str, _commit: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn record_unresolved_counts(
        &self,
        _repo_id: &str,
        _internal: u64,
        _external: u64,
    ) -> crate::Result<()> {
        Ok(())
    }
    fn update_repo_project(&self, _repo_id: &str, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _repo_id: &str) -> crate::Result<DeleteStats> {
        Ok(DeleteStats::default())
    }
    fn upsert_entity(&self, _entity: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_entity(&self, _entity_id: &str) -> crate::Result<Option<Entity>> {
        // Intentionally panics like every other MockStorage method: a data
        // path reaching get_entity here means the object-safety test is
        // being satisfied by a real data path, which it must not be.
        // Blast-radius tests use their own SqliteBox adapter
        // (tools_explore_blast_tests) instead of this mock.
        unimplemented!(
            "MockStorage::get_entity — data paths must not resolve entities through this mock"
        )
    }
    fn list_entities(
        &self,
        _project_id: &str,
        _tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn search_entities_by_name(
        &self,
        _project_id: &str,
        _words: &[&str],
        _limit: usize,
        _tier: Option<&str>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_repo(
        &self,
        _repo_id: &str,
        _tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_parent(&self, _parent_id: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entity_by_path(&self, _repo_id: &str, _path: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entity_ids_for_paths(
        &self,
        _repo_id: &str,
        _paths: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }

    fn entity_by_path_projectwide(
        &self,
        _project_id: &str,
        _path: &str,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }

    fn delete_entities_by_repo(&self, _repo_id: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(
        &self,
        _repo_id: &str,
        _exclude_paths: &[String],
    ) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _rel: &Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(&self, _source_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        unimplemented!()
    }
    fn relationships_to(&self, _target_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _source_id: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _insight: &Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _project_id: &str,
        _category: Option<&str>,
        _severity: Option<&str>,
        _limit: usize,
    ) -> crate::Result<Vec<Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _convention: &Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _project_id: &str,
        _category: Option<&str>,
    ) -> crate::Result<Vec<Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(
        &self,
        _repo_id: &str,
        _commit_hash: &str,
    ) -> crate::Result<AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _run: &AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _repo_id: &str, _file_path: &str) -> crate::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(
        &self,
        _repo_id: &str,
        _file_path: &str,
        _content_hash: &str,
    ) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_all_file_hashes(
        &self,
        _repo_id: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn delete_file_hash(&self, _repo_id: &str, _file_path: &str) -> crate::Result<()> {
        unimplemented!()
    }
    // get_unresolved_counts uses the trait default (None) — no impl needed.
    fn persist_analysis_batch(
        &self,
        _entities: &[&Entity],
        _relationships: &[Relationship],
        _run: &AnalysisRun,
        _repo_id: &str,
        _last_commit: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn delete_project(&self, _project_id: &str) -> crate::Result<DeleteStats> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _project_id: &str,
        _repo_id: &str,
        _repo_path: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _repo_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn add_output_dir(&self, _project_id: &str, _dir: &str) -> crate::Result<()> {
        Ok(())
    }
    fn get_output_dirs(&self, _project_id: &str) -> crate::Result<Vec<String>> {
        Ok(vec![])
    }
    fn count_missing_summaries(&self, _repo_id: &str) -> crate::Result<u64> {
        Ok(0)
    }
}
