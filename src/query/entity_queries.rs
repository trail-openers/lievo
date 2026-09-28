// QueryEngine — delegates entity queries to the Storage trait
use crate::model::{Entity, EntityTier, RelType};
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;

/// Return all Subsystem-tier entities for the given project.
pub fn subsystems(storage: &dyn Storage, project: &str) -> crate::Result<Vec<Entity>> {
    storage.list_entities(project, Some(EntityTier::Subsystem))
}

/// Return all direct children of the given subsystem entity.
pub fn modules_in(storage: &dyn Storage, subsystem_id: &str) -> crate::Result<Vec<Entity>> {
    storage.entities_by_parent(subsystem_id)
}

/// Return all direct children of the given module entity.
pub fn files_in(storage: &dyn Storage, module_id: &str) -> crate::Result<Vec<Entity>> {
    storage.entities_by_parent(module_id)
}

/// Look up an entity by its repository and file path.
pub fn entity_by_path(
    storage: &dyn Storage,
    repo_id: &str,
    path: &str,
) -> crate::Result<Option<Entity>> {
    storage.entity_by_path(repo_id, path)
}

/// Look up a single entity by its ID.
pub fn entity(storage: &dyn Storage, entity_id: &str) -> crate::Result<Option<Entity>> {
    storage.get_entity(entity_id)
}

