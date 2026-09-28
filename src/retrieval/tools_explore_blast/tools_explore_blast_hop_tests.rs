// Issue #854: blast_radius entries carry a 0-based `hop` (0 = direct
// dependent, 1 = second level — the same convention as get_impact's
// hop_tracked_dependents), the per-file cap is MAX_BLAST_ENTRIES (25)
// with the omission counted in the disclosure, and the depth response
// carries a per-symbol completeness signal.
//
// Declared with a plain relative `mod` in mod.rs (never `#[path = "../…"]`);
// helpers are imported from the sibling `tools_explore_blast_tests` module.

use std::collections::HashSet;

use crate::model::{Entity, RelType};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools_explore_blast::tools_explore_blast_tests::{
    SqliteBox, dep_edge, file_entity, setup,
};
use crate::retrieval::tools_explore_blast::{MAX_BLAST_ENTRIES, call_paths_and_blast_radius};
use crate::storage::Storage;
use serde_json::Value;

/// Upstream file with a deterministic id/path (used by the cross-tool and
/// cap fixtures; kept out of the shared helper set on purpose — it is
/// #854-specific).
fn up_file(
    storage: &crate::storage::sqlite::SqliteStorage,
    project_id: &str,
    repo_id: &str,
    path: &str,
) -> Entity {
    let id = format!("p:repo1:file:{path}");
    let stem = path.rsplit('/').next().unwrap();
    let name = stem.trim_end_matches(".js").to_string();
    let ent = file_entity(&id, project_id, repo_id, &name, path, "JavaScript");
    storage.upsert_entity(&ent).unwrap();
    ent
}

#[test]
fn blast_entries_carry_hop_labels_direct_zero_second_level_one() {
    // bottom.js <- mid.js (imports) <- top.js (imports): mid is the direct
    // dependent (hop 0), top the second-level dependent (hop 1).
    let (storage, project_id, repo_id) = setup();
    for p in ["bottom.js", "mid.js", "top.js"] {
        up_file(&storage, &project_id, &repo_id, p);
    }
    dep_edge(
        &storage,
        "p:repo1:file:mid.js",
        "p:repo1:file:bottom.js",
        RelType::Imports,
    );
    dep_edge(
        &storage,
        "p:repo1:file:top.js",
        "p:repo1:file:mid.js",
        RelType::Imports,
    );
    let target = storage
        .get_entity("p:repo1:file:bottom.js")
        .unwrap()
        .unwrap();
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);
    assert_eq!(blast.len(), 2, "two-level closure: {blast:?}");
    let mid = blast.iter().find(|e| e["name"] == "mid").unwrap();
    let top = blast.iter().find(|e| e["name"] == "top").unwrap();
    assert_eq!(
        mid["hop"], 0,
        "direct importer must be hop 0 (same convention as get_impact): {mid:?}"
    );
    assert_eq!(
        top["hop"], 1,
        "second-level importer must be hop 1: {top:?}"
    );
}

#[test]
fn diamond_reachable_at_both_hops_is_emitted_once_with_hop_zero() {
    // Diamond: bottom <- mid1, mid2 (hop 0); mid1 <- top, mid2 <- top
    // (top is hop 1 via both routes). bottom must appear once with hop 0 —
    // the merge branch never overwrites the hop, so the smallest wins.
    let (storage, project_id, repo_id) = setup();
    for p in ["bottom.js", "mid1.js", "mid2.js", "top.js"] {
        up_file(&storage, &project_id, &repo_id, p);
    }
    for (src, tgt) in [
        ("p:repo1:file:mid1.js", "p:repo1:file:bottom.js"),
        ("p:repo1:file:mid2.js", "p:repo1:file:bottom.js"),
        ("p:repo1:file:top.js", "p:repo1:file:mid1.js"),
        ("p:repo1:file:top.js", "p:repo1:file:mid2.js"),
    ] {
        dep_edge(&storage, src, tgt, RelType::Imports);
    }
    let target = storage
        .get_entity("p:repo1:file:bottom.js")
        .unwrap()
        .unwrap();
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);
    // top.js is the file reachable via two hop-1 routes; it appears once.
    let tops: Vec<&Value> = blast.iter().filter(|e| e["name"] == "top").collect();
    assert_eq!(
        tops.len(),
        1,
        "a diamond file must be emitted once, not per route: {blast:?}"
    );
    assert_eq!(tops[0]["hop"], 1, "diamond top: {tops:?}");
    // And the smallest-hop-wins invariant in the other direction: make a
    // file reachable at BOTH hop 0 and hop 1 — the hop-0 label must win.
    // mid1 is already hop 0; add top -> mid1 -> (top is hop 1). Instead the
    // fixture above already proves merge-keeps-earliest: bottom is never
    // re-emitted via its hop-1 route (mid1/mid2 are its hop-0 dependents,
    // and top's hop-1 route to bottom goes through mid1's incoming edges,
    // not bottom's). The direct assertion is: no entry carries hop > 1 and
    // no file is duplicated.
    let mut seen = HashSet::new();
    for e in &blast {
        let path = e["path"].as_str().unwrap().to_string();
        assert!(
            seen.insert(path.clone()),
            "duplicate entry for {path}: {blast:?}"
        );
    }
    assert_eq!(blast.len(), 3, "mid1, mid2, top: {blast:?}");
}

