// Unit tests for the pure selfcheck metric/gate functions (issue #715).
// No storage, no I/O beyond `on_disk_grep_importer_count`'s injected fs read.

use super::super::selfcheck_false_zero::FalseZeroCaller;
use super::*;
use std::collections::{HashMap, HashSet};

fn unit_with_imports(file: &str, imports: &[&str]) -> CodeUnit {
    CodeUnit {
        name: "f".to_string(),
        qualified_name: "f".to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 5,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: imports.iter().map(|s| s.to_string()).collect(),
    }
}

fn path_set(paths: &[&str]) -> HashSet<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

#[test]
fn sample_edges_deterministic_ordering_and_cap() {
    let edges = vec![
        ("b.rs".to_string(), "a.rs".to_string()),
        ("a.rs".to_string(), "c.rs".to_string()),
        ("a.rs".to_string(), "b.rs".to_string()),
    ];
    let sampled = sample_edges(&edges, 2);
    assert_eq!(sampled.len(), 2);
    // sorted lexicographically: ("a.rs","b.rs") < ("a.rs","c.rs") < ("b.rs","a.rs")
    assert_eq!(sampled[0], ("a.rs".to_string(), "b.rs".to_string()));
    assert_eq!(sampled[1], ("a.rs".to_string(), "c.rs".to_string()));
}

#[test]
fn sample_edges_dedups() {
    let edges = vec![
        ("a.rs".to_string(), "b.rs".to_string()),
        ("a.rs".to_string(), "b.rs".to_string()),
    ];
    assert_eq!(sample_edges(&edges, 10).len(), 1);
}

// --- sample_edges: census semantics (issue #768) ---------------------------

#[test]
fn sample_edges_census_verifies_entire_deduped_population() {
    // A census (`sample_size` >= the dedup'd edge count) verifies every dedup'd
    // edge, in the same deterministic order as the n=cap shape (no RNG).
    let edges = vec![
        ("b.rs".to_string(), "a.rs".to_string()),
        ("a.rs".to_string(), "c.rs".to_string()),
        ("a.rs".to_string(), "b.rs".to_string()),
    ];
    let census = sample_edges(&edges, usize::MAX);
    assert_eq!(
        census.len(),
        3,
        "census verifies the whole dedup'd population"
    );
    assert_eq!(
        census,
        sample_edges(&edges, 3),
        "census order matches the cap shape"
    );
}

#[test]
fn sample_edges_census_still_dedups_before_verify() {
    // Dedup happens BEFORE the take: a census over a duplicated list verifies
    // the edge once, never twice.
    let edges = vec![
        ("a.rs".to_string(), "b.rs".to_string()),
        ("a.rs".to_string(), "b.rs".to_string()),
        ("a.rs".to_string(), "b.rs".to_string()),
    ];
    assert_eq!(sample_edges(&edges, usize::MAX).len(), 1);
}

// --- verify_edges / wrong_edge_rate (Rust crate:: paths) -------------------

fn no_aliases() -> HashMap<String, String> {
    HashMap::new()
}

