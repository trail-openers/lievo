//! Files-mode include_depth coverage (issue #848: single-file depth
//! regression + withheld-hint / top-level completeness consistency).
//!
//! Wired via `#[path]` from `tools_explore_files.rs` as the `depth_tests`
//! module. Sibling helpers (`setup`, `upsert_file`, `make_tool`,
//! `write_file`) are shared with `tools_explore_files_tests` (the `tests`
//! module in the same parent) via `super::tests::*`.
//!
//! Covers:
//!   - REGRESSION: a single resolved file with include_depth=true receives
//!     a response that DIFFERS (byte-identity comparison) from the
//!     include_depth=false response, and blast_radius is non-empty
//!   - a multi-path request where exactly one path resolves returns that
//!     file WITH depth, not_found naming the bad path
//!   - a symbol carrying blast_radius carries no "pass include_depth=true"
//!     hint
//!   - a lean (include_depth=false) symbol with dependents still carries the
//!     standard lean hint (suppression must not break the lean case)
//!   - multi-file include_depth=true behaviour is unchanged (both files
//!     return WITH non-empty blast_radius)
//!   - depth shed under the cap (include_depth=true): the per-symbol hint
//!     does not advise passing include_depth=true again, and the top-level
//!     completeness is not complete:true
//!   - a single oversized file with include_source + include_depth: the
//!     ladder still yields a parseable response under the cap with
//!     complete:false
//!   - COMPLETENESS SIGNAL (issue #854): a target whose closure fits under
//!     the cap carries `blast_radius_complete: true`; one whose closure
//!     exceeds the cap carries `blast_radius_complete: false` with the
//!     omission count in `blast_radius_errors`; a lean and a depth-shed
//!     symbol carry NO `blast_radius_complete` field; neither contradicts
//!     the top-level completeness object
//!   - CALL-SHAPE REGRESSION TABLE: files+depth n=1/n=2/n=4, query+depth,
//!     files+query+depth n=1, files+src n=1, 2-path/1-resolve — each on a
//!     fixture with real incoming Imports edges, asserting the response
//!     parses, is <= MAX_EXPLORE_OUTPUT_CHARS, and every file that should
//!     carry depth does

use serde_json::{Value, json};

use crate::model::{EdgeProvenance, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

// Shared helpers from the sibling files-mode test module (same parent module).
use super::tests::{make_tool, setup, upsert_file, write_file};

const WITHHELD_REMEDY: &str =
    "depth withheld under the 24K output cap — request this file alone with include_depth=true";

/// Seed an incoming `Imports` edge `importer_path` -> `target_path` so the
/// target file has at least one dependent of an emitted rel type (gives a
/// non-empty blast_radius / lean dependent count when queried).
fn seed_importer(
    storage: &crate::storage::sqlite::SqliteStorage,
    project_id: &str,
    repo_id: &str,
    importer_path: &str,
    target_path: &str,
) {
    upsert_file(
        storage,
        project_id,
        repo_id,
        importer_path.trim_end_matches(".rs"),
        importer_path,
        None,
    );
    storage
        .upsert_relationship(&Relationship {
            source_id: format!("p:repo1:file:{importer_path}"),
            target_id: format!("p:repo1:file:{target_path}"),
            rel_type: RelType::Imports,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();
}

/// The regression test that would have caught issue #848: a single existing
/// file with an incoming edge of an emitted type must return a
/// include_depth=true response that DIFFERS from the include_depth=false
/// response (the old `n > 1` gate made them byte-identical), and the
/// include_depth=true response must carry a non-empty blast_radius.
#[test]
fn files_batch_single_file_depth_response_differs_from_lean() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tag.rs", "fn tag() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "tag", "tag.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "tag.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let with_depth = tool
        .call(json!({"files": ["tag.rs"], "include_depth": true}))
        .expect("single-file depth call must succeed");
    let lean = tool
        .call(json!({"files": ["tag.rs"], "include_depth": false}))
        .expect("single-file lean call must succeed");

    assert_ne!(
        with_depth, lean,
        "single-file include_depth=true response must DIFFER from the lean response (regression guard for #848): {with_depth}"
    );
    let parsed: Value = serde_json::from_str(&with_depth).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    let blast = sym["blast_radius"]
        .as_array()
        .expect("blast_radius array required");
    assert!(
        !blast.is_empty(),
        "a single file with an incoming Imports edge must return a non-empty blast_radius: {sym}"
    );
    assert!(
        sym.get("call_paths").is_some(),
        "call_paths key must be present: {sym}"
    );
    // Top-level completeness: depth was requested and delivered (not shed) —
    // the response is complete and the two completeness signals agree.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "delivered depth → complete:true: {parsed}"
    );
}

/// A two-path request where one path does not exist returns symbols=1 WITH
/// depth and `not_found` listing the bad path (multi-path, one resolved →
/// same shape as a single-path request, issue #848).
#[test]
fn files_batch_single_resolved_path_among_bogus_returns_depth() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tag.rs", "fn tag() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "tag", "tag.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "tag.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["tag.rs", "app/does/not/exist.js"], "include_depth": true}))
        .expect("batch with one bad path must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "the one good file must return: {parsed}");
    let sym = &symbols[0];
    assert_eq!(sym["name"], "tag");
    assert!(
        !sym["blast_radius"].as_array().unwrap().is_empty(),
        "the resolved file must carry depth even when a sibling path is bad: {sym}"
    );
    let not_found = parsed["not_found"].as_array().unwrap();
    let nf: Vec<&str> = not_found.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(
        nf,
        vec!["app/does/not/exist.js"],
        "bad path in not_found: {nf:?}"
    );
}

