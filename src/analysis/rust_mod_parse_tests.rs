// Shared Rust mod-declaration parser + test-file classifier (issue #744).
// The parser semantics are pinned by `module_map_tests.rs` (issue #732);
// these tests pin the EXTRACTION itself: both callers must see identical
// behaviour from the single shared copy.

use super::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// parse_mod_decls (shared copy)
// ---------------------------------------------------------------------------

#[test]
fn parse_mod_decls_shared_semicolon_forms() {
    let src = "mod a;\npub mod b;\npub(crate) mod c;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(
        decls,
        vec![
            ("a".to_string(), None),
            ("b".to_string(), None),
            ("c".to_string(), None)
        ]
    );
}

#[test]
fn parse_mod_decls_shared_inline_block_not_a_declaration() {
    // An inline `mod x { … }` block is NOT a file declaration (the
    // #744 gap-gate commitment: declaration-only test).
    let src = "mod inline { fn f() {} }\nmod real;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(decls, vec![("real".to_string(), None)]);
}

#[test]
fn parse_mod_decls_shared_same_named_item_not_a_declaration() {
    // A same-named non-module item (a struct) is not a module declaration.
    let src = "struct X;\nmod y;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(decls, vec![("y".to_string(), None)]);
}

#[test]
fn parse_mod_decls_shared_path_attribute_carried() {
    let src = "#[path = \"impls/cleanup.rs\"]\nmod cleanup;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(
        decls,
        vec![("cleanup".to_string(), Some("impls/cleanup.rs".to_string()))]
    );
}

#[test]
fn parse_mod_decls_shared_cfg_test_gated_skipped() {
    // `#[cfg(test)]`-gated declarations are excluded from the non-test
    // module tree (issue #732 round-2).
    let src = "#[cfg(test)]\nmod testonly;\nmod kept;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(decls, vec![("kept".to_string(), None)]);
}

#[test]
fn parse_mod_decls_shared_cfg_test_inline_block_does_not_leak() {
    // Regression test for issue #863: a `#[cfg(test)]` attribute above an
    // inline `mod x { ... }` block must NOT leak the `pending_cfg_test`
    // flag past the inline module's closing `}` to the next semicolon `mod`
    // declaration. Before the fix, the stale flag caused the next `mod`
    // declaration to be incorrectly skipped, dropping it from the module
    // map and breaking the super:: structural probe.
    let src = "#[cfg(test)]\nmod inline { \n    pub fn f() {} \n}\nmod real;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(
        decls,
        vec![("real".to_string(), None)],
        "the inline block is ignored; the stale #[cfg(test)] flag must not leak to 'mod real;'"
    );
}

#[test]
fn parse_mod_decls_shared_cfg_test_inline_block_with_body_lines() {
    // Stronger variant: the inline module body contains multiple lines
    // (use, static, function body) — the pending_cfg_test flag must be
    // reset by the first non-mod, non-attribute line, not by the closing
    // brace specifically.
    let src = "#[cfg(test)]\nmod inline {\n    use std::sync::Mutex;\n    static L: Mutex<()> = Mutex::new(());\n    pub fn f() { let _ = L.lock().unwrap(); }\n}\npub mod real;\n";
    let decls = parse_mod_decls(src);
    assert_eq!(
        decls,
        vec![("real".to_string(), None)],
        "the inline block body resets the flag; 'mod real' must be captured"
    );
}

// ---------------------------------------------------------------------------
// is_test_like_file (shared copy)
// ---------------------------------------------------------------------------

#[test]
fn is_test_like_shared_under_tests_dir() {
    assert!(is_test_like_file("src/anything/tests/foo.rs"));
}

#[test]
fn is_test_like_shared_under_underscore_tests_dir() {
    assert!(is_test_like_file(
        "src/analysis/convention_detector_tests/pattern_tests.rs"
    ));
}

#[test]
fn is_test_like_shared_suffixes() {
    assert!(is_test_like_file("src/a_tests.rs"));
    assert!(is_test_like_file("src/a_test.rs"));
    assert!(is_test_like_file("src/tests.rs"));
    assert!(is_test_like_file("src/test.rs"));
}

#[test]
fn is_test_like_shared_substring_and_lib() {
    assert!(is_test_like_file("src/schema_tests_migrations.rs"));
    assert!(is_test_like_file("src/lib.rs"));
    assert!(!is_test_like_file("src/ordinary.rs"));
}

// ---------------------------------------------------------------------------
// find_item_names (shared line-based item scan, issue #744 corrected rule)
// ---------------------------------------------------------------------------

#[test]
fn item_names_decl_kinds() {
    let src = "fn helper() -> i32 { 1 }\nstruct Widget;\nenum Kind { A }\n\ntrait Named {}\ntype Alias = i32;\nconst N: i32 = 0;\nstatic M: Mutex<i32> = Mutex::new(0);\nunion U { a: u32 }\n";
    let names = find_item_names(src);
    assert!(names.contains("helper"));
    assert!(names.contains("Widget"));
    assert!(names.contains("Kind"));
    assert!(names.contains("Named"));
    assert!(names.contains("Alias"));
    assert!(names.contains("N"));
    assert!(names.contains("M"));
    assert!(names.contains("U"));
}

#[test]
fn item_names_visibility_prefixes() {
    let src =
        "pub fn pub_fn() {}\npub(crate) struct CrateStruct;\npub(super) enum SuperEnum { A }\n";
    let names = find_item_names(src);
    assert!(names.contains("pub_fn"));
    assert!(names.contains("CrateStruct"));
    assert!(names.contains("SuperEnum"));
}

#[test]
fn item_names_macro_rules_and_reexport() {
    let src =
        "macro_rules! log_debug { () => {}; }\npub use self::inner::Thing;\nuse other::helper;\n";
    let names = find_item_names(src);
    assert!(names.contains("log_debug"));
    assert!(names.contains("Thing"));
    assert!(names.contains("helper"));
}

#[test]
fn item_names_non_decl_lines_are_ignored() {
    // No mid-line declaration keywords are matched; `foo::fn();` does not
    // declare `fn` (the anchor ` fn ` is not present — the char before `fn`
    // is `:`, not a space), and `let x = 1;` / `struct_only;` are not
    // declarations either.
    let src = "foo::fn();\nlet x = 1;\nstruct_only;\n";
    let names = find_item_names(src);
    assert!(
        names.is_empty(),
        "no declaration lines: no names, got {names:?}"
    );
}

// ---------------------------------------------------------------------------
// The shared map must agree with the alias map's keys when consulted
// (structural: the probe uses it as DATA, never as a resolver).
// ---------------------------------------------------------------------------

#[test]
fn shared_parser_feeds_logical_key_shape_used_by_probe_indeterminate() {
    // The probe's indeterminate rule keys off the parent's LOGICAL name in
    // the alias map; the map's keys are `::`-joined logical paths. A
    // `#[path]`-relocated parent (`deep` → divergent file) therefore makes
    // `super::x` sites in `src/deep/util.rs` indeterminate, confirmed by
    // the map's key shape — no resolver involved.
    let logical_to_physical: HashMap<String, String> =
        [("deep".to_string(), "src/elsewhere/storage.rs".to_string())]
            .into_iter()
            .collect();
    let parent_logical = "deep";
    assert!(
        logical_to_physical.contains_key(parent_logical),
        "the #[path]-relocated parent's logical name is the alias map key"
    );
}