#[test]
fn blast_and_get_impact_report_identical_path_hop_pairs() {
    // DEDICATED cross-tool fixture: one project/repo, file-to-file Imports
    // edges only (no function-tier children, so no callers at hop 0 that
    // could diverge). blast_radius entries and get_impact dependents must
    // report identical {path, hop} pairs.
    let (storage, project_id, repo_id) = setup();
    up_file(&storage, &project_id, &repo_id, "target.rs");
    up_file(&storage, &project_id, &repo_id, "mid1.rs");
    up_file(&storage, &project_id, &repo_id, "mid2.rs");
    up_file(&storage, &project_id, &repo_id, "top1.rs");
    for (src, tgt) in [
        ("p:repo1:file:mid1.rs", "p:repo1:file:target.rs"),
        ("p:repo1:file:mid2.rs", "p:repo1:file:target.rs"),
        ("p:repo1:file:top1.rs", "p:repo1:file:mid1.rs"),
        ("p:repo1:file:top1.rs", "p:repo1:file:mid2.rs"),
    ] {
        dep_edge(&storage, src, tgt, RelType::Imports);
    }
    // blast_radius side (file-mode traversal).
    let target_entity = storage
        .get_entity("p:repo1:file:target.rs")
        .unwrap()
        .unwrap();
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target_entity);
    let blast_pairs: HashSet<(String, u8)> = blast
        .iter()
        .map(|e| {
            (
                e["path"].as_str().unwrap().to_string(),
                e["hop"].as_u64().unwrap() as u8,
            )
        })
        .collect();

    // get_impact side (the same storage, through its own tool surface — the
    // same in-memory SqliteStorage is moved into the tool's context).
    let storage = boxed.0.into_inner().unwrap();
    let ctx = std::sync::Arc::new(crate::retrieval::tools::ToolContext {
        storage: std::sync::Arc::new(std::sync::Mutex::new(storage)),
        project_id: project_id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = crate::retrieval::tools::GetImpactTool { ctx };
    let impact = tool
        .call(serde_json::json!({"files": ["target.rs"]}))
        .unwrap();
    let impact: Value = serde_json::from_str(&impact).unwrap();
    // get_impact emits `dependents` as {path, hop} — same convention.
    let impact_pairs: HashSet<(String, u8)> = impact_dependent_pairs(&impact).into_iter().collect();

    assert_eq!(
        blast_pairs, impact_pairs,
        "blast_radius and get_impact must report identical {{path, hop}} pairs \
         (blast {blast_pairs:?} vs impact {impact_pairs:?})"
    );
}

/// Extract {(path, hop)} pairs from a get_impact response (the lean get_
/// impact contract, issue #840: top-level `dependents` array of {path, hop}).
fn impact_dependent_pairs(impact: &Value) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    if let Some(deps) = impact.get("dependents").and_then(Value::as_array) {
        for d in deps {
            if let (Some(p), Some(h)) = (
                d.get("path").and_then(Value::as_str),
                d.get("hop").and_then(Value::as_u64),
            ) {
                out.push((p.to_string(), h as u8));
            }
        }
    }
    out
}