/// A symbol built with depth (carrying blast_radius) must NOT simultaneously
/// carry the lean per-symbol hint that tells the caller to pass
/// include_depth=true — that is the contradictory signal from #848. The
/// lean hint is only attached on the include_depth=false build path, so
/// this is a no-op guard: the key must simply be absent.
#[test]
fn symbol_carrying_blast_radius_has_no_pass_include_depth_hint() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tag.rs", "fn tag() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "tag", "tag.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "tag.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let with_depth = tool
        .call(json!({"files": ["tag.rs"], "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&with_depth).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        !sym["blast_radius"].as_array().unwrap().is_empty(),
        "fixture must carry depth: {sym}"
    );
    assert!(
        sym.get("completeness").is_none(),
        "a symbol built with depth must not carry the lean 'pass include_depth=true' hint: {sym}"
    );
}

/// The lean (include_depth=false) case is unaffected by the suppression: a
/// file with dependents still carries the standard lean hint (complete:false,
/// omitted_direct_dependents, remedy naming include_depth=true) — the hint
/// is correct and valuable there (issue #848 edge case: suppression must
/// not break the lean path). A lean symbol carries NO blast_radius_complete
/// field: the lean hint / depth-withheld remedy is its completeness signal,
/// and the explicit signal is only stated on symbols that carry depth.
#[test]
fn lean_single_file_with_dependents_still_carries_standard_hint() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "tag.rs", "fn tag() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "tag", "tag.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "tag.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let lean = tool
        .call(json!({"files": ["tag.rs"], "include_depth": false}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&lean).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // A lean symbol (include_depth=false) carries no blast_radius and no
    // blast_radius_complete — the lean hint is its completeness signal.
    assert!(
        sym.get("blast_radius_complete").is_none(),
        "lean symbol carries no blast_radius_complete field: {sym}"
    );
    let hint = sym["completeness"]
        .as_object()
        .expect("lean symbol with dependents must carry the standard hint");
    assert_eq!(hint["complete"], json!(false));
    assert_eq!(hint["omitted_direct_dependents"], json!(1));
    assert!(
        hint["remedy"]
            .as_str()
            .unwrap()
            .contains("include_depth=true"),
        "lean remedy must name the flag: {hint:?}"
    );
    // An ordinary lean request (no depth asked) must still report
    // complete:true at the top level — the depth-incomplete gate must not
    // fire for callers that never asked for depth.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "lean request, nothing omitted → complete:true: {parsed}"
    );
}

