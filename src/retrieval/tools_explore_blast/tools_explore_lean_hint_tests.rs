// Issue #767: unit tests for the lean (include_depth=false) completeness
// hint — the structured completeness field in `tools_explore_blast.rs` that
// states how many direct dependents were withheld and names the remedy
// (`include_depth=true`). Wired via `#[path]` from tools_explore_blast.rs.
//
// Covers the test-surface items assigned to this workstream (task-c):
//   - a lean response for a file with N dependents carries the field
//     with count N and names include_depth
//   - a lean response for a file with zero dependents carries no field
//   - the count excludes dependents in other projects (#764 boundary)
//   - a hub over the blast cap states the true count plus the ceiling
//   - the include_depth input description advertises the hint
//   - the added bytes per lean response are measured and bounded
//
// The include_depth=true byte-identity regression lives in the #759 suite
// (tools_explore_blast_tests.rs / tools_explore_tier_tests.rs), unchanged.

use std::sync::{Arc, Mutex};

use crate::model::RelType;
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use serde_json::{Value, json};

// Reuse the shared helpers from the sibling blast-radius test module.
use crate::retrieval::tools_explore_blast::tools_explore_blast_tests::{
    dep_edge, file_entity, setup,
};
use crate::retrieval::tools_explore_blast::{
    count_direct_dependents, direct_seeds, lean_dependents_hint,
};

fn make_tool(
    storage: SqliteStorage,
    project_id: String,
    repo_path: std::path::PathBuf,
) -> ExploreTool<SqliteStorage> {
    ExploreTool {
        ctx: Arc::new(crate::retrieval::tools::ToolContext {
            storage: Arc::new(Mutex::new(storage)),
            project_id,
            repo_path,
            output_dir: None,
            zero_repo_guidance: None,
        }),
    }
}

fn lean_call(tool: &ExploreTool<SqliteStorage>, query: &str) -> Value {
    let result = tool
        .call(json!({ "query": query, "include_depth": false }))
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[test]
fn lean_response_with_dependents_carries_completeness_field() {
    let (storage, project_id, repo_id) = setup();
    let target = file_entity(
        "p:repo1:file:target.rs",
        &project_id,
        &repo_id,
        "target",
        "target.rs",
        "Rust",
    );
    storage.upsert_entity(&target).unwrap();
    let importer = file_entity(
        "p:repo1:file:importer.rs",
        &project_id,
        &repo_id,
        "importer",
        "importer.rs",
        "Rust",
    );
    storage.upsert_entity(&importer).unwrap();
    dep_edge(&storage, &importer.id, &target.id, RelType::Imports);

    let tool = make_tool(storage, project_id, std::path::PathBuf::from("/tmp"));
    let result = lean_call(&tool, "target");
    let sym = &result["symbols"].as_array().unwrap()[0];
    let hint = sym["completeness"]
        .as_object()
        .expect("structured hint expected");
    assert_eq!(hint["complete"], false);
    assert_eq!(hint["omitted_direct_dependents"], 1);
    assert!(
        hint["remedy"]
            .as_str()
            .unwrap()
            .contains("include_depth=true"),
        "remedy must name the parameter: {hint:?}"
    );
    // Under the cap → no retrieval_ceiling field.
    assert!(hint.get("retrieval_ceiling").is_none());
}

#[test]
fn lean_response_with_zero_dependents_has_no_completeness_field() {
    let (storage, project_id, repo_id) = setup();
    let isolated = file_entity(
        "p:repo1:file:isolated.rs",
        &project_id,
        &repo_id,
        "isolated",
        "isolated.rs",
        "Rust",
    );
    storage.upsert_entity(&isolated).unwrap();

    let tool = make_tool(storage, project_id, std::path::PathBuf::from("/tmp"));
    let result = lean_call(&tool, "isolated");
    let sym = &result["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("completeness").is_none(),
        "zero dependents must not carry a hint: {sym}"
    );
}

#[test]
fn hinted_count_excludes_cross_project_dependents() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let pa = storage
        .create_project(&format!("cnt-a-{unique}"), None)
        .unwrap();
    let pb = storage
        .create_project(&format!("cnt-b-{unique}"), None)
        .unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

    let target = file_entity(
        "pa:repoA:file:target.rs",
        &pa.id,
        &repo_a.id,
        "target",
        "target.rs",
        "Rust",
    );
    let same_proj = file_entity(
        "pa:repoA:file:importer.rs",
        &pa.id,
        &repo_a.id,
        "importer",
        "importer.rs",
        "Rust",
    );
    let cross_proj = file_entity(
        "pb:repoB:file:foreign.rs",
        &pb.id,
        &repo_b.id,
        "foreign",
        "foreign.rs",
        "Rust",
    );
    for e in [&target, &same_proj, &cross_proj] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(
        &storage,
        "pa:repoA:file:importer.rs",
        "pa:repoA:file:target.rs",
        RelType::Imports,
    );
    dep_edge(
        &storage,
        "pb:repoB:file:foreign.rs",
        "pa:repoA:file:target.rs",
        RelType::Imports,
    );

    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let count = count_direct_dependents(&boxed, &target, &direct_seeds(&boxed, &target));
    assert_eq!(
        count,
        Some(1),
        "cross-project dependent must not be counted (got {count:?})"
    );
}

