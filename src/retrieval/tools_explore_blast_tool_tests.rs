// Issue #759: tool-level (end-to-end) tests for blast_radius through
// ExploreTool. Wired via `#[path]` from tools_explore.rs.

use std::sync::{Arc, Mutex};

use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use serde_json::Value;

// Reuse the shared helpers from the sibling blast-radius test module.
use crate::retrieval::tools_explore_blast::tools_explore_blast_tests::{
    dep_edge, file_entity, setup,
};

use crate::retrieval::tools_explore_blast::call_paths_and_blast_radius;

fn explore_call(storage: SqliteStorage, project_id: String, query: &str) -> Value {
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: std::path::PathBuf::from("/tmp"),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    let result = tool
        .call(serde_json::json!({"query": query, "include_depth": true}))
        .unwrap();
    serde_json::from_str(&result).unwrap()
}

#[test]
fn tool_level_js_component_imported_returns_importers_in_blast_radius() {
    // End-to-end: the BannerSection regression through the tool surface.
    let (storage, project_id, repo_id) = setup();
    let target = file_entity(
        "p:repo1:file:src/banner.tsx",
        &project_id,
        &repo_id,
        "banner",
        "src/banner.tsx",
        "TypeScript",
    );
    storage.upsert_entity(&target).unwrap();
    for path in ["src/home.tsx", "src/store.tsx"] {
        let id = format!("p:repo1:file:{path}");
        let name = path
            .rsplit('/')
            .next()
            .unwrap()
            .strip_suffix(".tsx")
            .unwrap();
        let ent = file_entity(&id, &project_id, &repo_id, name, path, "TypeScript");
        storage.upsert_entity(&ent).unwrap();
        dep_edge(&storage, &id, &target.id, crate::model::RelType::Imports);
    }

    let parsed = explore_call(storage, project_id, "banner");
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert_eq!(sym["name"], "banner");
    let blast = sym["blast_radius"].as_array().unwrap();
    assert_eq!(blast.len(), 2, "both importers reported: {blast:?}");
    for entry in blast {
        assert_eq!(entry["direction"], "in");
        assert_eq!(
            entry["rel_types"],
            Value::from(vec![Value::from("imports")])
        );
    }
}

#[test]
fn hub_with_large_reverse_closure_stays_within_24k_cap_and_discloses() {
    // A hub file imported by many files: the response must stay within
    // MAX_EXPLORE_OUTPUT_CHARS. With 450 importers (long paths) the
    // serialized blast radius deterministically exceeds the 24K cap, so the
    // truncation signal and not-shown/completeness disclosure are exercised.
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.js",
        &project_id,
        &repo_id,
        "hub",
        "hub.js",
        "JavaScript",
    );
    storage.upsert_entity(&hub).unwrap();
    // issue #854 lowered MAX_BLAST_ENTRIES from 100 to 25, so a 25-entry
    // blast_radius no longer overflows the 24K output cap on its own (the
    // old 100-entry × 155-char fixture overflowed at ~15.5K + framing; 25
    // entries × ~155 chars is only ~3.9K). With 2000-char paths the 25
    // capped entries × ~2000 chars = ~50K, which deterministically exceeds
    // the 24K cap, so the response is truncated (cap_response path) with
    // the truncation signal, and the blast_radius is capped at 25 with the
    // omission disclosed.
    for i in 0..450 {
        let path = format!(
            "src/modules/importers/subsystem/component_module/importer_number_{}_{}/component.js",
            i,
            "a".repeat(2000)
        );
        let id = format!("p:repo1:file:{path}");
        let name = format!("importer_{i:03}");
        let ent = file_entity(&id, &project_id, &repo_id, &name, &path, "JavaScript");
        storage.upsert_entity(&ent).unwrap();
        dep_edge(&storage, &id, &hub.id, crate::model::RelType::Imports);
    }

    let result = explore_call(storage, project_id, "hub");

    // The whole response must respect the hard cap (chars, not bytes —
    // cap_response measures chars).
    let raw = serde_json::to_string(&result).unwrap();
    assert!(
        raw.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "response must be within the 24K hard cap, got {} chars",
        raw.chars().count()
    );
    // With 2000-char paths the 25-entry blast_radius deterministically
    // exceeds the 24K cap, so the truncation signal MUST be present.
    assert_eq!(
        result.get("truncated"),
        Some(&Value::from(true)),
        "2000-char-path fixture must exceed the 24K cap and trigger truncation: {result:?}"
    );
    // The truncated response carries a structured truncation object naming
    // what was dropped (issue #836 value-level cap contract).
    assert!(
        result.get("truncation").is_some(),
        "truncated response must carry a structured truncation object: {result:?}"
    );
}

