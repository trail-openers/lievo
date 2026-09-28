// GetImpactTool / GetHotspotsTool tests — split from tools_query.rs.

use super::*;
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::{GetImpactTool, ToolContext};
use crate::storage::sqlite::SqliteStorage;
use std::sync::{Arc, Mutex};

/// Seed a file entity stored with the ACTUAL storage format — an absolute path
/// under the repo root, exactly as `refresh` persists it (issue #644 regression).
fn seed_file_entity(
    storage: &SqliteStorage,
    project_id: &str,
    repo_id: &str,
    name: &str,
    path: &str,
) -> Entity {
    let entity = Entity {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        name: name.to_string(),
        path: Some(path.to_string()),
        parent_id: None,
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: Some(serde_json::json!({"complexity_max": 1.0}).to_string()),
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();
    entity
}

fn make_impact_tool(repo_local_path: &str) -> (GetImpactTool<SqliteStorage>, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo", repo_local_path)
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    (GetImpactTool { ctx }, project.id, repo.id)
}

fn make_test_tool() -> (GetHotspotsTool<SqliteStorage>, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let project_id = project.id.clone();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    (GetHotspotsTool { ctx }, project_id)
}

fn create_test_entity(project_id: &str, tier: EntityTier, complexity: f64) -> Entity {
    Entity {
        id: uuid::Uuid::new_v4().to_string(),
        project_id: project_id.to_string(),
        repo_id: None,
        tier,
        name: format!("test_{}", tier.to_string().to_lowercase()),
        path: Some(format!("/path/{}.rs", tier.to_string().to_lowercase())),
        parent_id: None,
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: Some(serde_json::json!({"complexity_max": complexity}).to_string()),
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    }
}

#[test]
fn test_get_hotspots_default_returns_only_file_tier() {
    let (tool, project_id) = make_test_tool();
    let storage = Arc::clone(&tool.ctx.storage);

    let file_entity = create_test_entity(&project_id, EntityTier::File, 10.0);
    storage.lock().unwrap().upsert_entity(&file_entity).unwrap();
    let module_entity = create_test_entity(&project_id, EntityTier::Module, 20.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&module_entity)
        .unwrap();
    let subsystem_entity = create_test_entity(&project_id, EntityTier::Subsystem, 30.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&subsystem_entity)
        .unwrap();

    let result = tool.call(json!({})).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    assert!(!lines.is_empty());

    // All returned entities should be file tier
    for line in lines {
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(json["tier"], "file");
    }
}

#[test]
fn test_get_hotspots_tier_module_returns_only_module_tier() {
    let (tool, project_id) = make_test_tool();
    let storage = Arc::clone(&tool.ctx.storage);

    let file_entity = create_test_entity(&project_id, EntityTier::File, 10.0);
    storage.lock().unwrap().upsert_entity(&file_entity).unwrap();
    let module_entity = create_test_entity(&project_id, EntityTier::Module, 20.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&module_entity)
        .unwrap();
    let subsystem_entity = create_test_entity(&project_id, EntityTier::Subsystem, 30.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&subsystem_entity)
        .unwrap();

    let result = tool.call(json!({"tier": "module"})).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 1);

    for line in lines {
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(json["tier"], "module");
    }
}

#[test]
fn test_get_hotspots_tier_subsystem_returns_only_subsystem_tier() {
    let (tool, project_id) = make_test_tool();
    let storage = Arc::clone(&tool.ctx.storage);

    let file_entity = create_test_entity(&project_id, EntityTier::File, 10.0);
    storage.lock().unwrap().upsert_entity(&file_entity).unwrap();
    let module_entity = create_test_entity(&project_id, EntityTier::Module, 20.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&module_entity)
        .unwrap();
    let subsystem_entity = create_test_entity(&project_id, EntityTier::Subsystem, 30.0);
    storage
        .lock()
        .unwrap()
        .upsert_entity(&subsystem_entity)
        .unwrap();

    let result = tool.call(json!({"tier": "subsystem"})).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 1);

    for line in lines {
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(json["tier"], "subsystem");
    }
}

#[test]
fn test_get_hotspots_no_duplicate_entities() {
    let (tool, project_id) = make_test_tool();
    let storage = Arc::clone(&tool.ctx.storage);

    // Create multiple file-tier entities
    for i in 0..5 {
        let entity = create_test_entity(&project_id, EntityTier::File, i as f64);
        storage.lock().unwrap().upsert_entity(&entity).unwrap();
    }

    let result = tool.call(json!({})).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 5);

    // Check for duplicates by entity ID
    let mut ids = std::collections::HashSet::new();
    for line in lines {
        let json: serde_json::Value = serde_json::from_str(line).unwrap();
        let id = json["id"].as_str().unwrap();
        assert!(!ids.contains(id), "Duplicate entity ID found: {}", id);
        ids.insert(id.to_string());
    }
}

