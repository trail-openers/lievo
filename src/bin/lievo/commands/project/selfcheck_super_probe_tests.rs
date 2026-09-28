// Unit tests for the `#[path]` alias-map integration into the structural
// super:: probe (issue #758).
//
// These tests verify that the probe correctly consults the physical→logical
// and logical→physical halves of the `#[path]` alias map when locating
// target files, and that the new target-location refinement (directory walk
// miss → physical parent fallback for non-relocated files) behaves as
// specified.

use std::collections::HashMap;

use super::super::selfcheck_ops::selfcheck_ops_tests::rust_unit_with_imports;
use super::super::selfcheck_ops::selfcheck_ops_tests::write_repo;

use super::probe_super_sites;

// ---------------------------------------------------------------------------
// AC: `#[path]`-relocated module's super:: import resolves against its
// LOGICAL parent and is confirmed, where the directory-derived walk would
// have failed it.
// ---------------------------------------------------------------------------

#[test]
fn probe_path_relocated_resolves_to_logical_parent() {
    // Fixture: `src/analysis/mod.rs` declares `mod relationships;`, and
    // `src/analysis/relationships.rs` declares
    // `#[path = "relationships_function_edges.rs"] mod relationships_function_edges;`.
    // The file `src/analysis/relationships_function_edges.rs` is physically
    // in `src/analysis/` but logically its parent is `relationships` (the
    // declaring file). The alias map maps physical
    // `src/analysis/relationships_function_edges.rs` → logical
    // `relationships::relationships_function_edges`.
    //
    // Site: `use super::relationships_aggregate` in
    // `src/analysis/relationships_function_edges.rs`.
    //
    // Directory walk: segments = [analysis, relationships_function_edges];
    // 1-hop walk → [analysis] → `src/analysis/mod.rs`. Target = mod.rs.
    // `src/analysis/mod.rs` declares `mod relationships;` but NOT
    // `relationships_aggregate` → FAILURE under directory walk.
    //
    // Alias-map path: logical = [relationships, relationships_function_edges];
    // 1-hop walk → [relationships] → `src/analysis/relationships.rs` (literal
    // hit). `relationships.rs` declares `mod relationships_aggregate;` →
    // CONFIRMED.
    //
    // This is the exact scenario that was failing before #758.
    let dir = write_repo(&[
        ("src/lib.rs", "mod analysis;\n"),
        ("src/analysis/mod.rs", "mod relationships;\n"),
        (
            "src/analysis/relationships.rs",
            "mod relationships_aggregate;\n#[path = \"relationships_function_edges.rs\"] mod relationships_function_edges;\n",
        ),
        (
            "src/analysis/relationships_aggregate.rs",
            "pub struct EdgeAggregation;\n",
        ),
        (
            "src/analysis/relationships_function_edges.rs",
            "use super::relationships_aggregate::EdgeAggregation;\nfn f() {}\n",
        ),
    ]);
    let units = vec![rust_unit_with_imports(
        "src/analysis/relationships_function_edges.rs",
        &["super::relationships_aggregate::EdgeAggregation"],
    )];
    let all_files = vec![
        "src/lib.rs".to_string(),
        "src/analysis/mod.rs".to_string(),
        "src/analysis/relationships.rs".to_string(),
        "src/analysis/relationships_aggregate.rs".to_string(),
        "src/analysis/relationships_function_edges.rs".to_string(),
    ];

    // Empty alias map: directory walk → mod.rs → no `relationships_aggregate`
    // → failure.
    let empty_map = HashMap::new();
    let results_dir = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &empty_map,
        &empty_map,
    );
    assert_eq!(
        results_dir.failures, 1,
        "directory walk without alias map: failure (target mod.rs lacks the name)"
    );
    assert_eq!(results_dir.confirmed, 0);

    // Alias map populated: the file's logical path is
    // [analysis, relationships, relationships_function_edges]; 1-hop walk
    // → [analysis, relationships] → literal `src/analysis/relationships.rs`
    // (exists). `relationships.rs` declares `mod relationships_aggregate;`
    // → `relationships_aggregate` is in the provided set → confirmed.
    //
    // The import `super::relationships_aggregate::EdgeAggregation` has the
    // referenced name `relationships_aggregate` (first segment after
    // `super::`).
    let mut physical_to_logical = HashMap::new();
    physical_to_logical.insert(
        "src/analysis/relationships_function_edges.rs".to_string(),
        "analysis::relationships::relationships_function_edges".to_string(),
    );
    let results_map = probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &physical_to_logical,
    );
    assert_eq!(
        results_map.confirmed, 1,
        "alias-map path: resolves to relationships.rs which declares the module: confirmed"
    );
    assert_eq!(results_map.failures, 0);
    assert_eq!(results_map.indeterminate, 0);
}

// ---------------------------------------------------------------------------
// AC: a file with no alias-map entry still resolves via directory layout
// exactly as before.
// ---------------------------------------------------------------------------