#[test]
fn blast_radius_does_not_leak_across_project_boundaries() {
    // Two projects share one database. Project A has file target.js; project
    // B has file foreign.js that imports target.js (a cross-project edge —
    // corrupt data, but the traversal must not leak B's file into A's blast
    // radius and vice versa). Entity ids embed the project id, but the
    // relationships table has no per-project column, so the traversal
    // itself must enforce the boundary via the source entity's project_id.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let pa = storage
        .create_project(&format!("blast-a-{unique}"), None)
        .unwrap();
    let pb = storage
        .create_project(&format!("blast-b-{unique}"), None)
        .unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

    let target = file_entity(
        "pa:repoA:file:target.js",
        &pa.id,
        &repo_a.id,
        "target",
        "target.js",
        "JavaScript",
    );
    let foreign = file_entity(
        "pb:repoB:file:foreign.js",
        &pb.id,
        &repo_b.id,
        "foreign",
        "foreign.js",
        "JavaScript",
    );
    for e in [&target, &foreign] {
        storage.upsert_entity(e).unwrap();
    }
    // Cross-project edge: foreign.js (project B) imports target.js (project A).
    dep_edge(
        &storage,
        "pb:repoB:file:foreign.js",
        "pa:repoA:file:target.js",
        crate::model::RelType::Imports,
    );

    // Blast radius of the A-project target: foreign.js (project B) must not
    // appear.
    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);
    // Identify by `path` (entity_id is no longer emitted on blast entries,
    // issue #834): the cross-project foreign.js must not appear.
    let leaked = blast.iter().any(|e| e["path"] == "foreign.js");
    assert!(
        !leaked,
        "cross-project importer must not leak into blast radius: {blast:?}"
    );
}

#[test]
fn blast_radius_two_hop_path_does_not_leak_across_project_boundaries() {
    // Issue #764 multi-hop: the project check must hold at EVERY hop of the
    // reverse closure, not just the first. Chain: target (projA) ← mid
    // (projA, imports target) ← foreign (projB, imports mid — a genuine
    // cross-project edge). Without the per-hop check, the hop-2 expansion
    // of mid would pull foreign into A's blast radius through a
    // same-project intermediate.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let pa = storage
        .create_project(&format!("blast2h-a-{unique}"), None)
        .unwrap();
    let pb = storage
        .create_project(&format!("blast2h-b-{unique}"), None)
        .unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

    let target = file_entity(
        "pa:repoA:file:target.js",
        &pa.id,
        &repo_a.id,
        "target",
        "target.js",
        "JavaScript",
    );
    let mid = file_entity(
        "pa:repoA:file:mid.js",
        &pa.id,
        &repo_a.id,
        "mid",
        "mid.js",
        "JavaScript",
    );
    let foreign = file_entity(
        "pb:repoB:file:foreign.js",
        &pb.id,
        &repo_b.id,
        "foreign",
        "foreign.js",
        "JavaScript",
    );
    for e in [&target, &mid, &foreign] {
        storage.upsert_entity(e).unwrap();
    }
    // Same-project hop-1 edge: mid (projA) imports target (projA).
    dep_edge(
        &storage,
        "pa:repoA:file:mid.js",
        "pa:repoA:file:target.js",
        crate::model::RelType::Imports,
    );
    // Cross-project hop-2 edge: foreign (projB) imports mid (projA).
    dep_edge(
        &storage,
        "pb:repoB:file:foreign.js",
        "pa:repoA:file:mid.js",
        crate::model::RelType::Imports,
    );

    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);
    // mid is a legitimate same-project dependent and must be present.
    // (Identify by `path`: entity_id is no longer emitted, issue #834.)
    assert!(
        blast.iter().any(|e| e["path"] == "mid.js"),
        "same-project hop-1 dependent mid.js must be in the blast radius: {blast:?}"
    );
    // foreign is two hops away via a cross-project edge and must NOT be.
    let leaked = blast.iter().any(|e| e["path"] == "foreign.js");
    assert!(
        !leaked,
        "cross-project 2-hop dependent must not leak into blast radius: {blast:?}"
    );
}