#[test]
fn verify_edges_correct_rust_edge_not_wrong() {
    let units = vec![unit_with_imports("src/a.rs", &["crate::b"])];
    let known = path_set(&["src/a.rs", "src/b.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert_eq!(samples.len(), 1);
    assert!(
        !samples[0].wrong,
        "recorded target matches independent resolution"
    );
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

#[test]
fn verify_edges_flags_wrong_rust_edge() {
    // Recorded edge says a.rs -> c.rs, but the independent resolver, given
    // a.rs's only specifier "crate::b", resolves to b.rs, not c.rs.
    let units = vec![unit_with_imports("src/a.rs", &["crate::b"])];
    let known = path_set(&["src/a.rs", "src/b.rs", "src/c.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/c.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(samples[0].wrong);
    assert_eq!(wrong_edge_rate(&samples), 1.0);
}

// --- issue #724: Rust symbol-level `use crate::a::b::c` (trailing symbol) ---

#[test]
fn verify_edges_rust_symbol_in_sibling_module_file_not_wrong() {
    // The 732/1033 regression shape: a.rs imports `crate::b::c::some_fn`
    // (symbol `some_fn` in module `b::c`), the recorded edge points at the
    // module file src/b/c.rs, and NO file named `some_fn.rs` exists. The old
    // literal mapping (src/b/c/some_fn.rs / src/b/c/some_fn/mod.rs) produced
    // no candidate, so sibling specifiers in the same file caused a false
    // "wrong" flag. The trailing-symbol fallback must resolve the specifier to
    // src/b/c.rs.
    let units = vec![unit_with_imports("src/a.rs", &["crate::b::c::some_fn"])];
    let known = path_set(&["src/a.rs", "src/b/c.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b/c.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(
        !samples[0].wrong,
        "symbol-level use must not be flagged wrong"
    );
    assert_eq!(samples[0].resolved_target.as_deref(), Some("src/b/c.rs"));
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

#[test]
fn verify_edges_rust_symbol_in_nested_mod_rs_not_wrong() {
    // Same bug, mod.rs form: symbol `helper` in module `b::c` whose module
    // file is src/b/c/mod.rs. Literal mapping has no mod.rs candidate for the
    // full path, so this only resolves via the module-prefix fallback.
    let units = vec![unit_with_imports("src/a.rs", &["crate::b::c::helper"])];
    let known = path_set(&["src/a.rs", "src/b/c/mod.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b/c/mod.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(
        samples[0].resolved_target.as_deref(),
        Some("src/b/c/mod.rs")
    );
}

#[test]
fn verify_edges_rust_repo_prefix_symbol_not_wrong() {
    // The `{repo_name}::` alias of `crate::` must get the same trailing-symbol
    // fallback (independent_resolve strips repo-name-prefixed specifiers).
    let units = vec![unit_with_imports("src/a.rs", &["repo::b::c::helper"])];
    let known = path_set(&["src/a.rs", "src/b/c.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b/c.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target.as_deref(), Some("src/b/c.rs"));
}

#[test]
fn verify_edges_rust_literal_module_file_path_still_prefers_full_path() {
    // When the FULL specifier path literally maps to a file, that file wins
    // over any shorter prefix (no behavior change for the flat shape).
    let units = vec![unit_with_imports("src/a.rs", &["crate::b::c"])];
    let known = path_set(&["src/a.rs", "src/b.rs", "src/b/c.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b/c.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target.as_deref(), Some("src/b/c.rs"));
}

#[test]
fn verify_edges_rust_resolved_elsewhere_still_flagged_wrong() {
    // Contrast: the specifier DOES resolve (via the module-prefix fallback to
    // src/b/c.rs), but the recorded edge points at a DIFFERENT file — the fix
    // must not turn every resolvable specifier into a pass.
    let units = vec![unit_with_imports("src/a.rs", &["crate::b::c::helper"])];
    let known = path_set(&["src/a.rs", "src/b/c.rs", "src/d.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/d.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(samples[0].wrong, "resolved-but-mismatched edge stays wrong");
    assert_eq!(wrong_edge_rate(&samples), 1.0);
}

#[test]
fn verify_edges_rust_symbol_in_reexported_child_module_file_not_wrong() {
    // Issue #724 nested fixture: main.rs imports `crate::rustmod::deep::deep_label`;
    // the module chain resolves via the prefix fallback (src/rustmod/deep/mod.rs is
    // on disk), so the specifier's independent resolution is the module file.
    // The edge to the re-exported child file src/rustmod/deep/label.rs is a
    // SEPARATE recorded edge (a different relationship in the extraction
    // pipeline), not the resolution of this specifier — so it is not part of
    // this specifier's candidate set and is not flagged wrong by THIS
    // specifier's independent resolution. Verify the specifier's own edges
    // (to mod.rs and to the parent rustmod.rs) are both not wrong.
    let units = vec![unit_with_imports(
        "src/main.rs",
        &[
            "crate::rustmod::deep::deep_label",
            "crate::rustmod::module_name",
        ],
    )];
    let known = path_set(&[
        "src/main.rs",
        "src/rustmod.rs",
        "src/rustmod/deep/mod.rs",
        "src/rustmod/deep/label.rs",
    ]);
    let sampled1 = vec![(
        "src/main.rs".to_string(),
        "src/rustmod/deep/mod.rs".to_string(),
    )];
    let samples1 = verify_edges(&sampled1, &units, &known, "repo", &HashMap::new());
    assert!(
        !samples1[0].wrong,
        "edge to the module file (prefix-fallback resolution) must not be flagged wrong"
    );
    let sampled2 = vec![("src/main.rs".to_string(), "src/rustmod.rs".to_string())];
    let samples2 = verify_edges(&sampled2, &units, &known, "repo", &HashMap::new());
    assert!(
        !samples2[0].wrong,
        "edge to the parent module must not be flagged wrong"
    );
    assert_eq!(wrong_edge_rate(&samples1), 0.0);
    assert_eq!(wrong_edge_rate(&samples2), 0.0);
}

#[test]
fn verify_edges_rust_symbol_with_sibling_specifier_no_false_wrong() {
    // The sibling-import trap from the issue: a.rs has BOTH a specifiers whose
    // candidate set misses the recorded edge's file AND the symbol-level
    // specifier that resolves to it. The union of candidates must include the
    // recorded target, so the edge is not wrong.
    let units = vec![unit_with_imports(
        "src/a.rs",
        &["crate::b", "crate::b::c::some_fn"],
    )];
    let known = path_set(&["src/a.rs", "src/b.rs", "src/b/c.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b/c.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(
        !samples[0].wrong,
        "sibling specifier must not poison the candidate set"
    );
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

// --- independent_resolve: #[path] alias seam (moved from selfcheck_module_map_tests, issue #732 round-4) ---

#[test]
fn independent_resolve_uses_alias_when_literal_missing() {
    // The unit-level seam: `crate::a::cleanup` has no literal file
    // (src/a/cleanup.rs) but the alias map redirects to the divergent
    // physical file.
    let known: HashSet<String> = ["src/lib.rs", "src/a/mod.rs", "src/a/impls/cleanup.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = vec![(
        "a::cleanup".to_string(),
        "src/a/impls/cleanup.rs".to_string(),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let resolved = independent_resolve(
        "src/a/mod.rs",
        "crate::a::cleanup",
        &known,
        "lievo",
        &aliases,
    );
    assert_eq!(resolved.as_deref(), Some("src/a/impls/cleanup.rs"));
}

#[test]
fn independent_resolve_literal_wins_over_alias() {
    // Additive behaviour: when the literal location exists it is used and
    // the alias is not consulted (a #[path] pointing at the literal
    // location records no alias, but a manual map must not break literal
    // resolution either).
    let known: HashSet<String> = ["src/a/cleanup.rs", "src/a/impls/cleanup.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = vec![(
        "a::cleanup".to_string(),
        "src/a/impls/cleanup.rs".to_string(),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let resolved =
        independent_resolve("src/lib.rs", "crate::a::cleanup", &known, "lievo", &aliases);
    assert_eq!(resolved.as_deref(), Some("src/a/cleanup.rs"));
}

#[test]
fn independent_resolve_alias_fallback_with_trailing_symbol() {
    // `use crate::a::cleanup::run` — the last segment is a symbol, so the
    // trailing-segment fallback must still consult the alias map for the
    // module prefix `a::cleanup`.
    let known: HashSet<String> = ["src/a/impls/cleanup.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = vec![(
        "a::cleanup".to_string(),
        "src/a/impls/cleanup.rs".to_string(),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let resolved = independent_resolve(
        "src/lib.rs",
        "crate::a::cleanup::run",
        &known,
        "lievo",
        &aliases,
    );
    assert_eq!(resolved.as_deref(), Some("src/a/impls/cleanup.rs"));
}

// --- independent_resolve: bare-local `use` specifiers (issue #732 round-5) ---

#[test]
fn independent_resolve_bare_local_in_mod_rs_resolves_via_alias() {
    // The ticket's concrete case: `use pipeline_cleanup::apply;` in
    // `src/a/mod.rs` — Rust resolves the bare specifier to the crate root, so
    // the alias map must carry a root-level `pipeline_cleanup` entry (the
    // module declared at the crate root with a `#[path]` diverting it).
    let known: HashSet<String> = ["src/a/mod.rs", "src/pipeline_cleanup.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = vec![(
        "pipeline_cleanup".to_string(),
        "src/pipeline_cleanup.rs".to_string(),
    )]
    .into_iter()
    .collect::<HashMap<_, _>>();
    let resolved = independent_resolve(
        "src/a/mod.rs",
        "pipeline_cleanup::apply",
        &known,
        "repo",
        &aliases,
    );
    assert_eq!(resolved.as_deref(), Some("src/pipeline_cleanup.rs"));
}

#[test]
fn independent_resolve_bare_local_unknown_module_stays_no_evidence() {
    // A bare specifier naming a module that exists nowhere must yield no
    // evidence (None), not a resolved-but-mismatched candidate — the
    // trailing-segment fallback only narrows; it never fabricates a target.
    let known: HashSet<String> = ["src/a/mod.rs"].iter().map(|s| s.to_string()).collect();
    let resolved = independent_resolve("src/a/mod.rs", "zz::fn1", &known, "repo", &no_aliases());
    assert_eq!(resolved, None);
}

#[test]
fn independent_resolve_bare_local_resolves_literal_root_module() {
    // A bare specifier for a crate-root module present at its literal
    // location resolves without any alias (root-level literal mapping).
    let known: HashSet<String> = ["src/a/mod.rs", "src/b.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let resolved = independent_resolve("src/a/mod.rs", "b::fn1", &known, "repo", &no_aliases());
    assert_eq!(resolved.as_deref(), Some("src/b.rs"));
}

#[test]
fn independent_resolve_bare_local_bin_file_stays_no_evidence() {
    // A bare specifier in a bin-crate file that names nothing at the crate
    // root yields no evidence (no fabricated target).
    let known: HashSet<String> = ["src/bin/lievo/main.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let resolved = independent_resolve(
        "src/bin/lievo/main.rs",
        "commands::thing::x",
        &known,
        "repo",
        &no_aliases(),
    );
    assert_eq!(resolved, None);
}

#[test]
fn verify_edges_rust_unresolvable_symbol_stays_no_evidence() {
    // No plausible file derivation AND no module prefix on disk → the
    // specifier yields no candidate (no evidence), not a wrong edge (e.g.
    // symbols in inlined `mod { … }` blocks or re-exports).
    let units = vec![unit_with_imports("src/a.rs", &["crate::zz::yy"])];
    let known = path_set(&["src/a.rs", "src/b.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target, None);
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

#[test]
fn verify_edges_no_evidence_not_wrong() {
    // Specifier the independent resolver cannot resolve at all (bare/external)
    // must not count as wrong-edge evidence either way.
    let units = vec![unit_with_imports("src/a.rs", &["tokio::runtime"])];
    let known = path_set(&["src/a.rs", "src/b.rs"]);
    let sampled = vec![("src/a.rs".to_string(), "src/b.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target, None);
    assert_eq!(
        wrong_edge_rate(&samples),
        0.0,
        "no-evidence samples must not inflate the rate"
    );
}

#[test]
fn verify_edges_js_relative_specifier_resolves() {
    let units = vec![unit_with_imports("src/a.ts", &["./b"])];
    let known = path_set(&["src/a.ts", "src/b.ts"]);
    let sampled = vec![("src/a.ts".to_string(), "src/b.ts".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target.as_deref(), Some("src/b.ts"));
}

#[test]
fn verify_edges_js_relative_parent_dir_specifier() {
    let units = vec![unit_with_imports("src/sub/a.ts", &["../b"])];
    let known = path_set(&["src/sub/a.ts", "src/b.ts"]);
    let sampled = vec![("src/sub/a.ts".to_string(), "src/b.ts".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(!samples[0].wrong);
    assert_eq!(samples[0].resolved_target.as_deref(), Some("src/b.ts"));
}

#[test]
fn wrong_edge_rate_empty_samples_is_zero() {
    assert_eq!(wrong_edge_rate(&[]), 0.0);
}

// false-0-callers tests moved to selfcheck_false_zero_tests.rs (issue #715:
// section (b) was split into its own module to stay within the file-size
// budget). The gate-evaluation test below still lives here since
// gate_false_zero_callers takes `FalseZeroCaller` values directly.
// --- probes ------------------------------------------------------------------

#[test]
fn parse_probes_parses_tab_separated_lines() {
    let content = "auth flow\tsrc/auth.rs\n# comment\n\nrouting\tsrc/router.rs\n";
    let probes = parse_probes(content).unwrap();
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0].query, "auth flow");
    assert_eq!(probes[0].expected_path, "src/auth.rs");
    assert_eq!(probes[1].query, "routing");
}

#[test]
fn parse_probes_rejects_malformed_line() {
    let content = "no-tab-here\n";
    let err = parse_probes(content).unwrap_err();
    assert!(err.to_string().contains("probe file line 1"));
}

#[test]
fn parse_probes_empty_content_is_empty_vec() {
    assert!(parse_probes("").unwrap().is_empty());
}

// --- worst_probe_recall (min, not mean) ------------------------------------

fn probe_result(recall: f32) -> ProbeResult {
    ProbeResult {
        query: "q".to_string(),
        expected_path: "p".to_string(),
        recall,
        payload_bytes: 1000,
    }
}

#[test]
fn worst_probe_recall_is_min_not_mean() {
    let results = vec![probe_result(1.0), probe_result(1.0), probe_result(0.0)];
    assert_eq!(
        worst_probe_recall(&results),
        0.0,
        "one 0.0 probe must not be masked by several 1.0 probes"
    );
}

#[test]
fn worst_probe_recall_empty_is_zero() {
    assert_eq!(worst_probe_recall(&[]), 0.0);
}

// --- gates -------------------------------------------------------------------

#[test]
fn gate_edge_correctness_passes_at_threshold_fails_above() {
    let thresholds = SelfcheckThresholds {
        max_wrong_edge_rate: 0.1,
        ..SelfcheckThresholds::default()
    };
    assert!(
        super::super::selfcheck_edge_split::gate_edge_correctness(0.1, 10, 10, 0, &thresholds)
            .passed
    );
    assert!(
        !super::super::selfcheck_edge_split::gate_edge_correctness(0.11, 10, 10, 0, &thresholds)
            .passed
    );
}

#[test]
fn gate_false_zero_callers_passes_at_threshold_fails_above() {
    let thresholds = SelfcheckThresholds {
        max_false_zero_callers: 1,
        ..SelfcheckThresholds::default()
    };
    let one = vec![FalseZeroCaller {
        file: "a".to_string(),
        on_disk_importers: 1,
    }];
    let two = vec![
        FalseZeroCaller {
            file: "a".to_string(),
            on_disk_importers: 1,
        },
        FalseZeroCaller {
            file: "b".to_string(),
            on_disk_importers: 1,
        },
    ];
    assert!(gate_false_zero_callers(&one, &thresholds).passed);
    assert!(!gate_false_zero_callers(&two, &thresholds).passed);
}

#[test]
fn gate_retrieval_probes_fails_on_worst_probe_floor_violation() {
    let thresholds = SelfcheckThresholds {
        min_worst_probe: 0.5,
        min_recall_floor: 0.0,
        ..SelfcheckThresholds::default()
    };
    let results = vec![probe_result(1.0), probe_result(0.0)];
    let report = gate_retrieval_probes(&results, &thresholds);
    assert!(!report.passed);
    assert!(report.detail.contains("worst_probe"));
}

#[test]
fn gate_retrieval_probes_fails_on_per_probe_floor_even_if_worst_probe_passes() {
    let thresholds = SelfcheckThresholds {
        min_worst_probe: 0.0,
        min_recall_floor: 0.5,
        ..SelfcheckThresholds::default()
    };
    // worst-probe floor (min=0.3) passes at 0.0, but per-probe floor 0.5 fails on 0.3.
    let results = vec![probe_result(1.0), probe_result(0.3)];
    let report = gate_retrieval_probes(&results, &thresholds);
    assert!(!report.passed);
}

#[test]
fn gate_payload_bytes_uses_min_across_probes() {
    let thresholds = SelfcheckThresholds {
        min_payload_bytes: 500,
        ..SelfcheckThresholds::default()
    };
    let mut small = probe_result(1.0);
    small.payload_bytes = 100;
    let big = probe_result(1.0);
    let report = gate_payload_bytes(&[small, big], &thresholds);
    assert!(!report.passed, "one under-floor probe must fail the gate");
}

#[test]
fn skipped_section_is_always_passed_true() {
    let s = skipped_section("retrieval_probes", "no probes file");
    assert!(s.skipped);
    assert!(s.passed, "a skipped section must not count as a failure");
    assert_eq!(s.skip_reason.as_deref(), Some("no probes file"));
}

// --- issue #742 task-b: super::/self:: verifier arm + production agreement ---
// The production resolver is `lievo::analysis::resolve_rust_relative_for_agreement`
// (task-a); both must produce the same target or the gate flags the new edges
// as wrong. The verifier arm short-circuits before the crate-loop.
fn verifier_super_resolve(src: &str, spec: &str, paths: &[&str]) -> Option<String> {
    let known: HashSet<String> = paths.iter().map(|s| s.to_string()).collect();
    independent_resolve(src, spec, &known, "repo", &no_aliases())
}

fn production_super_resolve(src: &str, spec: &str, paths: &[&str]) -> Option<String> {
    let known: HashSet<&str> = paths.iter().copied().collect();
    lievo::analysis::resolve_rust_relative_for_agreement(src, spec, &known)
}

#[test]
fn super_single_level_sibling_form_agrees() {
    let paths = ["src/storage/mod.rs", "src/storage/sqlite_ops.rs"];
    assert_eq!(
        verifier_super_resolve("src/storage/sqlite_ops.rs", "super::util", &paths),
        production_super_resolve("src/storage/sqlite_ops.rs", "super::util", &paths),
        "production and verifier must agree on the same super:: target"
    );
    assert_eq!(
        production_super_resolve("src/storage/sqlite_ops.rs", "super::util", &paths),
        Some("src/storage/mod.rs".to_string())
    );
}

#[test]
fn super_sibling_form_beats_mod_rs_form_agrees() {
    // Both src/storage.rs and src/storage/mod.rs present: sibling wins.
    let paths = ["src/storage.rs", "src/storage/mod.rs"];
    assert_eq!(
        verifier_super_resolve("src/storage/x.rs", "super::x", &paths),
        production_super_resolve("src/storage/x.rs", "super::x", &paths),
        "probe order (sibling first) must be identical in both resolvers"
    );
    assert_eq!(
        production_super_resolve("src/storage/x.rs", "super::x", &paths),
        Some("src/storage.rs".to_string())
    );
}

#[test]
fn super_two_levels_counts_all_leading_supers_agrees() {
    // 3-deep layout where 1-hop and 2-hop land on different files.
    let paths = [
        "src/rustmod.rs",
        "src/rustmod/deep/mod.rs",
        "src/rustmod/deep/label.rs",
    ];
    assert_eq!(
        verifier_super_resolve("src/rustmod/deep/label.rs", "super::super::rustmod", &paths),
        production_super_resolve("src/rustmod/deep/label.rs", "super::super::rustmod", &paths),
        "super::super::X must count TWO leading super segments in both resolvers"
    );
    assert_eq!(
        production_super_resolve("src/rustmod/deep/label.rs", "super::super::rustmod", &paths),
        Some("src/rustmod.rs".to_string())
    );
    // 1-hop form lands one level lower in both.
    assert_eq!(
        verifier_super_resolve("src/rustmod/deep/label.rs", "super::deep_label", &paths),
        production_super_resolve("src/rustmod/deep/label.rs", "super::deep_label", &paths),
    );
    assert_eq!(
        production_super_resolve("src/rustmod/deep/label.rs", "super::deep_label", &paths),
        Some("src/rustmod/deep/mod.rs".to_string())
    );
}

#[test]
fn super_from_mod_rs_walks_to_parent_dir_agrees() {
    // mod.rs-form file: super:: walks to the grandparent dir's file.
    let paths = ["src/rustmod.rs", "src/rustmod/deep/mod.rs"];
    assert_eq!(
        verifier_super_resolve("src/rustmod/deep/mod.rs", "super::module_name", &paths),
        production_super_resolve("src/rustmod/deep/mod.rs", "super::module_name", &paths),
    );
    assert_eq!(
        production_super_resolve("src/rustmod/deep/mod.rs", "super::module_name", &paths),
        Some("src/rustmod.rs".to_string())
    );
}

#[test]
fn self_resolves_to_own_module_agrees() {
    // self:: resolves to the importing file's own module — both mod.rs-form
    // and sibling-form files must agree between production and verifier.
    let mod_rs = ["src/rustmod/deep/mod.rs"];
    assert_eq!(
        verifier_super_resolve("src/rustmod/deep/mod.rs", "self::label", &mod_rs),
        production_super_resolve("src/rustmod/deep/mod.rs", "self::label", &mod_rs),
    );
    assert_eq!(
        production_super_resolve("src/rustmod/deep/mod.rs", "self::label", &mod_rs),
        Some("src/rustmod/deep/mod.rs".to_string())
    );
    let sib = ["src/a/b.rs"];
    assert_eq!(
        verifier_super_resolve("src/a/b.rs", "self::x", &sib),
        production_super_resolve("src/a/b.rs", "self::x", &sib),
    );
    assert_eq!(
        production_super_resolve("src/a/b.rs", "self::x", &sib),
        Some("src/a/b.rs".to_string())
    );
}

#[test]
fn super_none_cases_agree_between_resolvers() {
    // Both resolvers must agree on None for: crate-root super::, underflow,
    // and missing parent file.
    for (src, spec, paths) in [
        ("src/lib.rs", "super::x", vec!["src/lib.rs"]),
        (
            "src/utils.rs",
            "super::super::x",
            vec!["src/lib.rs", "src/utils.rs"],
        ),
        ("src/a/b.rs", "super::x", vec!["src/a/b.rs"]),
    ] {
        let v = verifier_super_resolve(src, spec, &paths);
        let p = production_super_resolve(src, spec, &paths);
        assert_eq!(v, p, "{src} {spec}: resolvers must agree");
        assert_eq!(p, None, "{src} {spec}: expected None");
    }
}

#[test]
fn super_glob_and_grouped_member_specifiers_resolve_to_parent_agrees() {
    // `super::*` and `super::{a, b}`: the trailing glob/braces are symbol-tails
    // on the parent module, so the target is the parent file in both resolvers.
    let paths = ["src/storage/mod.rs"];
    for spec in ["super::*", "super::{a, b}"] {
        let v = verifier_super_resolve("src/storage/x.rs", spec, &paths);
        let p = production_super_resolve("src/storage/x.rs", spec, &paths);
        assert_eq!(v, p, "{spec}: resolvers must agree");
        assert_eq!(p, Some("src/storage/mod.rs".to_string()));
    }
}

#[test]
fn verify_edges_super_edge_to_parent_not_flagged_wrong() {
    // A super:: specifier that resolves independently to the recorded parent
    // file must not be flagged wrong — the verifier arm produces the parent
    // as a candidate, so the recorded edge is evidence-backed.
    let units = vec![unit_with_imports("src/storage/util.rs", &["super::x"])];
    let known = path_set(&["src/storage/util.rs", "src/storage/mod.rs"]);
    let sampled = vec![(
        "src/storage/util.rs".to_string(),
        "src/storage/mod.rs".to_string(),
    )];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(
        !samples[0].wrong,
        "super:: edge to the parent module must not be flagged wrong"
    );
    assert_eq!(
        samples[0].resolved_target.as_deref(),
        Some("src/storage/mod.rs"),
        "the independent super:: resolution must land on the recorded parent"
    );
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

#[test]
fn verify_edges_super_at_crate_root_stays_no_evidence() {
    // super:: from src/lib.rs: the verifier arm returns None (empty candidate
    // set) — no evidence either way, must not be flagged wrong.
    let units = vec![unit_with_imports("src/lib.rs", &["super::x"])];
    let known = path_set(&["src/lib.rs"]);
    let sampled = vec![("src/lib.rs".to_string(), "src/lib.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(
        !samples[0].wrong,
        "crate-root super:: must stay unclassified (no evidence), not be flagged wrong"
    );
    assert_eq!(samples[0].resolved_target, None);
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

#[test]
fn verify_edges_super_edge_to_missing_sibling_parent_stays_no_evidence() {
    // super:: from a file whose parent module file does not exist in the
    // known corpus; the recorded edge targets a non-mod.rs file so the
    // re-export expansion does not fire — candidate set is empty.
    let units = vec![unit_with_imports("src/a/b.rs", &["super::x"])];
    let known = path_set(&["src/a/b.rs"]);
    let sampled = vec![("src/a/b.rs".to_string(), "src/a.rs".to_string())];
    let samples = verify_edges(&sampled, &units, &known, "repo", &no_aliases());
    assert!(
        !samples[0].wrong,
        "a super:: edge whose parent file is absent from the corpus must not be flagged wrong"
    );
    assert_eq!(samples[0].resolved_target, None);
}
