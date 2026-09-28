// Unit tests for the wrong-edge population split (issue #777).
//
// The `classify_edges` / gate unit tests below are pure. The
// `run_pure_sections_*` tests at the bottom are INTEGRATION tests over the
// real extraction pipeline (extract → group → RelationshipBuilder →
// classify → verify → gate), moved here from `selfcheck_ops_tests.rs` to
// stay within the 800-line test budget (AGENTS.md §6). They use the shared
// `write_repo` helper and `sample_size_from_detail` from the ops tests.

use super::{classify_edges, count_evidenced, gate_edge_correctness};
use crate::commands::project::selfcheck_false_zero::SelfcheckThresholds;
use crate::commands::project::selfcheck_ops::run_pure_sections;
use crate::commands::project::selfcheck_ops::selfcheck_ops_tests::{
    call_base_count_from_detail, evidenced_from_detail, section_sample_size, write_repo,
};
use lievo::model::{EdgeProvenance, RelType, Relationship};

fn imports(src: &str, tgt: &str, provenance: EdgeProvenance) -> Relationship {
    Relationship {
        source_id: src.to_string(),
        target_id: tgt.to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance,
    }
}

fn calls(src: &str, tgt: &str, provenance: EdgeProvenance) -> Relationship {
    Relationship {
        source_id: src.to_string(),
        target_id: tgt.to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance,
    }
}

/// Dual-path edge (present in both lists) lands in import_resolved exactly
/// once and never in call_based (the resolved-wins rule, mirroring
/// merge_provenance).
#[test]
fn dual_path_edge_is_import_resolved_exactly_once() {
    let id_to_path = std::collections::HashMap::from([
        ("a".to_string(), "src/a.rs".to_string()),
        ("b".to_string(), "src/b.rs".to_string()),
    ]);
    let rels = vec![
        imports("a", "b", EdgeProvenance::Resolved),
        imports("a", "b", EdgeProvenance::Heuristic),
    ];
    let pop = classify_edges(&rels, &id_to_path);
    assert_eq!(
        pop.import_resolved,
        vec![("src/a.rs".to_string(), "src/b.rs".to_string())]
    );
    assert!(
        pop.call_based.is_empty(),
        "dual-path edge must not be double-counted: {:?}",
        pop.call_based
    );
    assert_eq!(pop.import_resolved.len(), 1);
    assert_eq!(pop.call_based_census(), 0);
    // The union (false-zero section input) holds the pair exactly once.
    assert_eq!(
        pop.imported,
        vec![("src/a.rs".to_string(), "src/b.rs".to_string())]
    );
}

/// A call-based edge not covered by any Resolved edge is classified as
/// call-based and stays out of the import-resolved population — but it IS
/// in the union `imported` set (the false_zero_callers input must be
/// unchanged by the split: a call-based edge counts as an import edge, just
/// not a scored one — issue #777 regression criterion).
#[test]
fn call_based_edge_is_separate_category() {
    let id_to_path = std::collections::HashMap::from([
        ("a".to_string(), "src/a.rs".to_string()),
        ("b".to_string(), "src/b.rs".to_string()),
    ]);
    let rels = vec![imports("a", "b", EdgeProvenance::Heuristic)];
    let pop = classify_edges(&rels, &id_to_path);
    assert!(pop.import_resolved.is_empty());
    assert_eq!(
        pop.call_based,
        vec![("src/a.rs".to_string(), "src/b.rs".to_string())]
    );
    assert_eq!(
        pop.imported,
        vec![("src/a.rs".to_string(), "src/b.rs".to_string())],
        "call-based edges must stay in the false-zero imported set"
    );
}

/// Mixed provenances: the union set is the dedup'd combination of both
/// classes (dual-path pairs appear once).
#[test]
fn imported_union_covers_both_classes_deduped() {
    let id_to_path = std::collections::HashMap::from([
        ("a".to_string(), "src/a.rs".to_string()),
        ("b".to_string(), "src/b.rs".to_string()),
        ("c".to_string(), "src/c.rs".to_string()),
    ]);
    let rels = vec![
        imports("a", "b", EdgeProvenance::Resolved),
        imports("a", "b", EdgeProvenance::Heuristic), // dual-path: same pair
        imports("b", "c", EdgeProvenance::Heuristic),
    ];
    let pop = classify_edges(&rels, &id_to_path);
    assert_eq!(pop.import_resolved.len(), 1);
    assert_eq!(pop.call_based.len(), 1);
    assert_eq!(
        pop.imported,
        vec![
            ("src/a.rs".to_string(), "src/b.rs".to_string()),
            ("src/b.rs".to_string(), "src/c.rs".to_string()),
        ]
    );
}