#[test]
fn call_paths_do_not_leak_across_project_boundaries() {
    // Issue #764: the sibling of the #759 blast_radius regression for the
    // call_paths half of the same traversal. Two projects share one
    // database; project B's foreign_fn calls project A's target_fn (a
    // cross-project edge — corrupt data, but the traversal must not leak).
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let pa = storage
        .create_project(&format!("cp-a-{unique}"), None)
        .unwrap();
    let pb = storage
        .create_project(&format!("cp-b-{unique}"), None)
        .unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();

    let target_file = file_entity(
        "pa:repoA:file:target.js",
        &pa.id,
        &repo_a.id,
        "target",
        "target.js",
        "JavaScript",
    );
    let target_fn = crate::retrieval::tools_explore_blast::tools_explore_blast_tests::func_entity(
        "pa:repoA:fn:target_fn",
        &pa.id,
        &repo_a.id,
        "target_fn",
        Some("pa:repoA:file:target.js"),
    );
    let foreign_file = file_entity(
        "pb:repoB:file:foreign.js",
        &pb.id,
        &repo_b.id,
        "foreign",
        "foreign.js",
        "JavaScript",
    );
    let foreign_fn = crate::retrieval::tools_explore_blast::tools_explore_blast_tests::func_entity(
        "pb:repoB:fn:foreign_fn",
        &pb.id,
        &repo_b.id,
        "foreign_fn",
        Some("pb:repoB:file:foreign.js"),
    );
    for e in [&target_file, &target_fn, &foreign_file, &foreign_fn] {
        storage.upsert_entity(e).unwrap();
    }
    // Cross-project edge: foreign_fn (project B) calls target_fn (project A).
    dep_edge(
        &storage,
        "pb:repoB:fn:foreign_fn",
        "pa:repoA:fn:target_fn",
        crate::model::RelType::Calls,
    );

    // Project A: the foreign caller must not appear in target_fn's
    // call_paths (neither direction), and blast radius must stay empty.
    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let (call_paths, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target_file);
    assert!(
        call_paths.is_empty(),
        "cross-project call must not leak into call_paths: {call_paths:?}"
    );
    assert!(
        blast.is_empty(),
        "cross-project call must not leak into blast radius: {blast:?}"
    );
}

#[test]
fn deep_containment_chain_normalizes_to_owning_file() {
    // The A2 regression: an endpoint nested two levels below its file
    // (file -> class -> method) must normalize to the OWNING FILE, not be
    // dropped by a single-level parent lookup. The method is a direct
    // dependent (it calls a function in t.js) and must appear as f.js.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "TypeScript",
    );
    let f = file_entity(
        "p:repo1:file:f.js",
        &project_id,
        &repo_id,
        "f",
        "f.js",
        "TypeScript",
    );
    let cls = crate::model::Entity {
        id: "p:repo1:cls:User".to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: crate::model::EntityTier::Function,
        parent_id: Some("p:repo1:file:f.js".to_string()),
        name: "User".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let method = crate::model::Entity {
        id: "p:repo1:fn:User.save".to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: crate::model::EntityTier::Function,
        parent_id: Some("p:repo1:cls:User".to_string()),
        name: "save".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let t_fn = crate::retrieval::tools_explore_blast::tools_explore_blast_tests::func_entity(
        "fn-t",
        &project_id,
        &repo_id,
        "t_fn",
        Some("p:repo1:file:t.js"),
    );
    for e in [&t, &f, &cls, &method, &t_fn] {
        storage.upsert_entity(e).unwrap();
    }
    // f.js imports t.js (file -> file) — gives f.js a hop-1 entry.
    dep_edge(
        &storage,
        "p:repo1:file:f.js",
        "p:repo1:file:t.js",
        crate::model::RelType::Imports,
    );
    // The method (two levels below f.js) calls a function in t.js.
    dep_edge(
        &storage,
        "p:repo1:fn:User.save",
        "fn-t",
        crate::model::RelType::Calls,
    );

    let boxed =
        crate::retrieval::tools_explore_blast::tools_explore_blast_tests::SqliteBox::new(storage);
    let (_, blast, _, blast_errors, _complete) =
        crate::retrieval::tools_explore_blast::call_paths_and_blast_radius(&boxed, &t);

    assert!(
        blast_errors.is_empty(),
        "no storage errors expected on a clean graph: {blast_errors:?}"
    );
    // The deep method and the file import collapse to ONE entry for f.js,
    // not zero (old single-level lookup) and not two.
    assert_eq!(blast.len(), 1, "f.js must appear once: {blast:?}");
    let entry = &blast[0];
    // Identify by `path` (entity_id is no longer emitted, issue #834).
    assert_eq!(entry["path"], "f.js");
    assert_eq!(entry["name"], "f");
    let types = entry["rel_types"].as_array().unwrap();
    // The deep method reaches f.js through a CALLS edge and the file through
    // an IMPORTS edge. The calls edge still drives traversal but is no
    // longer labelled (issue #834), so only "imports" is emitted.
    assert_eq!(
        types.len(),
        1,
        "only the non-call rel type is labelled: {entry:?}"
    );
    assert!(
        types.iter().any(|t| t == "imports"),
        "imports must be listed: {entry:?}"
    );
    assert!(
        !types.iter().any(|t| t == "calls"),
        "calls must not be listed (issue #834): {entry:?}"
    );
}
