// Issue #723 (include_depth knob) / #731 (default flipped to false):
// coverage for the `include_depth` boolean knob on lievo_explore.
//
// Wired via `#[path]` from tools_explore.rs. Covers the acceptance criteria
// for the `include_depth` boolean knob:
//   - include_depth=false (and the new bare-default call, which deserializes
//     to false, issue #731) omits call_paths/blast_radius entirely (not
//     null, not empty arrays) while every other tier-1 key stays present.
//   - The serialized response with include_depth=false is measurably smaller
//     than the same call with an explicit include_depth=true (the opt-in).
//   - include_depth=false and include_source=true are orthogonal: tier-2
//     source is still returned while the depth keys are omitted.
//
// The byte-identity default pin lives in
// `same_query_same_index_produces_byte_identical_json` in
// tools_explore_tier_tests.rs (two bare-default calls — both now lean under
// the false default, issue #731).

use serde_json::{Value, json};

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

// Reuse the shared tier-test fixtures via super (sibling module under the
// same `tools_explore` parent).
use super::tools_explore_tier_tests::{file_entity, make_tool, setup, write_file};

#[test]
fn include_depth_false_omits_depth_keys_and_is_cheaper() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // File with a Function child that has a Calls edge, so the depth payload
    // is non-empty when requested (reuses the pattern of the base-suite
    // call_paths test).
    write_file(tmp.path(), "depth.rs", "fn caller() { callee() }\n");
    let make_fn = |id: &str, name: &str| Entity {
        id: id.to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: EntityTier::Function,
        parent_id: Some("p:repo1:file:depth.rs".to_string()),
        name: name.to_string(),
        path: Some("depth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let ent = file_entity(
        "p:repo1:file:depth.rs",
        &project_id,
        &repo_id,
        "depth",
        "depth.rs",
        None,
    );
    storage.upsert_entity(&ent).unwrap();
    storage
        .upsert_entity(&make_fn("fn-depth-caller", "caller"))
        .unwrap();
    storage
        .upsert_entity(&make_fn("fn-depth-callee", "callee"))
        .unwrap();
    let rel = Relationship {
        source_id: "fn-depth-caller".to_string(),
        target_id: "fn-depth-callee".to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage.upsert_relationship(&rel).unwrap();
    // Issue #759: add an incoming Imports edge so blast_radius is non-empty.
    // The file `importer.rs` imports `depth.rs`, making it a dependent.
    let importer = file_entity(
        "p:repo1:file:importer.rs",
        &project_id,
        &repo_id,
        "importer",
        "importer.rs",
        None,
    );
    storage.upsert_entity(&importer).unwrap();
    storage
        .upsert_relationship(&Relationship {
            source_id: "p:repo1:file:importer.rs".to_string(),
            target_id: "p:repo1:file:depth.rs".to_string(),
            rel_type: RelType::Imports,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    // Issue #731: the default is now lean — the full arm needs an explicit
    // include_depth=true to exercise the depth payload.
    let full = tool
        .call(json!({"query": "depth", "include_depth": true}))
        .unwrap();
    let minimal = tool
        .call(json!({"query": "depth", "include_depth": false}))
        .unwrap();

    // Depth keys absent entirely (not null, not empty arrays).
    let parsed: Value = serde_json::from_str(&minimal).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("call_paths").is_none(),
        "include_depth=false must omit call_paths: {sym}"
    );
    assert!(
        sym.get("blast_radius").is_none(),
        "include_depth=false must omit blast_radius: {sym}"
    );
    // All other tier-1 fields remain (entity_id intentionally absent from
    // the per-symbol payload — issue #834).
    for key in [
        "name",
        "kind",
        "qualified_path",
        "signature",
        "score",
        "reason",
    ] {
        assert!(sym.get(key).is_some(), "missing '{key}': {sym}");
    }
    assert!(sym.get("entity_id").is_none());

    // Explicit include_depth=true carries the depth payload; non-empty here.
    let full_parsed: Value = serde_json::from_str(&full).unwrap();
    let full_sym = &full_parsed["symbols"].as_array().unwrap()[0];
    assert!(!full_sym["call_paths"].as_array().unwrap().is_empty());
    assert!(!full_sym["blast_radius"].as_array().unwrap().is_empty());

    // Measurably smaller serialized response (issue #723 AC #1/#4).
    assert!(
        minimal.len() < full.len(),
        "minimal ({}) must be shorter than full ({})",
        minimal.len(),
        full.len()
    );
}

#[test]
fn include_depth_false_and_source_true_are_orthogonal() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "ortho.rs", "fn tiny() { 1 }\n");
    let ent = file_entity(
        "p:repo1:file:ortho.rs",
        &project_id,
        &repo_id,
        "ortho",
        "ortho.rs",
        None,
    );
    storage.upsert_entity(&ent).unwrap();

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    let result = tool
        .call(json!({"query": "ortho", "include_source": true, "include_depth": false}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // Tier 2 source present, depth keys absent — the knobs are independent.
    assert!(sym.get("source").is_some(), "source must be present: {sym}");
    assert!(sym.get("call_paths").is_none());
    assert!(sym.get("blast_radius").is_none());
}
