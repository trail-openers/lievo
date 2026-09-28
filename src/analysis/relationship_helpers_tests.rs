use super::*;
use crate::model::{EntityTier, RelType};

#[test]
fn test_normalize_crate_name() {
    assert_eq!(normalize_crate_name("my-crate"), "my_crate");
    assert_eq!(normalize_crate_name("MyLib"), "mylib");
    assert_eq!(normalize_crate_name("repo-b"), "repo_b");
}

#[test]
fn test_rust_module_keys_mod_rs() {
    let keys = rust_module_keys_for_path("src/utils/mod.rs", "myrepo", "proj");
    // bare "utils" must NOT be present (Issue 2 fix)
    assert!(!keys.contains(&"utils".to_string()));
    assert!(keys.contains(&"crate::utils".to_string()));
    assert!(keys.contains(&"myrepo::utils".to_string()));
}

#[test]
fn test_rust_module_keys_regular() {
    let keys = rust_module_keys_for_path("src/utils.rs", "myrepo", "proj");
    // bare "utils" must NOT be present (Issue 2 fix)
    assert!(!keys.contains(&"utils".to_string()));
    assert!(keys.contains(&"crate::utils".to_string()));
}

#[test]
fn test_resolve_import_direct() {
    let mut map = HashMap::new();
    map.insert("src/b.rs".to_string(), "file-b-id".to_string());
    assert_eq!(
        resolve_import_for("src/b.rs", &map, None),
        Some("file-b-id")
    );
}

#[test]
fn test_resolve_import_prefix_fallback() {
    let mut map = HashMap::new();
    map.insert("utils".to_string(), "file-utils-id".to_string());
    assert_eq!(
        resolve_import_for("utils::hash_map", &map, None),
        Some("file-utils-id")
    );
}

#[test]
fn test_resolve_import_external_none() {
    assert_eq!(
        resolve_import_for("tokio::runtime", &HashMap::new(), None),
        None
    );
}

#[test]
fn test_resolve_import_rust_argument_field_shape() {
    // Pin the extractor/resolver contract: the Rust `argument` field output
    // (e.g. "std::collections::HashMap") must resolve when the import map
    // contains that exact key. The `::`-stripping fallback also works for
    // deeper paths.
    let mut map = HashMap::new();
    map.insert(
        "std::collections::HashMap".to_string(),
        "ext-std".to_string(),
    );
    assert_eq!(
        resolve_import_for("std::collections::HashMap", &map, None),
        Some("ext-std")
    );

    // Deeper path: the resolver strips trailing `::` segments until a match.
    let mut map2 = HashMap::new();
    map2.insert("crate::rust_helper".to_string(), "file-h".to_string());
    assert_eq!(
        resolve_import_for("crate::rust_helper::helper_label", &map2, None),
        Some("file-h"),
        "resolver should strip trailing :: segments until a map key matches"
    );
}

#[test]
fn test_resolve_import_js_relative_specifier() {
    // Pin the extractor/resolver contract for JS: the `source` field output
    // (e.g. "./ts_sample") must resolve when the import map contains that
    // exact key. No `::` stripping applies (no `::` in JS specifiers).
    let mut map = HashMap::new();
    map.insert("./ts_sample".to_string(), "file-ts".to_string());
    assert_eq!(
        resolve_import_for("./ts_sample", &map, None),
        Some("file-ts")
    );
    // An unrelated JS specifier resolves to None (external/unknown).
    assert_eq!(resolve_import_for("./nonexistent", &map, None), None);
}

