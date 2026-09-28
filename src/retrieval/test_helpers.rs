// Shared test helpers for the retrieval module.
// Provides a minimal MockStorage that satisfies the Storage trait by
// implementing only the methods exercised by retrieval tests.

use crate::model::{
    AnalysisRun, Convention, EdgeProvenance, Entity, EntityTier, Insight, Project, RelType,
    Relationship, Repository,
};
use crate::storage::Storage;

/// A minimal in-memory storage for tests.
/// Only `get_entity`, `list_entities`, `relationships_from`, and
/// `relationships_to` are implemented; all others panic with `unimplemented!`.
pub struct MockStorage {
    pub entities: Vec<Entity>,
    /// (source_id, target_id) pairs.
    pub edges: Vec<(String, String)>,
}

impl MockStorage {
    pub fn empty() -> Self {
        Self {
            entities: vec![],
            edges: vec![],
        }
    }
}

impl Storage for MockStorage {
    fn get_entity(&self, id: &str) -> crate::Result<Option<Entity>> {
        Ok(self.entities.iter().find(|e| e.id == id).cloned())
    }

    fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        Ok(self.entities.clone())
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

    fn relationships_from(&self, src: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .edges
            .iter()
            .filter(|(s, _)| s == src)
            .filter_map(|(s, t)| {
                self.entities.iter().find(|e| &e.id == t).map(|e| {
                    (
                        Relationship {
                            source_id: s.clone(),
                            target_id: t.clone(),
                            rel_type: RelType::DependsOn,
                            weight: 1.0,
                            evidence_json: None,
                            provenance: EdgeProvenance::Heuristic,
                        },
                        e.clone(),
                    )
                })
            })
            .collect())
    }

    fn relationships_to(&self, tgt: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .edges
            .iter()
            .filter(|(_, t)| t == tgt)
            .filter_map(|(s, _)| {
                self.entities.iter().find(|e| &e.id == s).map(|e| {
                    (
                        Relationship {
                            source_id: s.clone(),
                            target_id: tgt.to_string(),
                            rel_type: RelType::DependsOn,
                            weight: 1.0,
                            evidence_json: None,
                            provenance: EdgeProvenance::Heuristic,
                        },
                        e.clone(),
                    )
                })
            })
            .collect())
    }

    // Remaining methods — not needed by retrieval tests.
    fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<Project> {
        unimplemented!()
    }
    fn get_project(&self, _: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<Project>> {
        unimplemented!()
    }
    fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _: &str) -> crate::Result<Option<Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _: &str) -> crate::Result<Vec<Repository>> {
        Ok(vec![]) // No repos, so no index
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
    fn entity_by_path(&self, _: &str, _: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
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
    fn delete_entities_by_repo(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _: &Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _: &Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: usize,
    ) -> crate::Result<Vec<Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _: &Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(&self, _: &str, _: Option<&str>) -> crate::Result<Vec<Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(&self, _: &str, _: &str) -> crate::Result<AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _: &AnalysisRun) -> crate::Result<()> {
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
        _: &[Relationship],
        _: &AnalysisRun,
        _: &str,
        _: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn delete_repo(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
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
