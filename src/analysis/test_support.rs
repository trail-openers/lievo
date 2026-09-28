//! Shared test infrastructure for analysis module unit tests.
//!
//! Provides a minimal `StubStorage` implementation of the `Storage` trait
//! so insight detector tests can run without a real database.

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::storage::Storage;
use crate::{model::Project, model::Repository};
use std::cell::RefCell;

/// Minimal stub `Storage` implementation for unit tests.
#[derive(Default)]
pub(crate) struct StubStorage {
    pub(crate) entities: RefCell<Vec<Entity>>,
    pub(crate) relationships: RefCell<Vec<Relationship>>,
}

impl StubStorage {
    pub(crate) fn add_entity(&self, e: Entity) {
        self.entities.borrow_mut().push(e);
    }

    pub(crate) fn add_rel(&self, src: &str, tgt: &str, rel_type: RelType) {
        self.relationships.borrow_mut().push(Relationship {
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            rel_type,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        });
    }

    fn entity_by_id(&self, id: &str) -> Option<Entity> {
        self.entities.borrow().iter().find(|e| e.id == id).cloned()
    }
}

pub(crate) fn stub_entity(
    id: &str,
    name: &str,
    tier: EntityTier,
    parent: Option<&str>,
    language: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier,
        parent_id: parent.map(|s| s.to_string()),
        name: name.to_string(),
        path: None,
        language: language.map(|s| s.to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

impl Storage for StubStorage {
    fn create_project(&self, _n: &str, _d: Option<&str>) -> crate::Result<Project> {
        unimplemented!()
    }
    fn get_project(&self, _n: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _id: &str) -> crate::Result<Option<Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<Project>> {
        unimplemented!()
    }
    fn delete_project(&self, _id: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn add_repo(&self, _p: &str, _n: &str, _l: &str) -> crate::Result<Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _id: &str) -> crate::Result<Option<Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _p: &str) -> crate::Result<Vec<Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _r: &str, _p: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _r: &str, _c: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _r: &str, _p: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _r: &str) -> crate::Result<crate::storage::DeleteStats> {
        Ok(crate::storage::DeleteStats::default())
    }
    fn upsert_entity(&self, _e: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_entity(&self, _id: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn list_entities(
        &self,
        project_id: &str,
        tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>> {
        Ok(self
            .entities
            .borrow()
            .iter()
            .filter(|e| e.project_id == project_id)
            .filter(|e| tier.is_none() || Some(e.tier) == tier)
            .cloned()
            .collect())
    }
    fn entities_by_repo(&self, _r: &str, _t: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_parent(&self, parent_id: &str) -> crate::Result<Vec<Entity>> {
        Ok(self
            .entities
            .borrow()
            .iter()
            .filter(|e| e.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect())
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
    fn entity_by_path(&self, _r: &str, _p: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entity_ids_for_paths(
        &self,
        _: &str,
        _: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _r: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(&self, _r: &str, _p: &[String]) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _r: &Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(&self, source_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .relationships
            .borrow()
            .iter()
            .filter(|r| r.source_id == source_id)
            .filter_map(|r| self.entity_by_id(&r.target_id).map(|e| (r.clone(), e)))
            .collect())
    }
    fn relationships_to(&self, target_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .relationships
            .borrow()
            .iter()
            .filter(|r| r.target_id == target_id)
            .filter_map(|r| self.entity_by_id(&r.source_id).map(|e| (r.clone(), e)))
            .collect())
    }
    fn delete_relationships_by_source(&self, _s: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _i: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _p: &str,
        _c: Option<&str>,
        _s: Option<&str>,
        _l: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _p: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _c: &crate::model::Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _p: &str,
        _c: Option<&str>,
    ) -> crate::Result<Vec<crate::model::Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(&self, _r: &str, _c: &str) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _r: &crate::model::AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _r: &str, _f: &str) -> crate::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(&self, _r: &str, _f: &str, _h: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _e: &[&Entity],
        _r: &[Relationship],
        _run: &crate::model::AnalysisRun,
        _repo: &str,
        _c: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _p: &str,
        _r: &str,
        _rp: &std::path::Path,
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