#[test]
fn hub_over_cap_hint_states_true_count_and_ceiling() {
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    storage.upsert_entity(&hub).unwrap();
    for i in 0..150 {
        let id = format!("p:repo1:file:imp_{i:03}.rs");
        let ent = file_entity(
            &id,
            &project_id,
            &repo_id,
            &format!("imp_{i:03}"),
            &format!("imp_{i:03}.rs"),
            "Rust",
        );
        storage.upsert_entity(&ent).unwrap();
        dep_edge(&storage, &id, &hub.id, RelType::Imports);
    }

    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let hint = lean_dependents_hint(&boxed, &hub).expect("hint expected");
    assert_eq!(hint.omitted_direct_dependents, 150);
    assert_eq!(
        hint.retrieval_ceiling,
        Some(crate::retrieval::tools_explore_blast::MAX_BLAST_ENTRIES),
        "the ceiling must track MAX_BLAST_ENTRIES (issue #854 lowered it to 25)"
    );
    // The hint is a constant-size structured object, not a growing string.
    let hint_json = serde_json::to_string(&hint).unwrap();
    assert!(hint_json.len() < 200);
}

/// The include_depth input description advertises the lean completeness hint
/// (how many direct dependents are withheld and that include_depth=true
/// retrieves them), so an agent reading the schema knows the hint exists
/// before calling with include_depth=false.
#[test]
fn include_depth_description_advertises_lean_completeness_hint() {
    let (storage, project_id, _repo_id) = setup();
    let tool = make_tool(storage, project_id, std::path::PathBuf::from("/tmp"));
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    let description = props["include_depth"]["description"]
        .as_str()
        .expect("include_depth description expected");
    assert!(
        description.contains("include_depth=true to retrieve them"),
        "description must name the remedy verbatim: {description}"
    );
    assert!(
        description.contains("direct dependents"),
        "description must name what the hint counts: {description}"
    );
    // Schema pin intact: still exactly the eight known properties — the hint
    // is an output change only, no new input parameter.
    assert_eq!(props.len(), 8, "unexpected schema change: {props:?}");
}