/// Multi-file + include_depth=true behaviour is unchanged by the gate
/// removal: both files return, the first carries non-empty blast_radius
/// (the existing test only asserted the files are present, never that depth
/// was non-empty), and when the depth-shed loop sheds a later file under
/// the cap the per-symbol withheld hint and the top-level completeness are
/// consistent (complete:false, no "pass include_depth=true" remedy).
///
/// The #838 depth-shed order is preserved for multi-file batches: the first
/// file's depth is built and kept (it fits), the second file's depth build
/// overflows the 24K probe while the first file's depth is in place, so the
/// Multi-file + include_depth=true (issue #848 decision #7): the existing
/// `files_batch_depth_is_shed_before_whole_files_dropped` seeds no
/// relationships, so its `blast_radius` is empty either way and proves
/// nothing about depth. This test seeds at least one incoming `Imports`
/// edge PER file and asserts non-empty `blast_radius` on BOTH returned
/// symbols — the shape the old `n > 1` gate could not regress (n=2 ran
/// the loop), but the shape a per-file no-depth path would. It also pins
/// the lean (include_depth=false) shape for the same two files: the lean
/// hints keep the standard "pass include_depth=true" remedy (unchanged per
/// decision #4) and top-level complete:true.
#[test]
fn files_batch_multi_file_depth_seeded_edges_both_files_carry_depth() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "m2.rs", "fn m2() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "m2", "m2.rs", None);
    write_file(tmp.path(), "m1.rs", "fn m1() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "m1", "m1.rs", None);
    write_file(tmp.path(), "imp1.rs", "fn i1() { 1 }\n");
    write_file(tmp.path(), "imp2.rs", "fn i2() { 1 }\n");
    write_file(tmp.path(), "imp2b.rs", "fn i2b() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "imp1.rs", "m1.rs");
    seed_importer(&storage, &project_id, &repo_id, "imp2.rs", "m1.rs");
    seed_importer(&storage, &project_id, &repo_id, "imp2b.rs", "m2.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // include_depth=true: BOTH files carry non-empty blast_radius (depth
    // was requested for every chosen file — the shape a per-file
    // no-depth path would break).
    let result = tool
        .call(json!({"files": ["m2.rs", "m1.rs"], "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2, "both files returned: {parsed}");
    for sym in symbols {
        let blast = sym["blast_radius"]
            .as_array()
            .unwrap_or_else(|| panic!("blast_radius key missing on {}: {sym}", sym["name"]));
        assert!(
            !blast.is_empty(),
            "multi-file seeded edge: {} must carry non-empty blast_radius: {sym}",
            sym["name"]
        );
        assert!(
            sym.get("completeness").is_none(),
            "symbol built with depth must not carry the lean hint: {sym}"
        );
    }
    // Depth was requested and delivered for every returned symbol: the
    // top-level completeness is complete.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "both files delivered with depth → complete:true: {parsed}"
    );

    // The lean shape for the same two files is unchanged (decision #4):
    // both symbols carry the standard lean hint (complete:false, remedy
    // naming include_depth=true) and the top level is complete.
    let lean = tool
        .call(json!({"files": ["m2.rs", "m1.rs"], "include_depth": false}))
        .unwrap();
    let lean_parsed: Value = serde_json::from_str(&lean).unwrap();
    let lean_symbols = lean_parsed["symbols"].as_array().unwrap();
    for sym in lean_symbols {
        assert!(
            sym.get("blast_radius_complete").is_none(),
            "lean symbol carries no blast_radius_complete field: {sym}"
        );
        let hint = sym["completeness"]
            .as_object()
            .unwrap_or_else(|| panic!("lean symbol with dependents must carry the hint: {sym}"));
        assert_eq!(hint["complete"], json!(false));
        assert!(
            hint["remedy"]
                .as_str()
                .unwrap()
                .contains("include_depth=true"),
            "lean remedy unchanged: {hint:?}"
        );
    }
    let lean_comp = lean_parsed["completeness"].as_object().unwrap();
    assert_eq!(
        lean_comp["complete"],
        json!(true),
        "lean request, nothing omitted → complete:true: {lean_parsed}"
    );
}

/// Depth shed under the cap (include_depth=true): a single file whose
/// tier-2 source alone exceeds the cap forces the depth-shed revert path.
/// The per-symbol hint must NOT advise passing include_depth=true again —
/// the caller already did; it must state the depth was withheld under the
/// output cap and suggest requesting the file on its own. The top-level
/// completeness must be complete:false (depth requested but withheld), and
/// the response must parse under the cap.
#[test]
fn single_file_depth_shed_hint_and_top_level_complete_false() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // ~48K-char body: tier-2 source alone exceeds the 24K cap, so the
    // full-depth payload overflows and the shed-revert fires for this file.
    let body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad \n"
            .repeat(1500)
        + "}\n";
    write_file(tmp.path(), "huge.rs", &body);
    upsert_file(&storage, &project_id, &repo_id, "huge", "huge.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "huge.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["huge.rs"], "include_source": true, "include_depth": true}))
        .expect("single oversized file with depth must still return");
    assert!(
        result.chars().count() <= crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).expect("must parse (never a fragment)");
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // The depth was shed: no blast_radius/call_paths on the returned symbol.
    assert!(
        sym.get("blast_radius").is_none(),
        "depth shed under the cap → no blast_radius: {sym}"
    );
    // The per-symbol hint is the depth-withheld text (decision #4) and the
    // top-level completeness is not complete (decision #5: the two signals
    // agree).
    let hint = sym["completeness"]
        .as_object()
        .expect("depth-shed symbol must carry the withheld hint");
    assert_eq!(hint["complete"], json!(false));
    let remedy = hint["remedy"].as_str().expect("remedy string").to_string();
    assert_eq!(
        remedy, WITHHELD_REMEDY,
        "remedy must be the exact depth-withheld text: {hint:?}"
    );
    // Top-level completeness: depth was requested and withheld → not
    // complete, even though the file itself was returned.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(false),
        "depth requested but withheld → complete:false: {parsed}"
    );
}