#[test]
fn probe_no_alias_entry_uses_directory_layout() {
    // `use super::b` in `src/a/b.rs`: the referenced module walks to the
    // parent module `a` (file `src/a.rs`); its one-level-up parent is the
    // crate root `src/lib.rs`, which declares `mod b;` → confirmed.
    // No alias map entry for `src/a/b.rs` — directory layout is used.
    let dir = write_repo(&[
        ("src/lib.rs", "mod a;\nmod b;\n"),
        ("src/a.rs", "mod b;\n"),
        ("src/a/b.rs", "use super::b;\nfn f() {}\n"),
    ]);
    let units = vec![rust_unit_with_imports("src/a/b.rs", &["super::b"])];
    let all_files = vec![
        "src/lib.rs".to_string(),
        "src/a.rs".to_string(),
        "src/a/b.rs".to_string(),
    ];
    let results = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert_eq!(
        results.confirmed, 1,
        "no alias entry: directory layout resolves as before"
    );
    assert_eq!(results.failures, 0);
    assert_eq!(results.indeterminate, 0);
}

// ---------------------------------------------------------------------------
// AC: the test-like-file exclusion and crate-root exemption from #744 still
// behave identically.
// ---------------------------------------------------------------------------

#[test]
fn probe_test_like_file_exclusion_unchanged_with_alias_map() {
    // A `use super::gone` located in a test-like file (`src/a/b_tests.rs`)
    // is excluded from the site list entirely — 0 of any kind, never a
    // failure. The alias map is present but should not affect the exclusion.
    let dir = write_repo(&[
        ("src/lib.rs", "mod a;\n"),
        ("src/a.rs", "mod b;\n"),
        ("src/a/b_tests.rs", "use super::gone;\nfn f() {}\n"),
    ]);
    let units = vec![rust_unit_with_imports("src/a/b_tests.rs", &["super::gone"])];
    let all_files = vec![
        "src/lib.rs".to_string(),
        "src/a.rs".to_string(),
        "src/a/b_tests.rs".to_string(),
    ];
    let mut physical_to_logical = HashMap::new();
    physical_to_logical.insert("src/a/b_tests.rs".to_string(), "a::b_tests".to_string());
    let results = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &physical_to_logical,
    );
    assert_eq!(results.confirmed, 0);
    assert_eq!(
        results.failures, 0,
        "test-like file site excluded (unchanged with alias map)"
    );
    assert_eq!(results.indeterminate, 0);
}

#[test]
fn probe_crate_root_exemption_unchanged_with_alias_map() {
    // `use super::x` in `src/lib.rs` (the crate root): the root is exempt
    // from the test-like check (rule c flags `lib.rs`), so the site is
    // processed. The walk goes past the root → indeterminate.
    let dir = write_repo(&[
        ("src/lib.rs", "mod a;\nuse super::x;\nfn f() {}\n"),
        ("src/a.rs", "fn g() {}\n"),
    ]);
    let units = vec![rust_unit_with_imports("src/lib.rs", &["super::x"])];
    let all_files = vec!["src/lib.rs".to_string(), "src/a.rs".to_string()];
    let results = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert_eq!(
        results.indeterminate, 1,
        "crate-root site: walk past root is indeterminate (unchanged)"
    );
    assert_eq!(results.failures, 0);
}

// ---------------------------------------------------------------------------
// AC: a site whose logical parent cannot be determined is indeterminate,
// not a failure.
// ---------------------------------------------------------------------------

#[test]
fn probe_unresolvable_logical_parent_is_indeterminate() {
    // `use super::gone` in `src/a/b.rs` where the walked parent module file
    // `src/a.rs` DOES NOT exist: the referenced module cannot be located —
    // indeterminate (never a failure). No alias map entry → the
    // non-relocated refinement applies: physical parent = `src/a.rs` is
    // also missing → indeterminate.
    let dir = write_repo(&[
        ("src/lib.rs", "mod a;\n"),
        ("src/a/b.rs", "use super::gone;\nfn f() {}\n"),
    ]);
    let units = vec![rust_unit_with_imports("src/a/b.rs", &["super::gone"])];
    let all_files = vec!["src/lib.rs".to_string(), "src/a/b.rs".to_string()];
    let results = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert_eq!(
        results.indeterminate, 1,
        "missing parent file (no alias entry): indeterminate, not failure"
    );
    assert_eq!(results.failures, 0);
}

#[test]
fn probe_include_d_file_degrades_to_indeterminate() {
    // The `include!` class: a file whose directory parent does NOT exist
    // (the file is pulled in via `include!` from a file in a different
    // location). The directory walk finds no module file; the alias map
    // has no entry (include! is not covered); the physical parent (the file
    // in the same directory) is also missing → indeterminate.
    //
    // Fixture: `src/deep/x.rs` does `use super::gone`. The directory
    // `src/deep/` has no `mod.rs` or `deep.rs` in the corpus. No alias map
    // entry. The physical parent of `src/deep/x.rs` would be `src/deep.rs`
    // or `src/deep/mod.rs` — neither in the corpus → indeterminate.
    let dir = write_repo(&[
        ("src/lib.rs", "mod deep;\n"),
        ("src/deep/x.rs", "use super::gone;\nfn f() {}\n"),
    ]);
    let units = vec![rust_unit_with_imports("src/deep/x.rs", &["super::gone"])];
    let all_files = vec!["src/lib.rs".to_string(), "src/deep/x.rs".to_string()];
    let results = super::super::selfcheck_super_probe::probe_super_sites(
        &units,
        &all_files,
        dir.path(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert_eq!(
        results.indeterminate, 1,
        "include!-class site (no module file, no alias, no parent file): indeterminate"
    );
    assert_eq!(
        results.failures, 0,
        "include! class must not be a false failure"
    );
}
