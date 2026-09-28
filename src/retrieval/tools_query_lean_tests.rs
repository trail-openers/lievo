// #840: lean get_impact response contract — the shared lean-contract
// fixture, the shape/hop/honesty-signal tests, and the before/after
// response-size measurement. Split from tools_query_tests.rs.

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::GetImpactTool;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use std::sync::{Arc, Mutex};

#[test]
fn test_get_impact_lean_response_shape_files_dependents_path_hop() {
    let (storage, project_id, repo_id) = lean_contract_storage();
    let _ = repo_id;
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    let result = tool
        .call(serde_json::json!({"files": ["src/core.rs", "src/utils.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();

    // Exactly the four top-level keys — the lean core, nothing else.
    assert_eq!(
        v.as_object().unwrap().len(),
        4,
        "lean shape must have exactly 4 keys: {result}"
    );
    // files echoes the requested changed files as repo-relative paths.
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|f| f.as_str().is_some()));

    // dependents: one entry per owning file, each {path, hop} — repo-relative
    // path plus hop 0 (direct) or 1 (second-hop).
    let dependents = v["dependents"].as_array().unwrap();
    let by_path: std::collections::HashMap<&str, i64> = dependents
        .iter()
        .map(|d| (d["path"].as_str().unwrap(), d["hop"].as_i64().unwrap()))
        .collect();
    // src/service.rs imports src/core.rs → hop 0.
    assert_eq!(by_path.get("src/service.rs"), Some(&0), "{result}");
    // src/lib.rs imports src/utils.rs → hop 0.
    assert_eq!(by_path.get("src/lib.rs"), Some(&0), "{result}");
    // fn main (in src/cli.rs) calls src/core.rs → normalizes to hop 0.
    assert_eq!(by_path.get("src/cli.rs"), Some(&0), "{result}");
    // The function-tier caller must NOT appear as its own entry.
    assert!(!by_path.contains_key("/repo/src/cli.rs"));
    // Every dependent entry is exactly {path, hop}.
    for d in dependents {
        assert_eq!(
            d.as_object().unwrap().len(),
            2,
            "dependent entry must be path+hop only: {d:?}"
        );
        let p = d["path"].as_str().unwrap();
        assert!(!p.starts_with("/"), "path must be repo-relative: {p}");
    }
    // The Contains edge to orphan.rs must not be traversed.
    assert!(
        !by_path.contains_key("src/orphan.rs"),
        "non-dependency edge leaked: {result}"
    );
}

/// hop 1 reachability: changed file ← A (imports, hop 0) ← B (imports A, hop 1).
#[test]
fn test_get_impact_hop1_reached_second_hop_only_kept() {
    let (storage, project_id, repo_id) = lean_contract_storage();
    let _ = repo_id;
    // Add a pure hop-1 chain: lib imports utils → lib is hop 0, cli imports
    // lib and is not reached directly → hop 1.
    upsert_rel(
        &storage,
        "f:lib.rs",
        "f:utils.rs",
        crate::model::RelType::Imports,
    );
    upsert_rel(
        &storage,
        "f:cli.rs",
        "f:lib.rs",
        crate::model::RelType::Imports,
    );
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    let result = tool
        .call(serde_json::json!({"files": ["src/utils.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let by_path: std::collections::HashMap<&str, i64> = v["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["path"].as_str().unwrap(), d["hop"].as_i64().unwrap()))
        .collect();
    // lib imports utils → hop 0.
    assert_eq!(by_path.get("src/lib.rs"), Some(&0), "utils←lib: {v:?}");
    // cli imports lib (a hop-0 dependent) and is not reached directly → hop 1.
    assert_eq!(by_path.get("src/cli.rs"), Some(&1), "utils←lib←cli: {v:?}");
}

#[test]
fn test_get_impact_honesty_signals_recorded_values_surface() {
    let (storage, project_id, repo_id) = lean_contract_storage();
    seed_unresolved_counts(&storage, &project_id, &repo_id, "repo", 3, 7);
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    // Empty-dependents case: a file with NO dependents but recorded unresolved
    // counts — distinguishable from genuine no-dependents (null signals).
    let result = tool
        .call(serde_json::json!({"files": ["src/orphan.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["dependents"], serde_json::json!([]));
    assert_eq!(
        v["unresolved_imports"], 3,
        "recorded counts must surface even when dependents is empty"
    );
    assert_eq!(v["resolution_coverage"], 0.0);
}

/// Issue #840 acceptance criterion: measurable response-size reduction.
///
/// Builds the SAME fixture as the lean contract tests (≥2 changed files,
/// dependents at BOTH hop 0 and hop 1, a function-tier caller normalizing
/// into its file, recorded unresolved counts), serializes BOTH the lean
/// shape and a faithful reconstruction of the OLD shape (the per-entry
/// id/name/tier payload that the slimming removed — the old response carried
/// changed files, affected modules/subsystems, downstream dependents and
/// affected callers as id/name/tier arrays, per the pre-#840 contract in
/// this same test suite's history), and asserts the lean form is strictly
/// smaller. Both character counts are printed via `println!` so the numbers
/// appear in `cargo test -- --nocapture` output as the baseline for a
/// future A/B comparison.
#[test]
fn test_get_impact_lean_response_is_smaller_than_old_shape() {
    let (storage, project_id, repo_id) = lean_contract_storage();
    seed_unresolved_counts(&storage, &project_id, &repo_id, "repo", 3, 7);
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    let result = tool
        .call(serde_json::json!({"files": ["src/core.rs", "src/utils.rs"]}))
        .unwrap();
    let lean: serde_json::Value = serde_json::from_str(&result).unwrap();

    // Reconstruct the old shape from the SAME fixture data: the five arrays
    // each entry of which carried {id, name, tier}. The dependents entries
    // are exactly the lean dependents' paths (one per owning file), and the
    // changed files are the two requested files.
    let entity_entry = |id: &str| serde_json::json!({"id": id, "name": id.rsplit('/').next().unwrap_or(id), "tier": "file"});
    let lean_dependents = lean["dependents"].as_array().unwrap();
    let changed_ids: Vec<String> = lean["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| format!("/repo/{}", f.as_str().unwrap()))
        .collect();
    let dependent_ids: Vec<String> = lean_dependents
        .iter()
        .map(|d| format!("/repo/{}", d["path"].as_str().unwrap()))
        .collect();
    let old = serde_json::json!({
        "changed_files": changed_ids.iter().map(|id| entity_entry(id)).collect::<Vec<_>>(),
        "affected_modules": [],
        "affected_subsystems": [],
        "downstream_dependents": dependent_ids.iter().map(|id| entity_entry(id)).collect::<Vec<_>>(),
        "affected_callers": dependent_ids.iter().map(|id| entity_entry(id)).collect::<Vec<_>>()
    });
    let old_str = old.to_string();
    let lean_len = result.len();
    let old_len = old_str.len();
    println!(
        "#840 size measurement (same fixture): old shape = {old_len} chars, lean shape = {lean_len} chars, reduction = {} chars ({}%)",
        old_len - lean_len,
        ((old_len - lean_len) as f64 / old_len as f64 * 100.0),
    );
    assert!(
        lean_len < old_len,
        "lean response ({lean_len} chars) must be smaller than the old shape ({old_len} chars)"
    );
}

#[test]
fn test_get_impact_honesty_signals_unknown_are_null_not_absent() {
    let (storage, project_id, repo_id) = lean_contract_storage();
    let _ = repo_id;
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = GetImpactTool { ctx };

    // No unresolved counts recorded → null (present), not absent, not fabricated.
    let result = tool
        .call(serde_json::json!({"files": ["src/orphan.rs"]}))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("unresolved_imports").is_some() && v["unresolved_imports"].is_null());
    assert!(v.get("resolution_coverage").is_some() && v["resolution_coverage"].is_null());
}

// ---------------------------------------------------------------------------
// Shared lean-contract fixture (issue #840 gap-gate #6): ≥2 changed files,
// dependents at BOTH hop 0 and hop 1, a function-tier caller normalizing
// into its file, and recorded unresolved counts (non-null honesty signals).
// ---------------------------------------------------------------------------

fn file_entity_with_id(
    project_id: &str,
    repo_id: &str,
    id: &str,
    name: &str,
    path: &str,
    parent_id: Option<String>,
) -> Entity {
    Entity {
        id: id.into(),
        project_id: project_id.into(),
        repo_id: Some(repo_id.into()),
        tier: EntityTier::File,
        parent_id,
        name: name.into(),
        path: Some(path.into()),
        language: Some("rust".into()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    }
}

fn upsert_rel(
    storage: &SqliteStorage,
    source: &str,
    target: &str,
    rel_type: crate::model::RelType,
) {
    storage
        .upsert_relationship(&crate::model::Relationship {
            source_id: source.into(),
            target_id: target.into(),
            rel_type,
            weight: 1.0,
            evidence_json: None,
            provenance: crate::model::EdgeProvenance::Heuristic,
        })
        .unwrap();
}

/// Record the repo-wide unresolved-import counts on the repository row's
/// columns (#856).
fn seed_unresolved_counts(
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

fn lean_contract_storage() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("lean-project", None).unwrap();
    let repo = storage.add_repo(&project.id, "repo", "/repo").unwrap();
    for id in [
        "f:core.rs",
        "f:utils.rs",
        "f:service.rs",
        "f:lib.rs",
        "f:cli.rs",
        "fn:cli.main",
        "f:orphan.rs",
    ] {
        let (name, path) = match id {
            "f:core.rs" => ("core.rs", "/repo/src/core.rs"),
            "f:utils.rs" => ("utils.rs", "/repo/src/utils.rs"),
            "f:service.rs" => ("service.rs", "/repo/src/service.rs"),
            "f:lib.rs" => ("lib.rs", "/repo/src/lib.rs"),
            "f:cli.rs" => ("cli.rs", "/repo/src/cli.rs"),
            "f:orphan.rs" => ("orphan.rs", "/repo/src/orphan.rs"),
            _ => (id, "/repo/x.rs"),
        };
        let parent = if id.starts_with("f:") {
            None
        } else {
            Some("f:cli.rs".into())
        };
        storage
            .upsert_entity(&file_entity_with_id(
                &project.id,
                &repo.id,
                id,
                name,
                path,
                parent,
            ))
            .unwrap();
    }
    // Function-tier caller of core.rs — must normalize into src/cli.rs.
    let mut fn_entity = file_entity_with_id(
        &project.id,
        &repo.id,
        "fn:cli.main",
        "main",
        "/repo/src/cli.rs",
        Some("f:cli.rs".into()),
    );
    fn_entity.tier = EntityTier::Function;
    storage.upsert_entity(&fn_entity).unwrap();

    // hop 0: service imports core; lib imports utils; fn main calls core.
    upsert_rel(
        &storage,
        "f:service.rs",
        "f:core.rs",
        crate::model::RelType::Imports,
    );
    upsert_rel(
        &storage,
        "f:lib.rs",
        "f:utils.rs",
        crate::model::RelType::Imports,
    );
    upsert_rel(
        &storage,
        "fn:cli.main",
        "f:core.rs",
        crate::model::RelType::Calls,
    );
    // hop 1: cli imports lib (lib is a hop-0 dependent of utils).
    upsert_rel(
        &storage,
        "f:cli.rs",
        "f:lib.rs",
        crate::model::RelType::Imports,
    );
    // Non-dependency edge: Contains must NOT be traversed (issue #840 gap-gate #3).
    upsert_rel(
        &storage,
        "f:orphan.rs",
        "f:core.rs",
        crate::model::RelType::Contains,
    );

    (storage, project.id, repo.id)
}