/// Non-Imports relationships never enter the population.
#[test]
fn non_imports_relationships_are_excluded() {
    let id_to_path = std::collections::HashMap::from([
        ("a".to_string(), "src/a.rs".to_string()),
        ("b".to_string(), "src/b.rs".to_string()),
    ]);
    let rels = vec![calls("a", "b", EdgeProvenance::Resolved)];
    let pop = classify_edges(&rels, &id_to_path);
    assert!(pop.import_resolved.is_empty());
    assert!(pop.call_based.is_empty());
}

/// Edges whose endpoint lacks a path mapping are dropped (as before the
/// split).
#[test]
fn edges_without_path_mapping_are_dropped() {
    let id_to_path = std::collections::HashMap::from([("a".to_string(), "src/a.rs".to_string())]);
    let rels = vec![imports("a", "missing", EdgeProvenance::Resolved)];
    let pop = classify_edges(&rels, &id_to_path);
    assert!(pop.import_resolved.is_empty());
    assert!(pop.call_based.is_empty());
}

/// Duplicate (src, tgt) pairs within a single provenance class are dedup'd
/// (same shape sample_edges produces).
#[test]
fn duplicate_pairs_are_deduped() {
    let id_to_path = std::collections::HashMap::from([
        ("a".to_string(), "src/a.rs".to_string()),
        ("b".to_string(), "src/b.rs".to_string()),
    ]);
    let rels = vec![
        imports("a", "b", EdgeProvenance::Resolved),
        imports("a", "b", EdgeProvenance::Resolved),
    ];
    let pop = classify_edges(&rels, &id_to_path);
    assert_eq!(pop.import_resolved.len(), 1);
}

fn sample(resolved: bool, wrong: bool) -> super::super::selfcheck_metrics::EdgeSample {
    super::super::selfcheck_metrics::EdgeSample {
        source_file: "src/a.rs".to_string(),
        recorded_target: "src/b.rs".to_string(),
        resolved_target: resolved.then(|| "src/b.rs".to_string()),
        wrong,
    }
}

#[test]
fn count_evidenced_counts_only_independently_resolved_samples() {
    let samples = vec![
        sample(true, false),
        sample(false, false),
        sample(true, true),
    ];
    assert_eq!(count_evidenced(&samples), 2);
    assert_eq!(count_evidenced(&[]), 0);
}

#[test]
fn gate_detail_names_all_three_numbers() {
    let report = gate_edge_correctness(0.1, 10, 10, 25, &SelfcheckThresholds::default());
    assert_eq!(report.name, "edge_correctness");
    assert!(!report.passed, "0.1 > default 0.0 threshold must fail");
    // All three numbers named explicitly (operator decision #2).
    assert!(
        report
            .detail
            .contains("over 10 evidenced import-resolved edges"),
        "evidenced import-resolved count missing: {}",
        report.detail
    );
    assert!(
        report.detail.contains("sample_size=10"),
        "sample_size must report the evidenced import-resolved count: {}",
        report.detail
    );
    assert!(
        report.detail.contains("call_based=25"),
        "call-based census missing: {}",
        report.detail
    );
    assert!(
        report
            .detail
            .contains("unverifiable-by-construction, not scored"),
        "call-based category must be marked unverifiable: {}",
        report.detail
    );
    assert!(
        report.detail.starts_with("wrong_edge_rate=0.1000"),
        "rate missing: {}",
        report.detail
    );
}

#[test]
fn gate_passes_at_or_below_threshold() {
    let at = SelfcheckThresholds {
        max_wrong_edge_rate: 0.1,
        ..Default::default()
    };
    assert!(
        gate_edge_correctness(0.1, 1, 1, 0, &at).passed,
        "equality passes"
    );
    assert!(gate_edge_correctness(0.05, 1, 1, 0, &at).passed);
    let above = SelfcheckThresholds {
        max_wrong_edge_rate: 0.05,
        ..Default::default()
    };
    assert!(!gate_edge_correctness(0.1, 1, 1, 0, &above).passed);
}

// ---------------------------------------------------------------------------
// Integration tests (moved from selfcheck_ops_tests.rs, issue #777).
//
// These exercise the full pure-section pipeline against a synthetic repo,
// including the required dual-path test. They live here (not in
// selfcheck_ops_tests.rs) to stay within the 800-line test budget.
// ---------------------------------------------------------------------------

