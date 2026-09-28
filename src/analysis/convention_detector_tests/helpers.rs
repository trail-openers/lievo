use crate::model::Convention;
use crate::model::Entity;
use crate::model::EntityTier;
use crate::storage::Storage;
use crate::storage::reconcile::ReconcileStats;
use std::collections::HashMap;
use std::path::Path;

/// MockStorage stub: only `list_entities` and `upsert_convention` are exercised.
/// All other Storage methods panic via `unimplemented!()` to surface accidental coupling.
pub struct MockStorage {
    pub entities: Vec<Entity>,
}

impl MockStorage {
    pub fn new(entities: Vec<Entity>) -> Self {
        Self { entities }
    }
}

impl Storage for MockStorage {
    fn create_project(
        &self,
        _name: &str,
        _description: Option<&str>,
    ) -> crate::Result<crate::model::Project> {
        unimplemented!()
    }
    fn get_project(&self, _name: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _project_id: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
        unimplemented!()
    }
    fn delete_project(&self, _project_id: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn add_output_dir(&self, _project_id: &str, _dir: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_output_dirs(&self, _project_id: &str) -> crate::Result<Vec<String>> {
        unimplemented!()
    }
    fn add_repo(
        &self,
        _project_id: &str,
        _name: &str,
        _local_path: &str,
    ) -> crate::Result<crate::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _repo_id: &str) -> crate::Result<Option<crate::model::Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _project_id: &str) -> crate::Result<Vec<crate::model::Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _repo_id: &str, _path: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _repo_id: &str, _commit: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _repo_id: &str, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _repo_id: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn upsert_entity(&self, _entity: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_entity(&self, _entity_id: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn list_entities(
        &self,
        _project_id: &str,
        _tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>> {
        Ok(self.entities.clone())
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
    ) -> crate::Result<HashMap<String, String>> {
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
    fn upsert_relationship(&self, _rel: &crate::model::Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(
        &self,
        _source_id: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn relationships_to(
        &self,
        _target_id: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _source_id: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _insight: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _project_id: &str,
        _category: Option<&str>,
        _severity: Option<&str>,
        _limit: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _convention: &Convention) -> crate::Result<()> {
        Ok(())
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
    ) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _run: &crate::model::AnalysisRun) -> crate::Result<()> {
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
    fn get_all_file_hashes(&self, _repo_id: &str) -> crate::Result<HashMap<String, String>> {
        unimplemented!()
    }
    fn delete_file_hash(&self, _repo_id: &str, _file_path: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _entities: &[&Entity],
        _relationships: &[crate::model::Relationship],
        _run: &crate::model::AnalysisRun,
        _repo_id: &str,
        _last_commit: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _project_id: &str,
        _repo_id: &str,
        _repo_path: &Path,
    ) -> crate::Result<ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _project_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _repo_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn count_missing_summaries(&self, _repo_id: &str) -> crate::Result<u64> {
        Ok(0)
    }
}

pub fn make_entity(
    id: &str,
    project_id: &str,
    tier: EntityTier,
    name: &str,
    path: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: None,
        tier,
        parent_id: None,
        name: name.to_string(),
        path: path.map(|p| p.to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}