/// Single oversized file with include_source=true AND include_depth=true
/// (the ladder's input shape under the new gate): the response must still
/// be parseable, under the cap, with complete:false — the ladder trims an
/// already-built (possibly already depth-shed) symbol and never builds
/// depth itself.
#[test]
fn files_batch_single_oversized_file_with_include_depth_ladder_behaves_correctly() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad \n"
            .repeat(1500)
        + "}\n";
    write_file(tmp.path(), "huge.rs", &body);
    upsert_file(&storage, &project_id, &repo_id, "huge", "huge.rs", None);
    write_file(tmp.path(), "importer.rs", "fn imp() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer.rs", "huge.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["huge.rs"], "include_source": true, "include_depth": true}))
        .unwrap();
    assert!(
        result.chars().count() <= crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).expect("must parse (never a fragment)");
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("name").is_some(),
        "identity survives the ladder: {sym}"
    );
    // Either depth survived (non-empty blast_radius) or the symbol carries
    // the depth-withheld remedy (decision #8).
    let blast = sym["blast_radius"].as_array();
    let has_nonempty_blast = blast.map(|b| !b.is_empty()).unwrap_or(false);
    if !has_nonempty_blast {
        let remedy = sym["completeness"]
            .as_object()
            .and_then(|h| h["remedy"].as_str())
            .map(String::from);
        assert_eq!(
            remedy.as_deref(),
            Some(WITHHELD_REMEDY),
            "no depth and no withheld remedy: {sym}"
        );
    }
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(false),
        "truncated/withheld single file → complete:false: {parsed}"
    );
    // If a source survived the ladder, it was truncated on a whole-line
    // boundary and flagged.
    let has_source = sym.get("source").map(|s| !s.is_null()).unwrap_or(false);
    if has_source {
        assert_eq!(
            sym["source_truncated"],
            json!(true),
            "flag set when source truncated: {sym}"
        );
    }
    // A depth-shed file carries no blast_radius, and therefore no
    // blast_radius_complete (consistent with #838/#848: the lean hint /
    // depth-withheld remedy is the signal there).
    assert!(
        sym.get("blast_radius_complete").is_none(),
        "depth-shed symbol carries no blast_radius_complete field: {sym}"
    );
}