#[test]
fn test_get_hotspots_invalid_tier_returns_error() {
    let (tool, _) = make_test_tool();

    let result = tool.call(json!({"tier": "invalid"}));
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid tier"));
    assert!(err.contains("invalid"));
}

#[test]
fn test_get_hotspots_valid_tier_values_accepted() {
    let (tool, _) = make_test_tool();

    for valid_tier in &["file", "module", "subsystem"] {
        let result = tool.call(json!({"tier": valid_tier}));
        assert!(result.is_ok(), "tier '{}' should be accepted", valid_tier);
    }
}

#[test]
fn test_get_hotspots_limit_zero_returns_error() {
    let (tool, _) = make_test_tool();

    let result = tool.call(json!({"limit": 0}));
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("limit must be greater than 0"),
        "expected limit=0 error, got: {err}"
    );
}

#[test]
fn test_get_hotspots_subsystem_equal_complexity_sorted_by_name() {
    let (tool, project_id) = make_test_tool();
    let storage = Arc::clone(&tool.ctx.storage);

    // Create two subsystem entities with same complexity (0.0 default)
    // but names in reverse alphabetical order
    let mut entity_b = create_test_entity(&project_id, EntityTier::Subsystem, 0.0);
    entity_b.name = "subsystem_b".to_string();
    let mut entity_a = create_test_entity(&project_id, EntityTier::Subsystem, 0.0);
    entity_a.name = "subsystem_a".to_string();

    storage.lock().unwrap().upsert_entity(&entity_b).unwrap();
    storage.lock().unwrap().upsert_entity(&entity_a).unwrap();

    let result = tool.call(json!({"tier": "subsystem"})).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 2);

    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();

    // With equal complexity, sort by name alphabetically
    assert_eq!(first["name"], "subsystem_a");
    assert_eq!(second["name"], "subsystem_b");
}