/// Return the top-`limit` entities for the project sorted by complexity descending.
///
/// Complexity is extracted from `metrics_json` field as `{"complexity_max": <number>}`.
/// Entities with no metrics or unparseable metrics are treated as complexity 0.
pub fn hotspots(storage: &dyn Storage, project: &str, limit: usize) -> crate::Result<Vec<Entity>> {
    let mut entities = storage.list_entities(project, None)?;
    entities.sort_by(|a, b| {
        let ca = complexity_of(a);
        let cb = complexity_of(b);
        cb.partial_cmp(&ca)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    entities.truncate(limit);
    Ok(entities)
}

/// Return all Function entities contained directly within a file.
pub fn functions_in_file(storage: &dyn Storage, file_id: &str) -> crate::Result<Vec<Entity>> {
    storage.entities_by_parent(file_id)
}

/// Return all Function entities that are called by a given function.
///
/// Looks up Calls relationships from the given function and returns the target
/// Function entities that are called.
pub fn calls_of_function(storage: &dyn Storage, function_id: &str) -> crate::Result<Vec<Entity>> {
    // Project boundary (issue #764): the caller's project is read up front so
    // a cross-project `Calls` edge cannot surface a foreign callee.
    let project_id: Option<String> = storage.get_entity(function_id)?.map(|e| e.project_id);
    let is_same_project = |dep_project: &str| match project_id.as_deref() {
        None => true,
        Some(p) => same_project(p, dep_project),
    };
    let relationships = storage.relationships_from(function_id)?;
    let mut callees = Vec::new();
    for (rel, target) in relationships {
        if rel.rel_type == RelType::Calls
            && is_same_project(&target.project_id)
            && let Some(entity) = storage.get_entity(&rel.target_id)?
            && entity.tier == EntityTier::Function
        {
            callees.push(entity);
        }
    }
    Ok(callees)
}

/// Extract the numeric `complexity_max` field from an entity's `metrics_json`.
/// Returns 0.0 if the field is absent or cannot be parsed.
pub(crate) fn complexity_of(entity: &Entity) -> f64 {
    let json = match entity.metrics_json.as_deref() {
        Some(s) => s,
        None => return 0.0,
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("complexity_max").and_then(|c| c.as_f64()))
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::sqlite::SqliteStorage;

    /// Build a minimal test fixture: one project, one repo, entities at all tiers.
    ///
    /// Hierarchy:
    ///   project "test-proj"
    ///   └── repo  "test-repo"  (id = repo.id)
    ///       └── subsystem "subsys-1"  (tier = Subsystem)
    ///           └── module "module-1" (tier = Module)
    ///               ├── file "file-1.rs" (tier = File, complexity_max = 10, path = src/file-1.rs)
    ///               └── file "file-2.rs" (tier = File, complexity_max = 5,  path = src/file-2.rs)
    ///       └── subsystem "subsys-2"  (tier = Subsystem)  [no children]
    fn build_fixture() -> (SqliteStorage, String, String, String, String, String) {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "/tmp/test")
            .unwrap();

        let now = "2024-01-01T00:00:00Z".to_string();

        let subsys1 = Entity {
            id: "subsys-1".to_string(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Subsystem,
            parent_id: None,
            name: "subsys-1".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        storage.upsert_entity(&subsys1).unwrap();

        let subsys2 = Entity {
            id: "subsys-2".to_string(),
            name: "subsys-2".to_string(),
            ..subsys1.clone()
        };
        storage.upsert_entity(&subsys2).unwrap();

        let module1 = Entity {
            id: "module-1".to_string(),
            tier: EntityTier::Module,
            parent_id: Some("subsys-1".to_string()),
            name: "module-1".to_string(),
            path: Some("src/module-1".to_string()),
            ..subsys1.clone()
        };
        storage.upsert_entity(&module1).unwrap();

        let file1 = Entity {
            id: "file-1".to_string(),
            tier: EntityTier::File,
            parent_id: Some("module-1".to_string()),
            name: "file-1.rs".to_string(),
            path: Some("src/file-1.rs".to_string()),
            metrics_json: Some(r#"{"complexity_max": 10}"#.to_string()),
            ..subsys1.clone()
        };
        storage.upsert_entity(&file1).unwrap();

        let file2 = Entity {
            id: "file-2".to_string(),
            tier: EntityTier::File,
            parent_id: Some("module-1".to_string()),
            name: "file-2.rs".to_string(),
            path: Some("src/file-2.rs".to_string()),
            metrics_json: Some(r#"{"complexity_max": 5}"#.to_string()),
            ..subsys1.clone()
        };
        storage.upsert_entity(&file2).unwrap();

        (
            storage,
            project.id,
            repo.id,
            "subsys-1".to_string(),
            "module-1".to_string(),
            "file-1".to_string(),
        )
    }

    #[test]
    fn test_subsystems_returns_subsystem_tier_entities() {
        let (storage, project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = subsystems(&storage, &project_id).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|e| e.tier == EntityTier::Subsystem));
        let names: Vec<&str> = result.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"subsys-1"));
        assert!(names.contains(&"subsys-2"));
    }

    #[test]
    fn test_subsystems_empty_project_returns_empty() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("empty-proj", None).unwrap();
        let result = subsystems(&storage, &project.id).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_modules_in_returns_children_of_subsystem() {
        let (storage, _project_id, _repo_id, subsys_id, _module_id, _file_id) = build_fixture();
        let result = modules_in(&storage, &subsys_id).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "module-1");
        assert_eq!(result[0].tier, EntityTier::Module);
    }

    #[test]
    fn test_modules_in_subsystem_with_no_children_returns_empty() {
        let (storage, _project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = modules_in(&storage, "subsys-2").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_files_in_returns_children_of_module() {
        let (storage, _project_id, _repo_id, _subsys_id, module_id, _file_id) = build_fixture();
        let result = files_in(&storage, &module_id).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|e| e.tier == EntityTier::File));
        let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"file-1"));
        assert!(ids.contains(&"file-2"));
    }

    #[test]
    fn test_entity_by_path_returns_correct_entity() {
        let (storage, _project_id, repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = entity_by_path(&storage, &repo_id, "src/file-1.rs").unwrap();
        assert!(result.is_some());
        let entity = result.unwrap();
        assert_eq!(entity.id, "file-1");
    }

    #[test]
    fn test_entity_by_path_missing_returns_none() {
        let (storage, _project_id, repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = entity_by_path(&storage, &repo_id, "nonexistent/path.rs").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_entity_returns_correct_entity() {
        let (storage, _project_id, _repo_id, _subsys_id, _module_id, file_id) = build_fixture();
        let result = entity(&storage, &file_id).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, file_id);
    }

    #[test]
    fn test_entity_missing_returns_none() {
        let (storage, _project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = entity(&storage, "does-not-exist").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_hotspots_returns_sorted_by_complexity_descending() {
        let (storage, project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = hotspots(&storage, &project_id, 10).unwrap();
        // file-1 has complexity_max 10, file-2 has complexity_max 5; rest have 0
        assert!(!result.is_empty());
        assert_eq!(result[0].id, "file-1");
        assert_eq!(result[1].id, "file-2");
    }

    #[test]
    fn test_hotspots_limit_is_respected() {
        let (storage, project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = hotspots(&storage, &project_id, 1).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "file-1");
    }

    #[test]
    fn test_hotspots_zero_limit_returns_empty() {
        let (storage, project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        let result = hotspots(&storage, &project_id, 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_hotspots_limit_exceeds_total_entities_returns_all() {
        let (storage, project_id, _repo_id, _subsys_id, _module_id, _file_id) = build_fixture();
        // Fixture has 5 entities total; requesting 100 should return all 5
        let result = hotspots(&storage, &project_id, 100).unwrap();
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_hotspots_entities_without_metrics_treated_as_zero_complexity() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let now = "2024-01-01T00:00:00Z".to_string();
        let entity = Entity {
            id: "e1".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "orphan.rs".to_string(),
            path: Some("orphan.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now,
        };
        storage.upsert_entity(&entity).unwrap();
        let result = hotspots(&storage, &project.id, 10).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "e1");
    }

    #[test]
    fn test_complexity_of_invalid_json_treated_as_zero() {
        let entity = Entity {
            id: "e".to_string(),
            project_id: "p".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "e".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: Some("not-json".to_string()),
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
        };
        assert_eq!(complexity_of(&entity), 0.0);
    }

    #[test]
    fn test_complexity_of_missing_field_treated_as_zero() {
        let entity = Entity {
            id: "e".to_string(),
            project_id: "p".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "e".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: Some(r#"{"other": 99}"#.to_string()),
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
        };
        assert_eq!(complexity_of(&entity), 0.0);
    }

    #[test]
    fn test_complexity_of_wrong_type_treated_as_zero() {
        let entity = Entity {
            id: "e".to_string(),
            project_id: "p".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "e".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: Some(r#"{"complexity_max": "not_a_number"}"#.to_string()),
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
        };
        assert_eq!(complexity_of(&entity), 0.0);
    }

    #[test]
    fn calls_of_function_does_not_leak_across_project_boundaries() {
        // Issue #764: a cross-project Calls edge must not surface a foreign
        // callee. Two projects share one database; project B's function
        // calls project A's function.
        use crate::model::EdgeProvenance;
        use crate::model::Relationship;
        let storage = SqliteStorage::open_in_memory().unwrap();
        let pa = storage.create_project("cp-a", None).unwrap();
        let pb = storage.create_project("cp-b", None).unwrap();
        let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
        let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

        let caller_fn = Entity {
            id: "pb:fn:caller".into(),
            project_id: pb.id.clone(),
            repo_id: Some(repo_b.id.clone()),
            tier: EntityTier::Function,
            parent_id: None,
            name: "caller".into(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        let callee_fn = Entity {
            id: "pa:fn:callee".into(),
            project_id: pa.id.clone(),
            repo_id: Some(repo_a.id.clone()),
            tier: EntityTier::Function,
            parent_id: None,
            name: "callee".into(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        for e in [&caller_fn, &callee_fn] {
            storage.upsert_entity(e).unwrap();
        }
        // Cross-project edge: caller (projB) → callee (projA).
        storage
            .upsert_relationship(&Relationship {
                source_id: "pb:fn:caller".into(),
                target_id: "pa:fn:callee".into(),
                rel_type: RelType::Calls,
                weight: 1.0,
                evidence_json: None,
                provenance: EdgeProvenance::Heuristic,
            })
            .unwrap();

        // Caller is in project B, so the cross-project callee in project A
        // must not appear.
        let callees = calls_of_function(&storage, "pb:fn:caller").unwrap();
        let leaked = callees.iter().any(|e| e.id == "pa:fn:callee");
        assert!(
            !leaked,
            "cross-project callee must not appear in calls_of_function: {callees:?}"
        );
    }
}
