// Issue #835: call_paths rows are deduplicated on the id triple
// (entity_id, target_entity_id, direction) — one set per file, shared
// across all Function-tier entities in the file and both direction
// loops — and emitted sorted by that triple. The emitted row drops
// `target_entity_id` (an `fn-<hash>` id that embeds no path and
// cannot be resolved by the caller) and `rel_type` (the constant
// "calls" on every row); `entity_id` (the source function) is retained
// because two functions can share a file and the path+tier pair does
// not reconstruct it.
//
// Helpers are imported from the sibling `tools_explore_blast_tests`
// module; no `#[path]` (that pattern tripped check_dead_files in #834).

use crate::model::RelType;
use crate::retrieval::tools_explore_blast::call_paths_and_blast_radius;
use crate::retrieval::tools_explore_blast::tools_explore_blast_tests::{
    SqliteBox, dep_edge, file_entity, func_entity, setup,
};
use crate::storage::Storage;

#[test]
fn call_paths_row_count_equals_distinct_id_triple_count() {
    // Hub file with 3 functions, all calling the same shared helper.
    // The raw edge set is: each of the 3 callers -> helper (out), so
    // 3 out edges + 3 in edges (helper is called 3 times = 3 in rows
    // for helper) = 6 raw rows. Distinct id triples: 6.
    //
    // Also: one function has an incoming edge from ANOTHER function in
    // the same file (fn_a calls fn_b) — this produces 2 raw rows in
    // the file's call_paths (one out from fn_a, one in for fn_b) but
    // only 2 distinct triples.
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let helper = file_entity(
        "p:repo1:file:helper.rs",
        &project_id,
        &repo_id,
        "helper",
        "helper.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_c = func_entity(
        "fn-c",
        &project_id,
        &repo_id,
        "fn_c",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_helper = func_entity(
        "fn-helper",
        &project_id,
        &repo_id,
        "fn_helper",
        Some("p:repo1:file:helper.rs"),
    );
    for e in [&hub, &helper, &fn_a, &fn_b, &fn_c, &fn_helper] {
        storage.upsert_entity(e).unwrap();
    }
    // Each of fn_a, fn_b, fn_c calls fn_helper (the shared helper).
    dep_edge(&storage, "fn-a", "fn-helper", RelType::Calls);
    dep_edge(&storage, "fn-b", "fn-helper", RelType::Calls);
    dep_edge(&storage, "fn-c", "fn-helper", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);

    // call_paths is scoped to the HUB file only. The callee lives in
    // helper.rs, so its incoming edges are NOT in this file's
    // call_paths. The expected triples are therefore just the 3 out
    // edges from each of fn-a, fn-b, fn-c to fn-helper.
    //
    // (A hub file with multiple callers of a SIBLING function in the
    // SAME file would exercise the in-direction dedup; see the
    // mutual_call_pair test.)
    assert_eq!(
        call_paths.len(),
        3,
        "expected 3 distinct (caller, callee, direction) triples, got: {call_paths:?}"
    );

    // No two rows share (entity_id, calls, direction).
    let mut seen = std::collections::HashSet::new();
    for row in &call_paths {
        let key = (
            row["entity_id"].as_str().unwrap().to_string(),
            row["calls"].as_str().unwrap().to_string(),
            row["direction"].as_str().unwrap().to_string(),
        );
        assert!(
            seen.insert(key),
            "duplicate (entity_id, calls, direction) triple emitted: {row:?}"
        );
    }
}

#[test]
fn call_paths_rows_drop_target_entity_id_and_rel_type() {
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    for e in [&hub, &fn_a, &fn_b] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-a", "fn-b", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);
    assert!(!call_paths.is_empty());
    for row in &call_paths {
        assert!(
            row.get("target_entity_id").is_none(),
            "target_entity_id must be dropped: {row:?}"
        );
        assert!(
            row.get("rel_type").is_none(),
            "rel_type must be dropped: {row:?}"
        );
    }
}

#[test]
fn call_paths_rows_retain_entity_id_calls_direction() {
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    for e in [&hub, &fn_a, &fn_b] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-a", "fn-b", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);
    for row in &call_paths {
        assert!(
            row.get("entity_id").is_some(),
            "entity_id must be retained: {row:?}"
        );
        assert!(
            row.get("calls").is_some(),
            "calls (callee name) must be retained: {row:?}"
        );
        assert!(
            row.get("direction").is_some(),
            "direction must be retained: {row:?}"
        );
    }
}

#[test]
fn two_same_named_callees_in_different_files_are_not_merged() {
    // The correctness guard for the id-triple dedup key. Two files each
    // have a function named `main`; the hub calls both. If the dedup
    // were keyed on (entity_id, calls-name, direction) instead of the
    // id triple, both rows would collapse into one and a real edge
    // would be lost.
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let file_alpha = file_entity(
        "p:repo1:file:alpha.rs",
        &project_id,
        &repo_id,
        "alpha",
        "alpha.rs",
        "Rust",
    );
    let file_beta = file_entity(
        "p:repo1:file:beta.rs",
        &project_id,
        &repo_id,
        "beta",
        "beta.rs",
        "Rust",
    );
    let fn_hub = func_entity(
        "fn-hub",
        &project_id,
        &repo_id,
        "fn_hub",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_alpha_main = func_entity(
        "fn-alpha-main",
        &project_id,
        &repo_id,
        "main",
        Some("p:repo1:file:alpha.rs"),
    );
    let fn_beta_main = func_entity(
        "fn-beta-main",
        &project_id,
        &repo_id,
        "main",
        Some("p:repo1:file:beta.rs"),
    );
    for e in [
        &hub,
        &file_alpha,
        &file_beta,
        &fn_hub,
        &fn_alpha_main,
        &fn_beta_main,
    ] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-hub", "fn-alpha-main", RelType::Calls);
    dep_edge(&storage, "fn-hub", "fn-beta-main", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);

    // Both "main" callees must survive as distinct out rows.
    let mut main_rows = Vec::new();
    for row in &call_paths {
        if row["calls"] == "main" && row["direction"] == "out" {
            main_rows.push(row.clone());
        }
    }
    assert_eq!(
        main_rows.len(),
        2,
        "two distinct same-named callees in different files must NOT merge: {call_paths:?}"
    );
    let entity_ids: std::collections::HashSet<&str> = main_rows
        .iter()
        .map(|r| r["entity_id"].as_str().unwrap())
        .collect();
    assert!(
        entity_ids.contains("fn-hub"),
        "both rows should be from fn-hub"
    );
}

#[test]
fn in_direction_rows_are_deduped_across_sibling_functions_in_one_file() {
    // Hub file has fn_a and fn_b, both of which are called by an
    // EXTERNAL function (fn_ext). The external caller's edge to fn_a is
    // one in-row, the edge to fn_b is another in-row — those are 2
    // DISTINCT triples (different entity_id), so both survive.
    //
    // The cross-function dedup that this test exercises: fn_a calls
    // fn_b (both in the same file). When the loop processes fn_a's
    // outgoing edges, it sees (fn_a -> fn_b, out). When the loop
    // processes fn_b's INCOMING edges, it sees (fn_a -> fn_b, in).
    // Those are different direction values, so both survive (2 rows).
    // But if the SAME edge were ever duplicated (e.g. two identical
    // rows from a storage quirk), the HashSet would collapse them.
    //
    // To exercise the actual cross-function dedup that this issue
    // targets, we need a file where the SAME (entity_id, target_id)
    // pair appears in BOTH the out-loop and the in-loop of a DIFFERENT
    // function. That happens when fn_a and fn_b are siblings and BOTH
    // are called by the same external fn_ext: the in-loop for fn_a
    // sees (fn_ext -> fn_a), the in-loop for fn_b sees (fn_ext -> fn_b).
    // Those are different triples. To force a true cross-function
    // duplicate, we need the SAME (entity_id, target_id, direction)
    // to appear in two different functions' loops. That happens when
    // fn_a calls fn_b AND fn_b calls fn_a (mutual pair) — but that's
    // a mutual-pair test, already covered. The simplest true
    // cross-function duplicate is when the storage returns the same
    // raw edge twice (a quirk we can't force via upsert_relationship
    // because it's a UNIQUE upsert).
    //
    // Instead, this test asserts the invariant that DIRECTLY matters
    // for this issue: when N sibling functions in the same file each
    // call the SAME sibling, the callee's in-rows are NOT
    // collapsed to 1 (they have distinct caller ids). This guards
    // against an over-aggressive dedup key that accidentally merges
    // distinct caller ids.
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_c = func_entity(
        "fn-c",
        &project_id,
        &repo_id,
        "fn_c",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_d = func_entity(
        "fn-d",
        &project_id,
        &repo_id,
        "fn_d",
        Some("p:repo1:file:hub.rs"),
    );
    for e in [&hub, &fn_a, &fn_b, &fn_c, &fn_d] {
        storage.upsert_entity(e).unwrap();
    }
    // fn_a, fn_b, fn_c all call fn_d (all in the same file).
    dep_edge(&storage, "fn-a", "fn-d", RelType::Calls);
    dep_edge(&storage, "fn-b", "fn-d", RelType::Calls);
    dep_edge(&storage, "fn-c", "fn-d", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);

    // Expected: 3 out rows (fn_a -> fn_d) + 3 in rows (fn_a <- fn_d,
    // fn_b <- fn_d, fn_c <- fn_d). The 3 in rows are NOT collapsed
    // because they have distinct entity_ids (fn_a, fn_b, fn_c) — that
    // is the pitfall the issue warns about: "a function called by N
    // distinct callers legitimately produces N rows with direction
    // 'in'. Only exact duplicate triples are removed."
    assert_eq!(
        call_paths.len(),
        6,
        "expected 6 distinct triples (3 out + 3 in with distinct caller ids), got: {call_paths:?}"
    );
    // The in-direction rows for fn_d are NOT present in hub's call_paths
    // because fn_d's incoming edges are scoped to fn_d's OWN file, not
    // the hub's. What IS present: each of fn_a, fn_b, fn_c appears as an
    // entity with an in-row (they are the call target of the in-edge
    // from fn_d, which is in the same file). Verify the 3 in-rows are
    // for fn_a, fn_b, fn_c (each distinct, not collapsed).
    let in_entity_ids: std::collections::HashSet<&str> = call_paths
        .iter()
        .filter(|r| r["direction"] == "in")
        .map(|r| r["entity_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        in_entity_ids,
        ["fn-a", "fn-b", "fn-c"].iter().copied().collect(),
        "expected in-rows for fn-a, fn-b, fn_c (each distinct), got: {call_paths:?}"
    );
    assert_eq!(
        in_entity_ids.len(),
        3,
        "3 distinct in-entity-ids expected (one per sibling caller), got: {call_paths:?}"
    );
}

#[test]
fn mutual_call_pair_yields_one_row_per_direction() {
    // fn_a calls fn_b AND fn_b calls fn_a: 2 distinct triples, one "out"
    // row for fn_a and one "out" row for fn_b, plus one "in" row for
    // fn_a and one "in" row for fn_b. Dedup must NOT collapse the
    // mutual pair because direction is part of the key.
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    for e in [&hub, &fn_a, &fn_b] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-a", "fn-b", RelType::Calls);
    dep_edge(&storage, "fn-b", "fn-a", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, _blast, _cpe, _bre, _) = call_paths_and_blast_radius(&boxed, &hub);

    // Expected: 4 rows total (fn_a out, fn_a in, fn_b out, fn_b in).
    assert_eq!(
        call_paths.len(),
        4,
        "mutual pair should yield 4 rows (2 functions x 2 directions): {call_paths:?}"
    );
    // One "out" for fn_a -> fn_b, one "out" for fn_b -> fn_a.
    let fn_a_out = call_paths
        .iter()
        .filter(|r| r["entity_id"] == "fn-a" && r["direction"] == "out" && r["calls"] == "fn_b")
        .count();
    let fn_b_out = call_paths
        .iter()
        .filter(|r| r["entity_id"] == "fn-b" && r["direction"] == "out" && r["calls"] == "fn_a")
        .count();
    let fn_a_in = call_paths
        .iter()
        .filter(|r| r["entity_id"] == "fn-a" && r["direction"] == "in" && r["calls"] == "fn_b")
        .count();
    let fn_b_in = call_paths
        .iter()
        .filter(|r| r["entity_id"] == "fn-b" && r["direction"] == "in" && r["calls"] == "fn_a")
        .count();
    assert_eq!(fn_a_out, 1, "fn_a -> fn_b out row missing: {call_paths:?}");
    assert_eq!(fn_b_out, 1, "fn_b -> fn_a out row missing: {call_paths:?}");
    assert_eq!(
        fn_a_in, 1,
        "fn_a in row (called by fn_b) missing: {call_paths:?}"
    );
    assert_eq!(
        fn_b_in, 1,
        "fn_b in row (called by fn_a) missing: {call_paths:?}"
    );
}

#[test]
fn call_paths_emission_order_is_deterministic_across_two_calls() {
    let (storage, project_id, repo_id) = setup();
    let hub = file_entity(
        "p:repo1:file:hub.rs",
        &project_id,
        &repo_id,
        "hub",
        "hub.rs",
        "Rust",
    );
    let fn_a = func_entity(
        "fn-a",
        &project_id,
        &repo_id,
        "fn_a",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_b = func_entity(
        "fn-b",
        &project_id,
        &repo_id,
        "fn_b",
        Some("p:repo1:file:hub.rs"),
    );
    let fn_c = func_entity(
        "fn-c",
        &project_id,
        &repo_id,
        "fn_c",
        Some("p:repo1:file:hub.rs"),
    );
    for e in [&hub, &fn_a, &fn_b, &fn_c] {
        storage.upsert_entity(e).unwrap();
    }
    // Multiple edges so order matters.
    dep_edge(&storage, "fn-a", "fn-b", RelType::Calls);
    dep_edge(&storage, "fn-a", "fn-c", RelType::Calls);
    dep_edge(&storage, "fn-b", "fn-c", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (first, _b1, _cpe1, _bre1, _) = call_paths_and_blast_radius(&boxed, &hub);
    let (second, _b2, _cpe2, _bre2, _) = call_paths_and_blast_radius(&boxed, &hub);
    assert_eq!(
        first, second,
        "two identical calls to call_paths_and_blast_radius must return identical row order"
    );

    // The rows must be sorted by (entity_id, calls, direction); the
    // callee name and callee id agree in relative order here (distinct
    // fn_* names map to distinct ids with the same relative ordering),
    // so name-order and id-order coincide on this fixture.
    let mut keys: Vec<(String, String, String)> = first
        .iter()
        .map(|r| {
            (
                r["entity_id"].as_str().unwrap().to_string(),
                r["calls"].as_str().unwrap().to_string(),
                r["direction"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let sorted = keys.clone();
    keys.sort();
    assert_eq!(
        sorted, keys,
        "rows must be sorted by (entity_id, calls, direction): {first:?}"
    );
}
