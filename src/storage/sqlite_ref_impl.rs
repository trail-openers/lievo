/// Allow `QueryEngine<&SqliteStorage>` by delegating all Storage methods through
/// the reference. All `Storage` methods take `&self`, so this is purely
/// syntactic delegation.
use crate::Result;
use crate::model::*;
use crate::storage::Storage;

use super::sqlite::SqliteStorage;

impl Storage for &SqliteStorage {
    fn create_project(&self, name: &str, description: Option<&str>) -> Result<Project> {
        (*self).create_project(name, description)
    }
    fn get_project(&self, name: &str) -> Result<Option<Project>> {
        (*self).get_project(name)
    }
    fn get_project_by_id(&self, project_id: &str) -> Result<Option<Project>> {
        (*self).get_project_by_id(project_id)
    }
    fn list_projects(&self) -> Result<Vec<Project>> {
        (*self).list_projects()
    }
    fn add_repo(&self, project_id: &str, name: &str, local_path: &str) -> Result<Repository> {
        (*self).add_repo(project_id, name, local_path)
    }
    fn get_repo(&self, repo_id: &str) -> Result<Option<Repository>> {
        (*self).get_repo(repo_id)
    }
    fn list_repos(&self, project_id: &str) -> Result<Vec<Repository>> {
        (*self).list_repos(project_id)
    }
    fn update_repo_index_path(&self, repo_id: &str, path: &str) -> Result<()> {
        (*self).update_repo_index_path(repo_id, path)
    }
    fn update_repo_last_commit(&self, repo_id: &str, commit: &str) -> Result<()> {
        (*self).update_repo_last_commit(repo_id, commit)
    }
    fn record_unresolved_counts(&self, repo_id: &str, internal: u64, external: u64) -> Result<()> {
        (*self).record_unresolved_counts(repo_id, internal, external)
    }
    fn update_repository_unconfigured_marker(
        &self,
        repo_id: &str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        (*self).update_repository_unconfigured_marker(repo_id, fingerprint)
    }
    fn update_repo_project(&self, repo_id: &str, project_id: &str) -> Result<()> {
        (*self).update_repo_project(repo_id, project_id)
    }
    fn set_repo_git_url(&self, repo_id: &str, key: &str) -> Result<()> {
        (*self).set_repo_git_url(repo_id, key)
    }
    fn find_repos_by_git_url(&self, key: &str) -> Result<Vec<Repository>> {
        (*self).find_repos_by_git_url(key)
    }
    fn update_repo_local_path(
        &self,
        repo_id: &str,
        new_path: &str,
        new_index_path: Option<&str>,
    ) -> Result<()> {
        (*self).update_repo_local_path(repo_id, new_path, new_index_path)
    }
    fn delete_repo(&self, repo_id: &str) -> Result<crate::storage::DeleteStats> {
        (*self).delete_repo(repo_id)
    }
    fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        (*self).upsert_entity(entity)
    }
    fn clear_entity_summary(&self, entity_id: &str) -> Result<()> {
        (*self).clear_entity_summary(entity_id)
    }
    fn get_entity(&self, entity_id: &str) -> Result<Option<Entity>> {
        (*self).get_entity(entity_id)
    }
    fn list_entities(&self, project_id: &str, tier: Option<EntityTier>) -> Result<Vec<Entity>> {
        (*self).list_entities(project_id, tier)
    }
    fn search_entities_by_name(
        &self,
        project_id: &str,
        words: &[&str],
        limit: usize,
        tier: Option<&str>,
    ) -> Result<Vec<Entity>> {
        (*self).search_entities_by_name(project_id, words, limit, tier)
    }
    fn entities_by_repo(&self, repo_id: &str, tier: Option<EntityTier>) -> Result<Vec<Entity>> {
        (*self).entities_by_repo(repo_id, tier)
    }

    fn get_unresolved_counts(&self, repo_id: &str) -> Option<(u64, u64)> {
        (*self).get_unresolved_counts(repo_id)
    }
    fn entities_by_parent(&self, parent_id: &str) -> Result<Vec<Entity>> {
        (*self).entities_by_parent(parent_id)
    }
    fn entity_by_path(&self, repo_id: &str, path: &str) -> Result<Option<Entity>> {
        (*self).entity_by_path(repo_id, path)
    }
    fn entity_ids_for_paths(
        &self,
        repo_id: &str,
        paths: &[&str],
    ) -> Result<std::collections::HashMap<String, String>> {
        (*self).entity_ids_for_paths(repo_id, paths)
    }
    fn entity_by_path_projectwide(&self, project_id: &str, path: &str) -> Result<Vec<Entity>> {
        (*self).entity_by_path_projectwide(project_id, path)
    }
    fn delete_entities_by_repo(&self, repo_id: &str) -> Result<u64> {
        (*self).delete_entities_by_repo(repo_id)
    }
    fn delete_entities_by_paths(&self, repo_id: &str, exclude_paths: &[String]) -> Result<u64> {
        (*self).delete_entities_by_paths(repo_id, exclude_paths)
    }
    fn upsert_relationship(&self, rel: &Relationship) -> Result<()> {
        (*self).upsert_relationship(rel)
    }
    fn relationships_from(&self, source_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        (*self).relationships_from(source_id)
    }
    fn list_all_relationships(&self, project_id: &str) -> Result<Vec<Relationship>> {
        (*self).list_all_relationships(project_id)
    }

    fn relationships_to(&self, target_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        (*self).relationships_to(target_id)
    }
    fn delete_relationships_by_source(&self, source_id: &str) -> Result<u64> {
        (*self).delete_relationships_by_source(source_id)
    }
    fn upsert_insight(&self, insight: &Insight) -> Result<()> {
        (*self).upsert_insight(insight)
    }
    fn list_insights(
        &self,
        project_id: &str,
        category: Option<&str>,
        severity: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Insight>> {
        (*self).list_insights(project_id, category, severity, limit)
    }
    fn invalidate_insights(&self, project_id: &str) -> Result<()> {
        (*self).invalidate_insights(project_id)
    }
    fn upsert_convention(&self, convention: &Convention) -> Result<()> {
        (*self).upsert_convention(convention)
    }
    fn list_conventions(
        &self,
        project_id: &str,
        category: Option<&str>,
    ) -> Result<Vec<Convention>> {
        (*self).list_conventions(project_id, category)
    }
    fn create_analysis_run(&self, repo_id: &str, commit_hash: &str) -> Result<AnalysisRun> {
        (*self).create_analysis_run(repo_id, commit_hash)
    }
    fn update_analysis_run(&self, run: &AnalysisRun) -> Result<()> {
        (*self).update_analysis_run(run)
    }
    fn get_file_hash(&self, repo_id: &str, file_path: &str) -> Result<Option<String>> {
        (*self).get_file_hash(repo_id, file_path)
    }
    fn upsert_file_hash(&self, repo_id: &str, file_path: &str, content_hash: &str) -> Result<()> {
        (*self).upsert_file_hash(repo_id, file_path, content_hash)
    }
    fn get_all_file_hashes(
        &self,
        repo_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        (*self).get_all_file_hashes(repo_id)
    }
    fn delete_file_hash(&self, repo_id: &str, file_path: &str) -> Result<()> {
        (*self).delete_file_hash(repo_id, file_path)
    }
    fn persist_analysis_batch(
        &self,
        entities: &[&Entity],
        relationships: &[Relationship],
        run: &AnalysisRun,
        repo_id: &str,
        last_commit: &str,
    ) -> Result<(i64, i64)> {
        (*self).persist_analysis_batch(entities, relationships, run, repo_id, last_commit)
    }
    fn delete_project(&self, project_id: &str) -> Result<crate::storage::DeleteStats> {
        (*self).delete_project(project_id)
    }
    fn reconcile_entities(
        &self,
        project_id: &str,
        repo_id: &str,
        repo_path: &std::path::Path,
    ) -> Result<crate::storage::reconcile::ReconcileStats> {
        (*self).reconcile_entities(project_id, repo_id, repo_path)
    }
    fn clear_all_summaries(&self, project_id: &str) -> Result<()> {
        (*self).clear_all_summaries(project_id)
    }
    fn clear_repo_summaries(&self, repo_id: &str) -> Result<()> {
        (*self).clear_repo_summaries(repo_id)
    }
    fn add_output_dir(&self, project_id: &str, dir: &str) -> Result<()> {
        (*self).add_output_dir(project_id, dir)
    }
    fn get_output_dirs(&self, project_id: &str) -> Result<Vec<String>> {
        (*self).get_output_dirs(project_id)
    }
    fn count_missing_summaries(&self, repo_id: &str) -> Result<u64> {
        (*self).count_missing_summaries(repo_id)
    }
    fn count_missing_summaries_tier(&self, repo_id: &str, tier: &str) -> Result<u64> {
        (*self).count_missing_summaries_tier(repo_id, tier)
    }
}