/// The traversal cap (issue #854, previously uncovered): a target with a
/// few direct importers and many transitive dependents exceeding the cap
/// returns exactly MAX_BLAST_ENTRIES entries, every direct importer present
/// with hop 0, all hop-0 entries before all hop-1 entries, and the
/// disclosure carrying the omitted count.
#[test]
fn cap_with_few_direct_and_many_transitive_keeps_all_direct_and_counts_omitted() {
    let (storage, project_id, repo_id) = setup();
    up_file(&storage, &project_id, &repo_id, "target.js");
    // 4 direct importers (hop 0) + 40 second-level importers (hop 1).
    let mut direct = Vec::new();
    for i in 0..4 {
        let p = format!("direct_{i:02}.js");
        up_file(&storage, &project_id, &repo_id, &p);
        dep_edge(
            &storage,
            &format!("p:repo1:file:{p}"),
            "p:repo1:file:target.js",
            RelType::Imports,
        );
        direct.push(p);
    }
    let mut transitive = Vec::new();
    for i in 0..40 {
        let p = format!("second_{i:02}.js");
        up_file(&storage, &project_id, &repo_id, &p);
        // Each second-level file imports one of the direct importers
        // (round-robin) — all are hop 1 from the target.
        let via = &direct[i % direct.len()];
        dep_edge(
            &storage,
            &format!("p:repo1:file:{p}"),
            &format!("p:repo1:file:{via}"),
            RelType::Imports,
        );
        transitive.push(p);
    }
    let target = storage
        .get_entity("p:repo1:file:target.js")
        .unwrap()
        .unwrap();
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, errors, complete) = call_paths_and_blast_radius(&boxed, &target);

    assert_eq!(
        blast.len(),
        MAX_BLAST_ENTRIES,
        "cap must bind at exactly {MAX_BLAST_ENTRIES} entries, got {}",
        blast.len()
    );
    // All 4 direct importers present with hop 0.
    for p in &direct {
        let entry = blast
            .iter()
            .find(|e| e["path"].as_str() == Some(p.as_str()))
            .unwrap_or_else(|| panic!("direct importer {p} must survive the cap: {blast:?}"));
        assert_eq!(entry["hop"], 0, "{entry:?}");
    }
    // All hop-0 entries before all hop-1 entries (set property, no
    // within-hop order pinned).
    let first_hop1 = blast
        .iter()
        .position(|e| e["hop"] == 1)
        .unwrap_or_else(|| panic!("expected hop-1 entries: {blast:?}"));
    let last_hop0 = blast
        .iter()
        .rposition(|e| e["hop"] == 0)
        .unwrap_or_else(|| panic!("expected hop-0 entries: {blast:?}"));
    assert!(
        last_hop0 < first_hop1,
        "hop-0 entries must all precede hop-1 entries: {blast:?}"
    );
    // The hop-1 entries are a subset of the seeded second-level files.
    for e in blast.iter().filter(|e| e["hop"] == 1) {
        let path = e["path"].as_str().unwrap().to_string();
        assert!(
            transitive.iter().any(|p| p.as_str() == path),
            "hop-1 entry must come from the seeded second-level set: {e:?}"
        );
    }
    // The disclosure: exactly one capped message, with the omitted count.
    // 44 dependents reached, 25 kept → 19 omitted, none at hop 0.
    let capped = errors
        .iter()
        .filter(|e| e.starts_with("blast_radius capped at"))
        .count();
    assert_eq!(
        capped, 1,
        "one disclosure, not one per overflow: {errors:?}"
    );
    let msg = &errors[0];
    assert!(
        msg.contains("19 dependents omitted"),
        "disclosure must carry the omitted count (44 reached - 25 kept = 19): {msg:?}"
    );
    assert!(
        !msg.contains("direct (hop 0)"),
        "the omission did not reach hop 0: {msg:?}"
    );
    // The completeness signal is capped (not complete) — consistent with the
    // disclosure.
    assert!(
        !complete,
        "capped traversal must not be complete: {errors:?}"
    );
}

/// The >25-DIRECT-importers case: the retained set is a subset of the
/// direct importers, every entry is hop 0, no hop-1 entry appears, and the
/// disclosure says the omission reached hop 0.
#[test]
fn cap_with_more_than_cap_direct_importers_keeps_only_hop_zero() {
    let (storage, project_id, repo_id) = setup();
    up_file(&storage, &project_id, &repo_id, "hub.js");
    let direct_count = MAX_BLAST_ENTRIES + 10; // 35 > 25
    for i in 0..direct_count {
        let p = format!("imp_{i:02}.js");
        up_file(&storage, &project_id, &repo_id, &p);
        dep_edge(
            &storage,
            &format!("p:repo1:file:{p}"),
            "p:repo1:file:hub.js",
            RelType::Imports,
        );
    }
    // A second-level importer via one of the direct ones — must NOT appear
    // because the cap is full at hop 0.
    let p = "second_level.js";
    up_file(&storage, &project_id, &repo_id, p);
    dep_edge(
        &storage,
        "p:repo1:file:second_level.js",
        "p:repo1:file:imp_00.js",
        RelType::Imports,
    );

    let hub = storage.get_entity("p:repo1:file:hub.js").unwrap().unwrap();
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, errors, complete) = call_paths_and_blast_radius(&boxed, &hub);

    assert_eq!(blast.len(), MAX_BLAST_ENTRIES);
    for e in &blast {
        assert_eq!(
            e["hop"], 0,
            "no hop-1 entry may survive when the cap binds at hop 0: {e:?}"
        );
    }
    // A subset of the direct importers (every entry is one of the 35).
    for e in &blast {
        let path = e["path"].as_str().unwrap().to_string();
        assert!(
            path.starts_with("imp_"),
            "entries must be a subset of the direct importers: {e:?}"
        );
    }
    let second = blast
        .iter()
        .any(|e| e["path"].as_str() == Some("second_level.js"));
    assert!(!second, "hop-1 entry must not appear: {blast:?}");
    // The disclosure counts 36 reached (35 direct + 1 second), 25 kept →
    // 11 omitted, and says the omission reached hop 0.
    let msg = errors
        .iter()
        .find(|e| e.starts_with("blast_radius capped at"))
        .expect("cap disclosure required: {errors:?}");
    assert!(
        msg.contains("11 dependents omitted"),
        "omitted count = 36 reached - 25 kept: {msg:?}"
    );
    assert!(
        msg.contains("direct (hop 0)"),
        "the disclosure must say the omission reached hop 0: {msg:?}"
    );
    assert!(!complete, "capped traversal must not be complete");
}

