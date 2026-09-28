// God module insight detector.
//
// Detects modules with an unusually large number of files ("god modules").
// Extracted from insights_arch.rs to keep files under 500 lines.

use crate::model::{EntityTier, Insight};
use crate::storage::Storage;

use super::insights::{insight_id, now};

/// Minimum file count for a module to be considered for god-module detection.
const GOD_MODULE_MIN_FILES: usize = 10;

/// Ratio above average at which a module is flagged as a god module.
const GOD_MODULE_HIGH_RATIO: f64 = 3.0;
const GOD_MODULE_MEDIUM_RATIO: f64 = 2.0;

/// Detect modules with an unusually large number of files ("god modules").
pub fn detect_god_modules(storage: &dyn Storage, project_id: &str) -> crate::Result<Vec<Insight>> {
    let modules = storage.list_entities(project_id, Some(EntityTier::Module))?;
    if modules.is_empty() {
        return Ok(vec![]);
    }

    // Count files per module.
    let mut counts: Vec<(&str, &str, usize)> = Vec::new(); // (id, name, count)
    for module in &modules {
        let files = storage.entities_by_parent(&module.id)?;
        let file_count = files.iter().filter(|e| e.tier == EntityTier::File).count();
        counts.push((module.id.as_str(), module.name.as_str(), file_count));
    }

    let total: usize = counts.iter().map(|(_, _, c)| c).sum();
    if counts.is_empty() || total == 0 {
        return Ok(vec![]);
    }
    let avg = total as f64 / counts.len() as f64;

    let mut insights = Vec::new();
    for (id, name, count) in &counts {
        if *count < GOD_MODULE_MIN_FILES {
            continue;
        }
        let ratio = *count as f64 / avg;
        let severity = if ratio >= GOD_MODULE_HIGH_RATIO {
            "high"
        } else if ratio >= GOD_MODULE_MEDIUM_RATIO {
            "medium"
        } else {
            continue;
        };

        let insight = Insight {
            id: insight_id(project_id, "god_module", id),
            project_id: project_id.to_string(),
            category: "god_module".to_string(),
            severity: Some(severity.to_string()),
            title: format!("God module: {name}"),
            description: Some(format!(
                "Module '{name}' has {count} files ({ratio:.1}x average of {avg:.0})"
            )),
            entity_ids_json: Some(format!("[\"{id}\"]")),
            detected_at: now(),
            still_valid: true,
        };
        insights.push(insight);
    }

    Ok(insights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entity, EntityTier, Relationship};
    use crate::storage::Storage;
    use crate::{model::Project, model::Repository};
    use std::cell::RefCell;

    // ---- Minimal stub storage for tests ----

    #[derive(Default)]
    struct StubStorage {
        entities: RefCell<Vec<Entity>>,
    }

    impl StubStorage {
        fn add_entity(&self, e: Entity) {
            self.entities.borrow_mut().push(e);
        }
    }

    fn stub_entity(id: &str, name: &str, tier: EntityTier, parent: Option<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier,
            parent_id: parent.map(|s| s.to_string()),
            name: name.to_string(),
            path: None,
            language: None,
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
        fn relationships_from(
            &self,
            _source_id: &str,
        ) -> crate::Result<Vec<(Relationship, Entity)>> {
            unimplemented!()
        }
        fn relationships_to(&self, _target_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
            unimplemented!()
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
        fn create_analysis_run(
            &self,
            _r: &str,
            _c: &str,
        ) -> crate::Result<crate::model::AnalysisRun> {
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

    // ---- God module tests ----

    fn add_module_with_files(s: &StubStorage, mod_id: &str, name: &str, file_count: usize) {
        s.add_entity(stub_entity(mod_id, name, EntityTier::Module, None));
        for i in 0..file_count {
            let fid = format!("{mod_id}_file_{i}");
            s.add_entity(stub_entity(&fid, &fid, EntityTier::File, Some(mod_id)));
        }
    }

    #[test]
    fn test_detect_god_modules_high_severity() {
        // avg = (50 + 5 + 5 + 5 + 5) / 5 = 14. 50 / 14 ≈ 3.57x → high
        let s = StubStorage::default();
        add_module_with_files(&s, "controllers", "controllers", 50);
        add_module_with_files(&s, "m2", "m2", 5);
        add_module_with_files(&s, "m3", "m3", 5);
        add_module_with_files(&s, "m4", "m4", 5);
        add_module_with_files(&s, "m5", "m5", 5);

        let insights = detect_god_modules(&s, "proj").unwrap();
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].severity.as_deref(), Some("high"));
        assert!(insights[0].title.contains("controllers"));
        assert!(
            insights[0]
                .description
                .as_deref()
                .unwrap()
                .contains("50 files")
        );
    }

    #[test]
    fn test_detect_god_modules_medium_severity() {
        // avg = (30 + 5 + 5 + 5 + 5) / 5 = 10. 30 / 10 = 3.0x → high (exactly at boundary)
        // Use 25 files: 25 / 10 = 2.5x → medium
        let s = StubStorage::default();
        add_module_with_files(&s, "m1", "m1", 25);
        add_module_with_files(&s, "m2", "m2", 5);
        add_module_with_files(&s, "m3", "m3", 5);
        add_module_with_files(&s, "m4", "m4", 5);
        add_module_with_files(&s, "m5", "m5", 5);

        let insights = detect_god_modules(&s, "proj").unwrap();
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].severity.as_deref(), Some("medium"));
    }

    #[test]
    fn test_detect_god_modules_below_min_threshold() {
        // Even if ratio is high, modules with < 10 files are excluded.
        let s = StubStorage::default();
        add_module_with_files(&s, "m1", "m1", 9); // < 10 files → excluded
        add_module_with_files(&s, "m2", "m2", 1);

        let insights = detect_god_modules(&s, "proj").unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn test_detect_god_modules_no_modules() {
        let s = StubStorage::default();
        let insights = detect_god_modules(&s, "proj").unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn test_detect_god_modules_insight_id_is_deterministic() {
        // avg = (60 + 1 + 1 + 1 + 1) / 5 = 12.8; 60 / 12.8 ≈ 4.7x → qualifies as high
        let s = StubStorage::default();
        add_module_with_files(&s, "big_mod", "big_mod", 60);
        add_module_with_files(&s, "small", "small", 1);
        add_module_with_files(&s, "small2", "small2", 1);
        add_module_with_files(&s, "small3", "small3", 1);
        add_module_with_files(&s, "small4", "small4", 1);

        let i1 = detect_god_modules(&s, "proj").unwrap();
        let i2 = detect_god_modules(&s, "proj").unwrap();
        assert_eq!(i1[0].id, i2[0].id);
        assert_eq!(i1[0].id, insight_id("proj", "god_module", "big_mod"));
    }
}
