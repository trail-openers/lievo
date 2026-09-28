// Issue #834: emission of blast_radius is gated on the EMIT set (Imports,
// DependsOn, Implements) rather than the full dependency set, while
// traversal still walks Calls edges. These tests pin the three invariants
// that the slimmed payload must preserve:
//
//   1. A file reached through a CALLS-only hop-1 edge is not itself emitted,
//      but its hop-2 importer (reached via an Imports edge) still is — the
//      calls edge still drives traversal, it just stops labelling entries.
//   2. No blast_radius entry ever carries "calls" in its rel_types, even when
//      a file is reached through BOTH an Imports and a Calls edge (the merge
//      site, not just new-entry creation, is gated).
//   3. Blast entries and per-symbol payloads omit the derivable entity_id,
//      while call_paths rows RETAIN their (non-derivable Function-tier)
//      entity_id.
//
// Wired via `#[path]` from tools_explore_blast.rs (sibling of
// tools_explore_blast_tests.rs). Reuses the shared SqliteBox + fixture
// helpers from that module.

use crate::model::RelType;
use crate::retrieval::tools_explore_blast::call_paths_and_blast_radius;
use crate::retrieval::tools_explore_blast::tools_explore_blast_tests::{
    SqliteBox, dep_edge, file_entity, func_entity, setup,
};
use crate::storage::Storage;
use serde_json::Value;

#[test]
fn calls_only_hop1_source_not_emitted_but_hop2_importer_is() {
    // target (T) is called by mid (M) via a CALLS edge at hop 1. M has no
    // import/depends_on/implements edge to T, so M itself must NOT appear in
    // T's blast_radius. But top (P) imports M, so at hop 2 P IS emitted.
    // This proves the calls edge still drives traversal (M enters the
    // frontier) even though it no longer creates an entry.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let m = file_entity(
        "p:repo1:file:m.js",
        &project_id,
        &repo_id,
        "m",
        "m.js",
        "JavaScript",
    );
    let p = file_entity(
        "p:repo1:file:p.js",
        &project_id,
        &repo_id,
        "p",
        "p.js",
        "JavaScript",
    );
    for e in [&t, &m, &p] {
        storage.upsert_entity(e).unwrap();
    }
    // M calls T (a calls edge, hop 1 from T's perspective).
    dep_edge(
        &storage,
        "p:repo1:file:m.js",
        "p:repo1:file:t.js",
        RelType::Calls,
    );
    // P imports M (an import edge, hop 2 from T's perspective).
    dep_edge(
        &storage,
        "p:repo1:file:p.js",
        "p:repo1:file:m.js",
        RelType::Imports,
    );

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);

    // M (calls-only hop-1 source) is NOT emitted.
    assert!(
        !blast.iter().any(|e| e["name"] == "m"),
        "a calls-only hop-1 source must not be emitted (issue #834): {blast:?}"
    );
    // P (hop-2 importer of M) IS emitted — the calls edge still drove the
    // traversal so M entered the frontier and P was reached at hop 2.
    let p_entry = blast
        .iter()
        .find(|e| e["name"] == "p")
        .unwrap_or_else(|| panic!("hop-2 importer p must be in blast_radius: {blast:?}"));
    assert_eq!(
        p_entry["rel_types"],
        Value::from(vec![Value::from("imports")]),
        "hop-2 importer p is labelled with the import edge: {p_entry:?}"
    );
}

#[test]
fn rel_types_never_contain_calls_even_when_merging_imports_and_calls() {
    // A file reached through BOTH an Imports and a Calls edge (the merge
    // site): the entry must be created (via imports) but the merge must NOT
    // append "calls". This is the exact merge-site guard the issue calls out
    // — a file reached first via an import then via a call must not get
    // "calls" appended to its rel_types.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let i = file_entity(
        "p:repo1:file:i.js",
        &project_id,
        &repo_id,
        "i",
        "i.js",
        "JavaScript",
    );
    let fn_i = func_entity(
        "fn-i",
        &project_id,
        &repo_id,
        "fn_i",
        Some("p:repo1:file:i.js"),
    );
    let fn_t = func_entity(
        "fn-t",
        &project_id,
        &repo_id,
        "fn_t",
        Some("p:repo1:file:t.js"),
    );
    for e in [&t, &i, &fn_i, &fn_t] {
        storage.upsert_entity(e).unwrap();
    }
    // Same file reached through two edge types: import (file->file) + call
    // (function->function).
    dep_edge(
        &storage,
        "p:repo1:file:i.js",
        "p:repo1:file:t.js",
        RelType::Imports,
    );
    dep_edge(&storage, "fn-i", "fn-t", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);

    assert_eq!(blast.len(), 1, "one merged entry: {blast:?}");
    let entry = &blast[0];
    let types = entry["rel_types"].as_array().unwrap();
    assert!(
        !types.iter().any(|t| t == "calls"),
        "calls must never appear in rel_types, even at the merge site (issue #834): {entry:?}"
    );
    assert!(
        types.iter().any(|t| t == "imports"),
        "the import edge must still be labelled: {entry:?}"
    );
}

#[test]
fn blast_entries_and_symbols_omit_entity_id_but_call_paths_retain_it() {
    // A file with an import edge AND a calls edge to a function in the
    // target. The blast entry must omit entity_id; the per-symbol payload
    // must omit entity_id (the symbol IS the file, identified by name/path);
    // the call_paths rows must RETAIN their Function-tier entity_id.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let i = file_entity(
        "p:repo1:file:i.js",
        &project_id,
        &repo_id,
        "i",
        "i.js",
        "JavaScript",
    );
    let fn_i = func_entity(
        "fn-i",
        &project_id,
        &repo_id,
        "fn_i",
        Some("p:repo1:file:i.js"),
    );
    let fn_t = func_entity(
        "fn-t",
        &project_id,
        &repo_id,
        "fn_t",
        Some("p:repo1:file:t.js"),
    );
    for e in [&t, &i, &fn_i, &fn_t] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(
        &storage,
        "p:repo1:file:i.js",
        "p:repo1:file:t.js",
        RelType::Imports,
    );
    dep_edge(&storage, "fn-i", "fn-t", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);

    // Every blast entry omits the derivable entity_id.
    assert!(
        !blast.is_empty(),
        "expected at least one blast entry: {blast:?}"
    );
    for entry in &blast {
        assert!(
            entry.get("entity_id").is_none(),
            "blast entry must not carry entity_id (issue #834): {entry:?}"
        );
        // ...but still carries the identity the caller needs.
        assert!(entry.get("name").is_some(), "name present: {entry:?}");
        assert!(entry.get("path").is_some(), "path present: {entry:?}");
        assert!(entry.get("tier").is_some(), "tier present: {entry:?}");
        assert!(
            entry.get("direction").is_some(),
            "direction present: {entry:?}"
        );
        assert!(
            entry.get("rel_types").is_some(),
            "rel_types present: {entry:?}"
        );
    }

    // The call is an incoming Calls edge on fn_t, so call_paths is non-empty
    // and each row retains its (non-derivable) Function-tier entity_id.
    assert!(
        !call_paths.is_empty(),
        "call_paths must be non-empty: {call_paths:?}"
    );
    for cp in &call_paths {
        assert!(
            cp.get("entity_id").is_some(),
            "call_paths row must retain entity_id: {cp:?}"
        );
    }
}