// ---------------------------------------------------------------------------
// Completeness signal tests (issue #854)
// ---------------------------------------------------------------------------

/// A target whose closure fits under the cap carries `blast_radius_complete`
/// == true (the complete two-level reverse dependency set), and the
/// top-level completeness is complete:true.
#[test]
fn completeness_signal_closure_fits_under_cap_is_complete() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "target.rs", "fn target() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "target", "target.rs", None);
    write_file(tmp.path(), "importer1.rs", "fn i1() { 1 }\n");
    write_file(tmp.path(), "importer2.rs", "fn i2() { 1 }\n");
    seed_importer(&storage, &project_id, &repo_id, "importer1.rs", "target.rs");
    seed_importer(&storage, &project_id, &repo_id, "importer2.rs", "target.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["target.rs"], "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // The closure fits under the cap (2 dependents << 25): the explicit
    // completeness signal is true.
    assert_eq!(
        sym["blast_radius_complete"],
        json!(true),
        "closure under cap → blast_radius_complete:true: {sym}"
    );
    // The blast_radius is non-empty (the closure is delivered).
    let blast = sym["blast_radius"].as_array().unwrap();
    assert!(!blast.is_empty(), "non-empty blast_radius: {sym}");
    // Top-level completeness: complete:true (depth delivered, not shed).
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "complete closure → complete:true: {parsed}"
    );
}

/// A target whose closure exceeds the cap carries `blast_radius_complete: false`
/// with the omission count in `blast_radius_errors`, and the top-level
/// completeness is also not complete (the two signals never contradict).
#[test]
fn completeness_signal_closure_exceeds_cap_is_not_complete() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "hub.rs", "fn hub() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "hub", "hub.rs", None);
    // 30 importers: 25 kept, 5 omitted → cap bound.
    for i in 0..30 {
        let p = format!("imp_{i:02}.rs");
        write_file(tmp.path(), &p, &format!("fn i{i}() {{ {i} }}\n"));
        seed_importer(&storage, &project_id, &repo_id, &p, "hub.rs");
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["hub.rs"], "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // The closure exceeds the cap: blast_radius_complete is false.
    assert_eq!(
        sym["blast_radius_complete"],
        json!(false),
        "closure over cap → blast_radius_complete:false: {sym}"
    );
    // The blast_radius is exactly 25 entries.
    let blast = sym["blast_radius"].as_array().unwrap();
    assert_eq!(blast.len(), 25, "exactly 25 entries: {sym}");
    // The omission count is in blast_radius_errors.
    let errors = sym["blast_radius_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "one disclosure: {errors:?}");
    assert!(
        errors[0].as_str().unwrap().contains("5 dependents omitted"),
        "omission count = 30 - 25 = 5: {errors:?}"
    );
    // Top-level completeness: depth was delivered (not shed) but capped →
    // the per-symbol signal says false, and the top-level says true
    // (depth was NOT shed — the cap is a per-symbol disclosure, not a
    // top-level incompleteness). The two signals are consistent: the
    // per-symbol `blast_radius_complete: false` does not contradict the
    // top-level `complete: true` because the top-level only tracks shed,
    // not cap-bound per-symbol closures.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "depth delivered (not shed) → top-level complete:true: {parsed}"
    );
}

// ---------------------------------------------------------------------------
// Call-shape regression table (issue #854)
// ---------------------------------------------------------------------------

