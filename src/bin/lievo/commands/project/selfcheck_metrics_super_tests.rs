// #742 / PR #747 review fixes: verifier `#[path]` alias lookup in the
// super::/self:: arm, and the `mod`/`bmod` segment-pop correction.
// Split from selfcheck_metrics_tests.rs to stay within the 800-line test
// budget (AGENTS.md §6).

use super::*;
use std::collections::HashSet;

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

fn no_aliases() -> HashMap<String, String> {
    HashMap::new()
}

fn verifier_super_resolve(src: &str, spec: &str, paths: &[&str]) -> Option<String> {
    let known: HashSet<String> = paths.iter().map(|s| s.to_string()).collect();
    independent_resolve(src, spec, &known, "repo", &no_aliases())
}

fn production_super_resolve(src: &str, spec: &str, paths: &[&str]) -> Option<String> {
    let known: HashSet<&str> = paths.iter().copied().collect();
    lievo::analysis::resolve_rust_relative_for_agreement(src, spec, &known)
}

// ── FIX 2: super:: arm must run the #[path] alias lookup ───────────────

#[test]
fn super_parent_module_at_path_divergent_location_resolves_via_alias() {
    // The parent module's file is declared with a `#[path]` diverting it to
    // `src/elsewhere/storage.rs` (alias `deep` → physical file). The
    // literal locations `src/storage/deep.rs` / `src/storage/deep/mod.rs`
    // are ABSENT, so without the alias lookup the verifier arm returns None
    // while production (import_map holds the physical path) resolves — a
    // healthy edge would count as wrong and silently regress the ratchet.
    let known: HashSet<String> = ["src/elsewhere/storage.rs", "src/deep/util.rs"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    let resolved = independent_resolve("src/deep/util.rs", "super::x", &known, "repo", &aliases);
    assert_eq!(
        resolved.as_deref(),
        Some("src/elsewhere/storage.rs"),
        "the super:: arm must consult the #[path] alias map after the walk"
    );
}

#[test]
fn super_alias_lookup_requires_alias_target_in_corpus() {
    // An alias whose physical file is absent from the corpus must not be
    // returned (same contract as the crate:: arm) — no fabricated target.
    let known: HashSet<String> = ["src/deep/util.rs"].iter().map(|s| s.to_string()).collect();
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    let resolved = independent_resolve("src/deep/util.rs", "super::x", &known, "repo", &aliases);
    assert_eq!(resolved, None);
}

#[test]
fn super_literal_location_wins_over_alias() {
    // When the walked module's literal location exists, it wins and the
    // alias is not consulted (mirrors the crate:: arm's literal-first rule).
    let known: HashSet<String> = [
        "src/deep/mod.rs",
        "src/elsewhere/storage.rs",
        "src/deep/util.rs",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    let resolved = independent_resolve("src/deep/util.rs", "super::x", &known, "repo", &aliases);
    assert_eq!(resolved.as_deref(), Some("src/deep/mod.rs"));
}

// ── FIX 5: segment-pop correction (verifier copy) ──────────────────────

#[test]
fn importing_module_segments_verifier_file_named_bmod_keeps_segment() {
    // A file named `bmod.rs` keeps its `bmod` segment — the old string
    // suffix-strip of "/mod" would have mangled it to just the parent dir.
    assert_eq!(importing_module_segments("src/a/bmod.rs"), ["a", "bmod"]);
}

#[test]
fn importing_module_segments_verifier_directory_named_mod() {
    // A path containing a directory literally named `mod` — the trailing
    // `mod` pop only ever fires for a `mod.rs` marker (the leaf).
    assert_eq!(
        importing_module_segments("src/a/b/mod/leaf.rs"),
        ["a", "b", "mod", "leaf"]
    );
}

// ── end-to-end: verify_edges with the alias map ────────────────────────

#[test]
fn verify_edges_super_at_path_divergent_parent_not_flagged_wrong() {
    // Production records the edge against the physical (aliased) parent
    // file; the verifier must agree through the alias lookup — not flag the
    // healthy edge as wrong.
    let units = vec![unit_with_imports("src/deep/util.rs", &["super::x"])];
    let known = path_set(&["src/deep/util.rs", "src/elsewhere/storage.rs"]);
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    let sampled = vec![(
        "src/deep/util.rs".to_string(),
        "src/elsewhere/storage.rs".to_string(),
    )];
    let samples = verify_edges(&sampled, &units, &known, "repo", &aliases);
    assert!(
        !samples[0].wrong,
        "a super:: edge to a #[path]-divergent parent must not be flagged wrong"
    );
    assert_eq!(
        samples[0].resolved_target.as_deref(),
        Some("src/elsewhere/storage.rs")
    );
    assert_eq!(wrong_edge_rate(&samples), 0.0);
}

// ── production/verifier agreement at the alias boundary ────────────────

#[test]
fn super_alias_boundary_verifier_resolves_where_production_would_too() {
    // At the alias boundary the two resolvers use different mechanisms
    // (verifier: alias map; production: import_map of the physical path),
    // so agreement here is an alias-lookup sanity check: the verifier
    // resolves the divergent parent, and its answer matches the physical
    // file production's import_map would point at.
    let paths = ["src/deep/util.rs", "src/elsewhere/storage.rs"];
    let known: HashSet<String> = paths.iter().map(|s| s.to_string()).collect();
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    let verifier = independent_resolve("src/deep/util.rs", "super::x", &known, "repo", &aliases);
    assert_eq!(verifier.as_deref(), Some("src/elsewhere/storage.rs"));
    // Production's raw walk resolves against the literal locations of the
    // walked module (src/deep.rs / src/deep/mod.rs); with those absent and
    // only the alias physical file in the corpus, the raw walk has no
    // evidence either. What the test pins is the verifier's alias seam:
    // it lands on the exact physical file that production's import_map
    // would point at (production resolves via the import_map's raw-path
    // keys — the physical file IS in the corpus, so edges through it
    // resolve in production). The raw-walk agreement is pinned separately
    // for the literal-location case below.
    let physical_in_corpus = known.contains("src/elsewhere/storage.rs");
    assert!(
        physical_in_corpus,
        "the aliased physical file is in the corpus — production's import_map would resolve to it"
    );
    // The two resolvers' raw walks agree when the literal location exists.
    let literal_paths = ["src/deep/mod.rs", "src/deep/util.rs"];
    let verifier_literal = verifier_super_resolve("src/deep/util.rs", "super::x", &literal_paths);
    let prod_literal = production_super_resolve("src/deep/util.rs", "super::x", &literal_paths);
    assert_eq!(
        verifier_literal, prod_literal,
        "with the literal location present the raw walks agree"
    );
    assert_eq!(verifier_literal.as_deref(), Some("src/deep/mod.rs"));
}

#[test]
fn super_aliased_parent_missing_from_corpus_both_resolvers_none() {
    // The defect shape before the fix: the parent's physical file is NOT in
    // the corpus (e.g. a #[cfg(test)]-gated module). Production's walk
    // returns None (the file is not in known_paths); the verifier must ALSO
    // return None — an alias whose physical file is absent is no evidence.
    let paths = ["src/deep/util.rs"];
    let known: HashSet<String> = paths.iter().map(|s| s.to_string()).collect();
    let aliases = vec![("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
        .into_iter()
        .collect::<HashMap<_, _>>();
    assert_eq!(
        verifier_super_resolve("src/deep/util.rs", "super::x", &paths),
        production_super_resolve("src/deep/util.rs", "super::x", &paths),
    );
    assert_eq!(
        independent_resolve("src/deep/util.rs", "super::x", &known, "repo", &aliases),
        None,
        "alias to an absent physical file must not fabricate a target"
    );
}