/// Clean repo: the single use-statement edge is the import-resolved
/// population; the call-based census is 0 (reported, never scored, but must
/// still be named — issue #777).
#[test]
fn run_pure_sections_passes_on_clean_repo() {
    let dir = write_repo(&[
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "use crate::b;\npub fn a_fn() {}\n"),
        ("src/b.rs", "pub fn b_fn() {}\n"),
    ]);
    let thresholds = SelfcheckThresholds::default();
    let sections = run_pure_sections(dir.path(), &thresholds).expect("run_pure_sections");
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].name, "edge_correctness");
    assert_eq!(sections[1].name, "false_zero_callers");
    assert_eq!(sections[2].name, "super_structural_probe");

    // The edge section must not just "pass vacuously": the independent
    // resolver must actually have evidence. The detail's `sample_size=N`
    // reports the import-resolved population (issue #777) — a 0.0 rate with
    // zero evidence is the silent no-evidence path the section exists to
    // catch.
    let edge = &sections[0];
    assert!(edge.passed, "clean repo must pass: {}", edge.detail);
    assert_eq!(
        section_sample_size(edge),
        Some(1),
        "expected the 1-edge import-resolved census, got: {}",
        edge.detail
    );
    // The single edge is use-statement-resolved — nothing call-based exists
    // in this fixture, so the call-based census must be 0 (it is reported,
    // never scored, but must still be named — issue #777).
    assert_eq!(
        call_base_count_from_detail(&edge.detail),
        Some(0),
        "got: {}",
        edge.detail
    );
    assert!(
        edge.detail.starts_with("wrong_edge_rate=0.0000"),
        "expected zero wrong edges, got: {}",
        edge.detail
    );
}

/// Issue #777: a call-based edge (a public fn in one file called from
/// another, NO use-statement to that file) is counted in the call-based
/// census and does NOT contribute to wrong_edge_rate or the import-resolved
/// population. This is the exact shape of the 5 feature/issue-776 wrong
/// edges (files calling public fns from src/config.rs without an import).
#[test]
fn run_pure_sections_call_based_edge_is_unverifiable_not_wrong() {
    let dir = write_repo(&[
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn helper() -> i32 { 1 }\n"),
        // NO `use crate::a;` — the call below produces a Heuristic edge only
        // (bare-name fn_map path — the issue-776 shape: a public fn called
        // from another file with no import).
        ("src/b.rs", "pub fn caller() -> i32 { helper() }\n"),
    ]);
    let thresholds = SelfcheckThresholds::default();
    let sections = run_pure_sections(dir.path(), &thresholds).expect("run_pure_sections");
    let edge = sections
        .iter()
        .find(|s| s.name == "edge_correctness")
        .expect("edge_correctness section");
    // The call-based edge must be visible in the census (reported, never
    // silently dropped) ...
    assert_eq!(
        call_base_count_from_detail(&edge.detail),
        Some(1),
        "call-based census must name the 1 call-based edge, got: {}",
        edge.detail
    );
    // ... and it must be OUT of the import-resolved population (the
    // sample_size= field) and out of the rate's denominator (over N).
    assert_eq!(
        section_sample_size(edge),
        Some(0),
        "call-based edge must not be in the import-resolved population, got: {}",
        edge.detail
    );
    assert_eq!(
        evidenced_from_detail(&edge.detail),
        Some(0),
        "got: {}",
        edge.detail
    );
    assert!(
        edge.detail.starts_with("wrong_edge_rate=0.0000"),
        "call-based edges must not be scored wrong, got: {}",
        edge.detail
    );
    assert!(edge.passed, "got: {}", edge.detail);
}