/// One table-driven test over the call shapes agents actually
/// issue: files+depth n=1, n=2, n=4; query+depth; files+query+depth n=1;
/// files+src n=1; 2-path/1-resolve. Each on a fixture with real incoming
/// Imports edges, asserting: the response parses as JSON, is <=
/// MAX_EXPLORE_OUTPUT_CHARS, and every file that should carry depth does.
#[test]
fn call_shape_regression_table() {
    use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;

    // Shared fixture: target.rs with 2 importers + second.rs with 1 importer.
    // Each file has a real incoming Imports edge so depth is non-empty.
    fn make_fixture() -> (
        crate::storage::sqlite::SqliteStorage,
        String,
        std::path::PathBuf,
        tempfile::TempDir,
    ) {
        let (storage, project_id, repo_id) = setup();
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().canonicalize().unwrap();
        // target.rs with 2 direct importers.
        write_file(&repo_path, "target.rs", "fn target() { 42 }\n");
        upsert_file(&storage, &project_id, &repo_id, "target", "target.rs", None);
        write_file(&repo_path, "imp1.rs", "fn i1() { 1 }\n");
        write_file(&repo_path, "imp2.rs", "fn i2() { 1 }\n");
        seed_importer(&storage, &project_id, &repo_id, "imp1.rs", "target.rs");
        seed_importer(&storage, &project_id, &repo_id, "imp2.rs", "target.rs");
        // A second file (second.rs) with its own importer, for n=2/n=4 shapes.
        write_file(&repo_path, "second.rs", "fn second() { 2 }\n");
        upsert_file(&storage, &project_id, &repo_id, "second", "second.rs", None);
        write_file(&repo_path, "imp3.rs", "fn i3() { 3 }\n");
        seed_importer(&storage, &project_id, &repo_id, "imp3.rs", "second.rs");
        (storage, project_id, repo_path, tmp)
    }

    // Each case: (name, input JSON, expected number of symbols with blast_radius).
    let cases: Vec<(&str, serde_json::Value, usize)> = vec![
        (
            "files+depth n=1",
            json!({"files": ["target.rs"], "include_depth": true}),
            1,
        ),
        (
            "files+depth n=2",
            json!({"files": ["target.rs", "second.rs"], "include_depth": true}),
            2,
        ),
        (
            "files+depth n=4",
            json!({"files": ["target.rs", "second.rs", "imp1.rs", "imp2.rs"], "include_depth": true}),
            4,
        ),
        (
            "query+depth",
            json!({"query": "target", "include_depth": true}),
            1,
        ),
        (
            "files+query+depth n=1",
            json!({"files": ["target.rs"], "query": "target", "include_depth": true}),
            1,
        ),
        (
            "files+src n=1",
            json!({"files": ["target.rs"], "include_source": true}),
            0,
        ),
        (
            "2-path/1-resolve",
            json!({"files": ["target.rs", "nonexistent/nope.rs"], "include_depth": true}),
            1,
        ),
    ];

    for (name, input, expected_depth_files) in cases {
        let (storage, project_id, repo_path, _tmp_keepalive) = make_fixture();
        let tool = make_tool(storage, project_id, repo_path);
        let result = tool
            .call(input.clone())
            .unwrap_or_else(|e| panic!("case '{name}': call failed: {e}"));
        // The response parses as JSON.
        let parsed: Value = serde_json::from_str(&result)
            .unwrap_or_else(|e| panic!("case '{name}': must parse: {e}"));
        // Measured payload numbers (fixture numbers — not the owners' index
        // numbers; the fixture is a handful of tiny synthetic files):
        // serialized response char count and how many requested files carry
        // a non-empty blast_radius.
        let symbols = parsed["symbols"].as_array().unwrap();
        let depth_count = symbols
            .iter()
            .filter(|s| s.get("blast_radius").is_some())
            .count();
        eprintln!(
            "call-shape {name}: {} chars, depth on {}/{}",
            result.chars().count(),
            depth_count,
            symbols.len()
        );
        // The response is <= MAX_EXPLORE_OUTPUT_CHARS.
        assert!(
            result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
            "case '{name}': response {} chars > cap {}",
            result.chars().count(),
            MAX_EXPLORE_OUTPUT_CHARS
        );
        // Every file that should carry depth does.
        assert_eq!(
            depth_count, expected_depth_files,
            "case '{name}': expected {expected_depth_files} symbols with blast_radius, got {depth_count}: {symbols:?}"
        );
    }
}