#[test]
fn test_get_impact_repo_relative_path_resolves_against_absolute_stored_path() {
    // Seed a file entity with a stored path in the REAL storage format: an
    // absolute path under the repo root (issue #644 regression).
    let repo_root = "/tmp/example-repo";
    let stored_path = format!("{}/src/storage/sqlite.rs", repo_root);
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "sqlite.rs",
        &stored_path,
    );

    // Repo-relative input must resolve to the stored entity.
    let result = tool
        .call(json!({"files": ["src/storage/sqlite.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let changed = v["files"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "expected one changed file, got: {result}");
    // The lean contract echoes repo-relative paths, not entity ids.
    assert_eq!(changed[0], "src/storage/sqlite.rs");
}

#[test]
fn test_get_impact_dotslash_relative_path_resolves_against_absolute_stored_path() {
    let repo_root = "/tmp/example-repo";
    let stored_path = format!("{}/src/storage/sqlite.rs", repo_root);
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "sqlite.rs",
        &stored_path,
    );

    // The "./"-prefixed form must resolve to the same stored entity.
    let result = tool
        .call(json!({"files": ["./src/storage/sqlite.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let changed = v["files"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "expected one changed file, got: {result}");
    assert_eq!(changed[0], "src/storage/sqlite.rs");
}

#[test]
fn test_get_impact_absolute_path_still_resolves() {
    let repo_root = "/tmp/example-repo";
    let stored_path = format!("{}/src/storage/sqlite.rs", repo_root);
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "sqlite.rs",
        &stored_path,
    );

    // Absolute input matches the stored form directly (exact match path);
    // the response still echoes the repo-relative form.
    let result = tool.call(json!({"files": [stored_path]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let changed = v["files"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "expected one changed file, got: {result}");
    assert_eq!(changed[0], "src/storage/sqlite.rs");
}

#[test]
fn test_get_impact_unknown_path_reports_entity_not_found() {
    let repo_root = "/tmp/example-repo";
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "other.rs",
        &format!("{}/src/other.rs", repo_root),
    );

    let err = tool
        .call(json!({"files": ["src/nope/missing.rs"]}))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("no entity found for paths"),
        "expected no-entity-found error, got: {err}"
    );
}

// --- #690: get_impact additive unresolved_imports / resolution_coverage fields ---

/// Record the repo-wide unresolved-import counts on the repository row's
/// columns (#856), so `get_unresolved_counts` returns them.
pub(crate) fn seed_unresolved_counts(
    storage: &SqliteStorage,
    _project_id: &str,
    repo_id: &str,
    _repo_name: &str,
    internal: u64,
    external: u64,
) {
    storage
        .record_unresolved_counts(repo_id, internal, external)
        .unwrap();
}

#[test]
fn test_get_impact_empty_files_early_return_carries_null_resolution_fields() {
    let (tool, _, _) = make_impact_tool("/tmp/example-repo");
    let result = tool.call(json!({"files": []})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    // Lean shape (issue #840): exactly the four top-level keys, nothing else.
    assert_eq!(
        v.as_object().unwrap().len(),
        4,
        "lean shape must have exactly 4 keys: {result}"
    );
    assert_eq!(v["files"], serde_json::json!([]));
    assert_eq!(v["dependents"], serde_json::json!([]));
    // Null-not-absent guarantee (issue #840 gap-gate #7).
    assert!(v["unresolved_imports"].is_null());
    assert!(v["resolution_coverage"].is_null());
}

#[test]
fn test_get_impact_pre681_index_null_never_zero_or_full() {
    let repo_root = "/tmp/example-repo";
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "main.rs",
        &format!("{}/src/main.rs", repo_root),
    );
    // No unresolved-count marker seeded: pre-#681 shape.
    let result = tool.call(json!({"files": ["src/main.rs"]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(
        v["unresolved_imports"].is_null(),
        "pre-#681: unresolved_imports must be null, not 0"
    );
    assert!(
        v["resolution_coverage"].is_null(),
        "pre-#681: resolution_coverage must be null, not 1.0"
    );
    let changed = v["files"].as_array().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0], "src/main.rs");
}

#[test]
fn test_get_impact_zero_unresolved_reports_full_coverage() {
    let repo_root = "/tmp/example-repo";
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "main.rs",
        &format!("{}/src/main.rs", repo_root),
    );
    seed_unresolved_counts(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "repo",
        0,
        5,
    );
    let result = tool.call(json!({"files": ["src/main.rs"]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["unresolved_imports"], 0);
    assert_eq!(v["resolution_coverage"], 1.0);
}

#[test]
fn test_get_impact_unresolved_internal_reports_count_and_zero_coverage() {
    let repo_root = "/tmp/example-repo";
    let (tool, project_id, repo_id) = make_impact_tool(repo_root);
    let storage = Arc::clone(&tool.ctx.storage);
    let _ = seed_file_entity(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "main.rs",
        &format!("{}/src/main.rs", repo_root),
    );
    seed_unresolved_counts(
        &storage.lock().unwrap(),
        &project_id,
        &repo_id,
        "repo",
        3,
        7,
    );
    let result = tool.call(json!({"files": ["src/main.rs"]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["unresolved_imports"], 3);
    assert_eq!(v["resolution_coverage"], 0.0);
    // Lean keys remain intact alongside the honesty signals (issue #840).
    for key in ["files", "dependents"] {
        assert!(v.get(key).is_some(), "missing key {key}");
    }
}

// --- #764: cross-project boundary regression for get_impact ---

#[test]
fn get_impact_does_not_leak_across_project_boundaries() {
    // Two projects share one database. Project A has a file with a module
    // parent; project B has a file that depends on that module (cross-
    // project edge). The get_impact call for project A's file must not
    // surface project B's file in downstream_dependents or affected_callers.
    use crate::model::{EdgeProvenance, RelType, Relationship};
    use crate::storage::Storage;

    let storage = SqliteStorage::open_in_memory().unwrap();
    let pa = storage.create_project("impact-a", None).unwrap();
    let pb = storage.create_project("impact-b", None).unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

    // Project A: module + file.
    let module_a = Entity {
        id: "pa:mod".into(),
        project_id: pa.id.clone(),
        repo_id: Some(repo_a.id.clone()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "mod_a".into(),
        path: Some("mod_a".into()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    let file_a = Entity {
        id: "pa:file:main.rs".into(),
        project_id: pa.id.clone(),
        repo_id: Some(repo_a.id.clone()),
        tier: EntityTier::File,
        parent_id: Some("pa:mod".into()),
        name: "main.rs".into(),
        path: Some(format!("{}/src/main.rs", "/tmp/repoA")),
        language: Some("rust".into()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    // Project B: a file that has a cross-project DependsOn edge to module_a.
    let file_b = Entity {
        id: "pb:file:foreign.rs".into(),
        project_id: pb.id.clone(),
        repo_id: Some(repo_b.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "foreign.rs".into(),
        path: Some("/tmp/repoB/src/foreign.rs".into()),
        language: Some("rust".into()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    for e in [&module_a, &file_a, &file_b] {
        storage.upsert_entity(e).unwrap();
    }
    // Cross-project edge: file_b (projB) -> module_a (projA).
    storage
        .upsert_relationship(&Relationship {
            source_id: "pb:file:foreign.rs".into(),
            target_id: "pa:mod".into(),
            rel_type: RelType::DependsOn,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: pa.id.clone(),
        repo_path: std::path::PathBuf::from("/tmp/repoA"),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    let result = tool.call(json!({"files": ["src/main.rs"]})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();

    // Project B's file must not appear in the lean dependents array.
    let downstream = v["dependents"].as_array().unwrap();
    let leaked = downstream
        .iter()
        .any(|e| e["path"] == "/tmp/repoB/src/foreign.rs");
    assert!(
        !leaked,
        "cross-project entity must not appear in dependents: {v:?}"
    );
}