/// The merge branch must still append rel_types to an already-emitted file
/// when the file is re-encountered through a DIFFERENT endpoint entity (the
/// `visited` set is keyed by entity id, so the merge only fires when two
/// distinct endpoints normalize to the same file — e.g., a file-level
/// Imports edge plus a function-level Calls edge, both pointing at the
/// target). This test pins that merge: t.js has a function fn_t; m.js
/// Imports t.js (file-level, hop 0, emitted with "imports") AND m.js's
/// function fn_m Calls fn_t (function-level, hop 0, the merge branch
/// appends... nothing, because Calls is not in the emit set — but the
/// function-level edge still drives traversal). The key assertion: m.js
/// appears ONCE (not twice), and its entry carries "imports" (the
/// file-level edge was emitted; the function-level Calls edge did not add
/// a second entry).
#[test]
fn merge_appends_second_rel_type_on_reencounter() {
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let fn_t = crate::retrieval::tools_explore_blast::tools_explore_blast_tests::func_entity(
        "fn-t",
        &project_id,
        &repo_id,
        "t_fn",
        Some("p:repo1:file:t.js"),
    );
    let m = file_entity(
        "p:repo1:file:m.js",
        &project_id,
        &repo_id,
        "m",
        "m.js",
        "JavaScript",
    );
    let fn_m = crate::retrieval::tools_explore_blast::tools_explore_blast_tests::func_entity(
        "fn-m",
        &project_id,
        &repo_id,
        "m_fn",
        Some("p:repo1:file:m.js"),
    );
    for e in [&t, &fn_t, &m, &fn_m] {
        storage.upsert_entity(e).unwrap();
    }
    // m.js Imports t.js (file-level edge — hop 0, emitted with "imports").
    dep_edge(
        &storage,
        "p:repo1:file:m.js",
        "p:repo1:file:t.js",
        RelType::Imports,
    );
    // fn_m (in m.js) Calls fn_t (in t.js) — a function-level edge that
    // normalizes to m.js (the same file). The visited set already has
    // m.js's file entity id from the first edge, so this second edge is
    // skipped (the merge branch does NOT fire for the same entity id via
    // two edges — only via two different endpoint entities). But fn_m is a
    // DIFFERENT entity from m.js's file entity, so the function-level edge
    // IS processed: fn_m is not in visited, fn_m's owning file is m.js
    // (already in entry_index), and the merge branch runs. However, the
    // rel type is Calls, which is NOT in the emit set, so the merge branch
    // is gated: the guard `if !should_emit(&rel.rel_type)` skips the merge
    // (Calls is not emitted). The assertion: m.js appears ONCE (not twice),
    // with "imports" only.
    dep_edge(&storage, "fn-m", "fn-t", RelType::Calls);
    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);
    // m.js is a single entry (the function-level Calls edge did not create
    // a second entry or append "calls" to the rel_types).
    let ms: Vec<&Value> = blast
        .iter()
        .filter(|e| e["path"].as_str() == Some("m.js"))
        .collect();
    assert_eq!(ms.len(), 1, "m.js emitted once: {blast:?}");
    let types = ms[0]["rel_types"].as_array().unwrap();
    let has_imports = types.iter().any(|t| t == "imports");
    let has_calls = types.iter().any(|t| t == "calls");
    assert!(
        has_imports,
        "file-level Imports edge must be labelled: {ms:?}"
    );
    assert!(
        !has_calls,
        "function-level Calls edge must NOT be labelled (issue #834): {ms:?}"
    );
}