/// Issue #777 regression criterion: the false_zero_callers section must
/// report IDENTICAL numbers before and after the edge-split. A file imported
/// ONLY via a call-based (Heuristic) edge was "covered" by the pre-split
/// resolved-by-path list and must stay covered after the split — it must not
/// become a false-zero caller (the failure mode of the round-1 commit: a
/// file reachable only via its `mod x;` declaration's heuristic edge was
/// flagged because only the import-resolved list was passed to the section).
/// Issue #777 regression criterion: the false_zero_callers section must
/// report IDENTICAL numbers before and after the edge-split. The fixture
/// shape mirrors the e2e sample_repo case: `src/a.rs` is the target of a
/// call-based (Heuristic) edge ONLY — it is called from `src/b.rs` (bare-
/// name fn_map / call path) but no file `use`s it. Pre-split, that edge
/// covered `src/a.rs` in the false-zero section's imported set; post-split
/// it must stay covered (the `imported` union), so `src/a.rs` must not
/// become a false-zero caller.
#[test]
fn run_pure_sections_call_based_import_stays_covered_by_false_zero_section() {
    let dir = write_repo(&[
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        // Defines the public fn that b.rs calls — the (b -> a) edge is
        // Heuristic-only (no use-statement to `a` anywhere).
        ("src/a.rs", "pub fn deep_label() -> &str { \"x\" }\n"),
        // NO `use crate::a;` — the call below produces a Heuristic edge
        // (src/b.rs -> src/a.rs) only, covering `a` in the imported set.
        ("src/b.rs", "pub fn caller() -> &str { deep_label() }\n"),
    ]);
    let thresholds = SelfcheckThresholds::default();
    let sections = run_pure_sections(dir.path(), &thresholds).expect("run_pure_sections");
    let fz = sections
        .iter()
        .find(|s| s.name == "false_zero_callers")
        .expect("false_zero_callers section");
    // src/a.rs (the call-based edge's target) must not be flagged — a
    // false-zero on "a" would mean the heuristic edge stopped covering it
    // after the split. (src/b.rs legitimately flags in both pre- and
    // post-split code: nothing covers it, and its stem appears in a.rs's
    // declaration — identical behavior, not a regression.)
    assert!(
        !fz.detail.contains("src/a.rs"),
        "a call-based-only import edge must keep covering its target file in the false-zero section, got: {}",
        fz.detail
    );
    let edge = sections
        .iter()
        .find(|s| s.name == "edge_correctness")
        .expect("edge section");
    // The same call-based edge must be in the call-based census (reported,
    // never scored) and NOT in the import-resolved population.
    assert_eq!(call_base_count_from_detail(&edge.detail), Some(1));
    assert_eq!(section_sample_size(edge), Some(0));
    assert!(edge.passed, "got: {}", edge.detail);
}

/// Issue #777 integration test (the required dual-path test): a file that
/// BOTH `use`s another module AND calls a function in it produces a SINGLE
/// (src, tgt) edge, counted exactly once in the import-resolved population
/// (resolved-wins, mirroring merge_provenance) and zero times in the
/// call-based census.
#[test]
fn run_pure_sections_dual_path_edge_counted_once_as_import_resolved() {
    let dir = write_repo(&[
        ("src/lib.rs", "pub mod a;\npub mod b;\n"),
        ("src/a.rs", "pub fn fn_a() -> i32 { 1 }\n"),
        // BOTH a use-statement to a ... and a call to fn_a in the same file:
        // the production resolver emits the (b, a) pair from both paths and
        // merge_provenance keeps Resolved — the split must count it once,
        // as import-resolved, and never as call-based.
        (
            "src/b.rs",
            "use crate::a::fn_a;\npub fn use_fn_a() -> i32 { fn_a() }\n",
        ),
    ]);
    let thresholds = SelfcheckThresholds::default();
    let sections = run_pure_sections(dir.path(), &thresholds).expect("run_pure_sections");
    let edge = sections
        .iter()
        .find(|s| s.name == "edge_correctness")
        .expect("edge_correctness section");
    assert!(
        edge.passed,
        "dual-path healthy edge must pass: {}",
        edge.detail
    );
    // Exactly ONE import-resolved edge (the dedup'd (b, a) pair) — the dual
    // path must not double-count it.
    assert_eq!(
        section_sample_size(edge),
        Some(1),
        "dual-path edge must be counted exactly once as import-resolved, got: {}",
        edge.detail
    );
    // ... and ZERO times in the call-based census.
    assert_eq!(
        call_base_count_from_detail(&edge.detail),
        Some(0),
        "dual-path edge must not appear in the call-based census, got: {}",
        edge.detail
    );
}

#[cfg(test)]
mod sample_size_detail_tests {
    use crate::commands::project::selfcheck_ops::selfcheck_ops_tests::sample_size_from_detail;

    #[test]
    fn sample_size_detail_parsing_roundtrips_census_counts() {
        // Census detail strings: sample_size is the full verified edge count.
        let detail = "wrong_edge_rate=0.0000 sample_size=1 threshold<= 0.0000";
        assert_eq!(sample_size_from_detail(detail), Some(1));
        let big = "wrong_edge_rate=0.1136 sample_size=4057 threshold<= 0.1200";
        assert_eq!(sample_size_from_detail(big), Some(4057));
        let truncated = "wrong_edge_rate=0.0000 sample_size=123";
        assert_eq!(sample_size_from_detail(truncated), Some(123));
        let none = "count=0 threshold<= 0";
        assert_eq!(sample_size_from_detail(none), None);
    }
}