/// The added bytes per lean response: a file with dependents carries the
/// structured hint; a leaf file carries no hint at all (zero dependents →
/// no field). The diff is the per-call byte cost of the hint, measured
/// against the 24K hard cap. The count itself is a hop-0 read (one
/// `relationships_to` query per seed — file + its Function children), not
/// the tier-2 2-hop traversal, so the lean path keeps #731's token win.
#[test]
fn lean_response_byte_cost_of_hint_is_small_and_bounded() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let target = file_entity(
        "p:repo1:file:cost.rs",
        &project_id,
        &repo_id,
        "cost",
        "cost.rs",
        "Rust",
    );
    let iso = file_entity(
        "p:repo1:file:iso.rs",
        &project_id,
        &repo_id,
        "iso",
        "iso.rs",
        "Rust",
    );
    let importer = file_entity(
        "p:repo1:file:imp.rs",
        &project_id,
        &repo_id,
        "imp",
        "imp.rs",
        "Rust",
    );
    for e in [&target, &iso, &importer] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, &importer.id, &target.id, RelType::Imports);

    // Same small body inlined for both symbols (no source bodies in lean
    // mode) so the hint is the only delta between the two symbol entries.
    for path in ["cost.rs", "iso.rs", "imp.rs"] {
        std::fs::write(tmp.path().join(path), "fn z() {}\n").unwrap();
    }

    // One lean tool instance, two lean calls on the same index (no
    // dependents for iso; one for cost) — the hint is the only per-symbol
    // delta.
    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
    let with_hint = tool
        .call(json!({ "query": "cost", "include_depth": false }))
        .unwrap();
    let without_hint = tool
        .call(json!({ "query": "iso", "include_depth": false }))
        .unwrap();
    let with_parsed: Value = serde_json::from_str(&with_hint).unwrap();
    let with_sym = with_parsed["symbols"].as_array().unwrap()[0].clone();
    let without_parsed: Value = serde_json::from_str(&without_hint).unwrap();
    let without_sym = without_parsed["symbols"].as_array().unwrap()[0].clone();
    // The hint is present exactly on the dependent-bearing symbol.
    assert!(
        with_sym.get("completeness").is_some(),
        "dependent-bearing symbol must carry the hint: {with_sym}"
    );
    assert!(
        without_sym.get("completeness").is_none(),
        "leaf symbol must not carry a hint: {without_sym}"
    );
    // Added bytes per lean response = hint object + its key and separators
    // in the serialized symbol. Constant-size by construction (fixed keys,
    // fixed remedy string, one counter, optional ceiling) and small in
    // practice — bounded well under 200 chars, i.e. ~0.8% of the 24K cap
    // for a single-symbol response, and never grows with dependent count.
    let hint_json = serde_json::to_string(&with_sym["completeness"]).unwrap();
    assert!(
        hint_json.len() < 200,
        "hint must be constant-size, got {hint_json} ({} chars)",
        hint_json.len()
    );
    let delta = with_sym
        .to_string()
        .len()
        .saturating_sub(without_sym.to_string().len());
    assert!(
        delta < 200,
        "per-symbol hint cost must stay small, got {delta} chars"
    );
    // The hint adds no new cap logic: the full lean response still sits
    // under the hard 24K cap, enforced by the existing cap_response path.
    assert!(
        with_hint.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "lean response with hint must stay within the 24K cap, got {} chars",
        with_hint.chars().count()
    );
}

/// A hub with hundreds of dependents produces a small constant-size hint and
/// the lean response stays within the hard 24,000-char cap (the hint is a
/// count, never a list — so the hub's dependent count cannot blow the cap).
#[test]
fn lean_response_hub_stays_within_24k_cap() {
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    storage.upsert_entity(&hub).unwrap();
    for i in 0..200 {
        let id = format!("p:repo1:file:imp_{i:03}.rs");
        let ent = file_entity(
            &id,
            &project_id,
            &repo_id,
            &format!("imp_{i:03}"),
            &format!("imp_{i:03}.rs"),
            "Rust",
        );
        storage.upsert_entity(&ent).unwrap();
        dep_edge(&storage, &id, &hub.id, RelType::Imports);
    }

    let tool = make_tool(storage, project_id, std::path::PathBuf::from("/tmp"));
    let result = lean_call(&tool, "hub");
    let raw = result.to_string();
    assert!(
        raw.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "response must be within the 24K hard cap, got {} chars",
        raw.chars().count()
    );
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    let hint = sym["completeness"]
        .as_object()
        .expect("hub with 200 dependents must carry a structured hint");
    assert_eq!(hint["omitted_direct_dependents"], 200);
    assert_eq!(
        hint["retrieval_ceiling"],
        crate::retrieval::tools_explore_blast::MAX_BLAST_ENTRIES,
        "the ceiling must track MAX_BLAST_ENTRIES (issue #854 lowered it to 25)"
    );
}