#[test]
fn test_entity_parent_map() {
    use crate::model::Entity;
    let file = Entity {
        id: "file-a".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-x".to_string()),
        name: "a.rs".to_string(),
        path: Some("a.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let map = entity_parent_map(&[file]);
    assert_eq!(map.get("file-a"), Some(&"mod-x".to_string()));
}

#[test]
fn test_aggregate_depends_on_basic() {
    let mut edges: HashMap<(String, String, RelType), u32> = HashMap::new();
    edges.insert(
        ("file-a".to_string(), "file-b".to_string(), RelType::Imports),
        3,
    );
    let mut parent = HashMap::new();
    parent.insert("file-a".to_string(), "mod-x".to_string());
    parent.insert("file-b".to_string(), "mod-y".to_string());

    let result = aggregate_depends_on(&edges, &parent);
    assert_eq!(
        result.get(&("mod-x".to_string(), "mod-y".to_string())),
        Some(&3)
    );
}

#[test]
fn test_aggregate_depends_on_excludes_self() {
    let mut edges: HashMap<(String, String, RelType), u32> = HashMap::new();
    edges.insert(
        ("file-a".to_string(), "file-b".to_string(), RelType::Imports),
        1,
    );
    let mut parent = HashMap::new();
    parent.insert("file-a".to_string(), "mod-x".to_string());
    parent.insert("file-b".to_string(), "mod-x".to_string()); // same module

    assert!(aggregate_depends_on(&edges, &parent).is_empty());
}

#[test]
fn test_build_fn_map_ambiguous_skipped() {
    use crate::model::Entity;
    let file_a = Entity {
        id: "file-a".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("a.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let file_b = Entity {
        id: "file-b".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "b.rs".to_string(),
        path: Some("b.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let unit_a = CodeUnit {
        name: "helper".to_string(),
        qualified_name: "a::helper".to_string(),
        unit_type: "function".to_string(),
        file: "a.rs".to_string(),
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
        imports: vec![],
    };
    let mut unit_b = unit_a.clone();
    unit_b.file = "b.rs".to_string();
    unit_b.qualified_name = "b::helper".to_string();

    let units = [unit_a, unit_b];
    let files = [file_a, file_b];
    let map = build_fn_map(&units, &files);
    // "helper" defined by 2 files → must be absent
    assert!(!map.contains_key("helper"));
}

#[test]
fn test_build_fn_map_same_file_two_units_not_ambiguous() {
    use crate::model::Entity;
    let file_a = Entity {
        id: "file-a".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("a.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let unit1 = CodeUnit {
        name: "helper".to_string(),
        qualified_name: "a::helper".to_string(),
        unit_type: "function".to_string(),
        file: "a.rs".to_string(),
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
        imports: vec![],
    };
    // Second unit in the SAME file, same name (e.g. duplicate parse artifact)
    let mut unit2 = unit1.clone();
    unit2.line = 6; // Change line to make it unique while keeping same name

    let units = [unit1, unit2];
    let files = [file_a];
    let map = build_fn_map(&units, &files);
    // Both units are in the same file → NOT ambiguous → "helper" must be present
    assert!(
        map.contains_key("helper"),
        "same-file duplicates should not suppress the name"
    );
}

#[test]
fn test_crate_errors_import_resolves_to_errors_rs() {
    // Regression test for #432: `use crate::errors` must resolve to `src/errors.rs`
    let mut files = vec![];
    // Create a mock Entity for src/errors.rs
    use crate::model::EntityTier;
    let errors_file = Entity {
        id: "proj:file:src/errors.rs".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "errors.rs".to_string(),
        path: Some("src/errors.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    files.push(errors_file);

    let map = build_import_map(&files, "lievo", "lievo");

    // The key "crate::errors" must be present and resolve to the errors.rs entity
    assert!(
        map.contains_key("crate::errors"),
        "import_map must contain 'crate::errors' key for src/errors.rs"
    );
    assert_eq!(
        resolve_import_for("crate::errors", &map, None),
        Some("proj:file:src/errors.rs"),
        "use crate::errors must resolve to src/errors.rs entity"
    );
    // Also verify lievo::errors works
    assert_eq!(
        resolve_import_for("lievo::errors", &map, None),
        Some("proj:file:src/errors.rs"),
        "use lievo::errors must also resolve to src/errors.rs entity"
    );
}

// ── #681 hard prerequisite: repo-relative path normalisation ──────────

#[test]
fn test_normalize_to_repo_relative_relative_path_passthrough() {
    // Relative paths are used as-is
    assert_eq!(
        normalize_to_repo_relative("src/utils.rs", "myrepo"),
        "src/utils.rs"
    );
    assert_eq!(normalize_to_repo_relative("utils.rs", "myrepo"), "utils.rs");
}

#[test]
fn test_normalize_to_repo_relative_absolute_path_with_repo_name() {
    // Absolute path containing the repo name is normalized to repo-relative
    let path = "/tmp/example-repo/src/utils.rs";
    let result = normalize_to_repo_relative(path, "myrepo");
    assert_eq!(result, "src/utils.rs");
}

#[test]
fn test_normalize_to_repo_relative_absolute_path_fallback_to_src() {
    // Absolute path without the repo name falls back to /src/ marker
    let path = "/home/user/projects/other-repo/src/utils.rs";
    let result = normalize_to_repo_relative(path, "different-repo");
    assert_eq!(result, "src/utils.rs");
}

#[test]
fn test_normalize_to_repo_relative_absolute_no_src_marker() {
    // Absolute path without repo name or /src/ marker: use path as-is
    let path = "/home/user/projects/other-repo/lib.rs";
    let result = normalize_to_repo_relative(path, "different-repo");
    // Falls through to the last-resort: use the path as-is
    // (This will produce garbage keys but won't panic)
    assert_eq!(result, path);
}

#[test]
fn test_rust_module_keys_absolute_path_normalization() {
    // #681 hard prerequisite: absolute paths must produce the same keys
    // as repo-relative paths
    let abs_keys = rust_module_keys_for_path("/tmp/example-repo/src/utils.rs", "lievo", "proj");
    let rel_keys = rust_module_keys_for_path("src/utils.rs", "lievo", "proj");

    assert_eq!(
        abs_keys, rel_keys,
        "absolute and relative paths must produce identical module keys"
    );
    assert!(abs_keys.contains(&"crate::utils".to_string()));
    assert!(abs_keys.contains(&"lievo::utils".to_string()));
}

#[test]
fn test_rust_module_keys_absolute_mod_rs_normalization() {
    // #681 hard prerequisite: absolute mod.rs paths must normalize correctly
    let abs_keys = rust_module_keys_for_path(
        "/home/user/projects/myrepo/src/utils/mod.rs",
        "myrepo",
        "proj",
    );
    let rel_keys = rust_module_keys_for_path("src/utils/mod.rs", "myrepo", "proj");

    assert_eq!(
        abs_keys, rel_keys,
        "absolute and relative mod.rs paths must produce identical keys"
    );
    assert!(abs_keys.contains(&"crate::utils".to_string()));
    assert!(abs_keys.contains(&"myrepo::utils".to_string()));
}

#[test]
fn test_rust_module_keys_absolute_nested_path_normalization() {
    // #681 hard prerequisite: nested absolute paths must normalize correctly
    let abs_keys = rust_module_keys_for_path(
        "/home/user/projects/myrepo/src/storage/sqlite_ops.rs",
        "myrepo",
        "proj",
    );
    let rel_keys = rust_module_keys_for_path("src/storage/sqlite_ops.rs", "myrepo", "proj");

    assert_eq!(
        abs_keys, rel_keys,
        "absolute and relative nested paths must produce identical keys"
    );
    assert!(abs_keys.contains(&"crate::storage::sqlite_ops".to_string()));
    assert!(abs_keys.contains(&"myrepo::storage::sqlite_ops".to_string()));
}

// ── #742 task-a: relative super::/self:: resolver ─────────────────────

fn known(v: impl IntoIterator<Item = &'static str>) -> HashSet<&'static str> {
    v.into_iter().collect()
}

#[test]
fn test_super_single_level_sibling_form() {
    // src/storage/sqlite_ops.rs → parent module storage → src/storage/mod.rs
    let known_paths = known(["src/storage/mod.rs", "src/storage/sqlite_ops.rs"]);
    assert_eq!(
        resolve_rust_relative("src/storage/sqlite_ops.rs", "super::util", &known_paths),
        Some("src/storage/mod.rs".to_string())
    );
}

#[test]
fn test_super_sibling_form_beats_mod_rs_form() {
    // Both src/storage.rs and src/storage/mod.rs present: sibling wins
    // (Rust treats the two forms as mutually exclusive, but the probe
    // order must be pinned to sibling-first anyway).
    let known_paths = known(["src/storage.rs", "src/storage/mod.rs"]);
    assert_eq!(
        resolve_rust_relative("src/storage/sqlite_ops.rs", "super::x", &known_paths),
        Some("src/storage.rs".to_string())
    );
}

#[test]
fn test_super_two_levels_counts_all_leading_supers() {
    // The off-by-one defect (issue #742): a 1-hop and a 2-hop must land
    // on DIFFERENT files on this 3-deep layout.
    // src/rustmod/deep/label.rs: 1 hop → src/rustmod/deep/mod.rs,
    // 2 hops → src/rustmod.rs.
    let known_paths = known([
        "src/rustmod.rs",
        "src/rustmod/deep/mod.rs",
        "src/rustmod/deep/label.rs",
    ]);
    assert_eq!(
        resolve_rust_relative(
            "src/rustmod/deep/label.rs",
            "super::super::rustmod",
            &known_paths
        ),
        Some("src/rustmod.rs".to_string()),
        "super::super::X must count ALL two leading super segments"
    );
    // Sanity: the 1-hop form lands one level lower.
    assert_eq!(
        resolve_rust_relative(
            "src/rustmod/deep/label.rs",
            "super::deep_label",
            &known_paths
        ),
        Some("src/rustmod/deep/mod.rs".to_string())
    );
}

#[test]
fn test_super_from_mod_rs_file_walks_to_parent_dir() {
    // src/rustmod/deep/mod.rs (a mod.rs-form module): its own module is
    // `rustmod::deep`; super:: walks to `rustmod` → src/rustmod.rs.
    let known_paths = known(["src/rustmod.rs", "src/rustmod/deep/mod.rs"]);
    assert_eq!(
        resolve_rust_relative(
            "src/rustmod/deep/mod.rs",
            "super::module_name",
            &known_paths
        ),
        Some("src/rustmod.rs".to_string())
    );
}

#[test]
fn test_self_resolves_to_own_module() {
    // self:: in a mod.rs-form file resolves to that file itself.
    let known_paths = known(["src/rustmod/deep/mod.rs"]);
    assert_eq!(
        resolve_rust_relative("src/rustmod/deep/mod.rs", "self::label", &known_paths),
        Some("src/rustmod/deep/mod.rs".to_string())
    );
}

#[test]
fn test_super_at_crate_root_is_none() {
    let known_paths = known(["src/lib.rs"]);
    assert_eq!(
        resolve_rust_relative("src/lib.rs", "super::x", &known_paths),
        None,
        "super:: at the crate root must not produce a self-edge or panic"
    );
}

#[test]
fn test_nested_super_cannot_underflow_past_crate_root() {
    // src/utils.rs (depth 1): super::super::x walks past the crate root
    // and must return None, not a bogus edge.
    let known_paths = known(["src/lib.rs", "src/utils.rs"]);
    assert_eq!(
        resolve_rust_relative("src/utils.rs", "super::super::x", &known_paths),
        None
    );
}

#[test]
fn test_super_parent_file_missing_returns_none() {
    // The parent module's file does not exist in the known corpus → no
    // guess, None.
    let known_paths = known(["src/a/b.rs"]);
    assert_eq!(
        resolve_rust_relative("src/a/b.rs", "super::x", &known_paths),
        None
    );
}

#[test]
fn test_super_glob_resolves_to_parent_module() {
    // `use super::*`: one edge to the parent module file, no expansion.
    let known_paths = known(["src/storage/mod.rs"]);
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "super::*", &known_paths),
        Some("src/storage/mod.rs".to_string())
    );
}

#[test]
fn test_super_grouped_member_specifier() {
    // Grouped `use super::{a, b}` reaches the resolver verbatim as one
    // specifier string; the member braces/symbols are tails that name the
    // parent module's contents, so the target is the parent file.
    let known_paths = known(["src/storage/mod.rs"]);
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "super::{a, b}", &known_paths),
        Some("src/storage/mod.rs".to_string())
    );
}

#[test]
fn test_resolve_import_for_super_edge_shape_matches_crate() {
    // The edge target for `super::x` must be the same file entity that an
    // equivalent `crate::` import resolves to — the same edge shape.
    let mut map = HashMap::new();
    map.insert(
        "src/storage/mod.rs".to_string(),
        "id-storage-mod".to_string(),
    );
    map.insert(
        "src/storage/util.rs".to_string(),
        "id-storage-util".to_string(),
    );
    for k in rust_module_keys_for_path("src/storage/mod.rs", "proj", "p") {
        map.entry(k).or_insert_with(|| "id-storage-mod".to_string());
    }
    for k in rust_module_keys_for_path("src/storage/util.rs", "proj", "p") {
        map.entry(k)
            .or_insert_with(|| "id-storage-util".to_string());
    }
    // crate::storage and super:: (1-hop from src/storage/util.rs) both
    // resolve to the parent module file src/storage/mod.rs — the same edge
    // shape.
    let via_crate = resolve_import_for("crate::storage", &map, None);
    let via_super = resolve_import_for("super::x", &map, Some("src/storage/util.rs"));
    assert_eq!(
        via_crate, via_super,
        "super:: and the equivalent crate:: specifier must resolve to the same entity"
    );
    assert_eq!(via_super, Some("id-storage-mod"));
}

#[test]
fn test_resolve_import_for_super_at_crate_root_none() {
    let mut map = HashMap::new();
    map.insert("src/lib.rs".to_string(), "id-lib".to_string());
    assert_eq!(
        resolve_import_for("super::x", &map, Some("src/lib.rs")),
        None,
        "crate-root super:: must stay unresolved (no self-edge)"
    );
}

#[test]
fn test_resolve_import_for_relative_without_context_stays_unresolved() {
    // Without the importing file's context threaded in (importing_file
    // None) a super:: specifier must not be guessed — it stays unresolved
    // exactly as before #742.
    let mut map = HashMap::new();
    map.insert(
        "src/storage/mod.rs".to_string(),
        "id-storage-mod".to_string(),
    );
    assert_eq!(resolve_import_for("super::x", &map, None), None);
}

#[test]
fn test_super_two_levels_off_by_one_defect_guard() {
    // Pins the prior-attempt defect directly: `super::super::X` must
    // count TWO leading super segments on the ORIGINAL specifier, not one
    // (strip-one-then-count-remainder yields the wrong target).
    let known_paths = known(["src/a/b/c.rs", "src/a/b/mod.rs", "src/a/mod.rs"]);
    assert_eq!(
        resolve_rust_relative("src/a/b/c.rs", "super::super::x", &known_paths),
        Some("src/a/mod.rs".to_string()),
        "two leading super segments must walk two levels, not one"
    );
}

#[test]
fn test_importing_module_segments_mod_rs_form() {
    // mod.rs form: the file's own module is its parent directory.
    assert_eq!(
        importing_module_segments("src/storage/mod.rs"),
        vec!["storage".to_string()]
    );
}

#[test]
fn test_importing_module_segments_lib_is_crate_root() {
    // src/lib.rs is the crate root (empty segment list).
    assert!(importing_module_segments("src/lib.rs").is_empty());
}

#[test]
fn test_importing_module_segments_file_named_bmod_keeps_segment() {
    // Review fix (PR #747): a file named `bmod.rs` must keep its `bmod`
    // segment — the old string-suffix strip of "/mod" would have mangled it.
    assert_eq!(
        importing_module_segments("src/a/bmod.rs"),
        vec!["a".to_string(), "bmod".to_string()]
    );
}

#[test]
fn test_super_and_self_arms_ordering_independent() {
    // Ordering regression guard (issue #810): with the type-enforced
    // let-else, each arm's prefix check is a full "super::"/"self::"
    // string guard that returns `None` cleanly on a non-match rather than
    // relying on match-arm ordering. This test pins that the super:: and
    // self:: arms resolve identically regardless of arm ordering — i.e.
    // adding a new arm before the super:: arm does NOT change what these
    // specifiers resolve to.
    let known_paths = known(["src/storage/mod.rs", "src/storage/x.rs"]);
    // super:: arm: walks one level up to src/storage/mod.rs.
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "super::util", &known_paths),
        Some("src/storage/mod.rs".to_string())
    );
    // self:: arm: stays at own module (src/storage/x.rs → x module).
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "self::label", &known_paths),
        Some("src/storage/x.rs".to_string())
    );
    // Neither super:: nor self:: — must fall through to None cleanly (the
    // let-else makes the non-matching prefix yield None, not panic on a
    // reordered arm).
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "crate::x", &known_paths),
        None
    );
    assert_eq!(
        resolve_rust_relative("src/storage/x.rs", "superior::x", &known_paths),
        None,
        "a prefix that shares a `super` head but is not exactly `super::` must not misfire"
    );
}

#[test]
fn test_importing_module_segments_directory_named_mod() {
    // Review fix (PR #747): a path containing a directory literally named
    // `mod` — the `mod` pop is only the trailing `mod.rs` marker, so the
    // intermediate `mod` directory and the leaf module both survive.
    assert_eq!(
        importing_module_segments("src/a/b/mod/leaf.rs"),
        vec![
            "a".to_string(),
            "b".to_string(),
            "mod".to_string(),
            "leaf".to_string()
        ]
    );
}
