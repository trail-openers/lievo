use super::*;
use std::fs;
use tree_sitter::Parser;

#[test]
fn test_language_detection() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    assert!(extractor.get_language(Path::new("test.rs")).is_some());
    assert!(extractor.get_language(Path::new("test.py")).is_some());
    assert!(extractor.get_language(Path::new("test.js")).is_some());
    assert!(extractor.get_language(Path::new("test.ts")).is_some());
    assert!(extractor.get_language(Path::new("test.go")).is_some());
}

#[test]
fn test_index_dir_outside_repo() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = temp.path();

    // Create a dummy Rust file
    fs::write(
        repo_path.join("test.rs"),
        r#"fn test() { println!("hello"); }"#,
    )
    .unwrap();

    let mut extractor = TreeSitterExtractor::new(repo_path, false).unwrap();
    extractor.index(false).unwrap();

    let index_dir = extractor.index_dir().expect("index_dir should be set");

    // Index dir should NOT be inside the repo
    assert!(
        !index_dir.starts_with(repo_path),
        "index_dir {:?} should not start with repo_path {:?}",
        index_dir,
        repo_path
    );

    // Verify .lievo-ts-index was not created in the repo
    assert!(
        !repo_path.join(".lievo-ts-index").exists(),
        ".lievo-ts-index should not exist in repo"
    );
}

fn parse_js(source: &str) -> tree_sitter::Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}

fn parse_rust(source: &str) -> tree_sitter::Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}

/// Find the first node of the given kind in the tree.
fn find_node<'a>(root: tree_sitter::Node<'a>, kind: &'a str) -> Option<tree_sitter::Node<'a>> {
    if root.kind() == kind {
        return Some(root);
    }
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if let Some(found) = find_node(child, kind) {
            return Some(found);
        }
    }
    None
}

/// Extract all function-unit names for a JS/TS source string.
fn js_unit_names(source: &str) -> Vec<String> {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let tree = parse_js(source);
    let mut units = Vec::new();
    let file_imports = extractor.extract_imports_from_file(tree.root_node(), source, "JavaScript");
    extractor.extract_functions_from_node(
        tree.root_node(),
        source,
        "JavaScript",
        "test.js",
        &file_imports,
        &mut units,
    );
    extractor.extract_wrapped_default_export(
        tree.root_node(),
        source,
        "JavaScript",
        "test.js",
        &file_imports,
        &mut units,
    );
    units.into_iter().map(|u| u.name).collect()
}

/// Extract all function-unit names for a TypeScript source string
/// (issue #701: wrapped-export forms must work in .ts/.tsx too).
fn ts_unit_names(source: &str) -> Vec<String> {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let tree = parse_js(source);
    let mut units = Vec::new();
    let file_imports = extractor.extract_imports_from_file(tree.root_node(), source, "TypeScript");
    extractor.extract_functions_from_node(
        tree.root_node(),
        source,
        "TypeScript",
        "test.ts",
        &file_imports,
        &mut units,
    );
    extractor.extract_wrapped_default_export(
        tree.root_node(),
        source,
        "TypeScript",
        "test.ts",
        &file_imports,
        &mut units,
    );
    units.into_iter().map(|u| u.name).collect()
}

/// Extract all function-unit names for a JS/TS source string, using the
/// given repo-relative file path (affects `export default` naming).
fn js_unit_names_for(source: &str, file_path: &str) -> Vec<String> {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let tree = parse_js(source);
    let mut units = Vec::new();
    let file_imports = extractor.extract_imports_from_file(tree.root_node(), source, "JavaScript");
    extractor.extract_functions_from_node(
        tree.root_node(),
        source,
        "JavaScript",
        file_path,
        &file_imports,
        &mut units,
    );
    extractor.extract_wrapped_default_export(
        tree.root_node(),
        source,
        "JavaScript",
        file_path,
        &file_imports,
        &mut units,
    );
    units.into_iter().map(|u| u.name).collect()
}

#[test]
fn test_js_arrow_function_named_after_binding() {
    let source = "const double = (x) => x * 2;\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["double".to_string()],
        "arrow function should be named after its const binding, not its parameter"
    );
}

#[test]
fn test_js_arrow_function_destructured_param() {
    let source = "const Pagination = ({ page }) => page + 1;\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["Pagination".to_string()],
        "destructured-param arrow must be named after its binding, not discarded"
    );
}

#[test]
fn test_js_arrow_function_no_params() {
    let source = "const noArg = () => 42;\n";
    let names = js_unit_names(source);
    assert_eq!(names, vec!["noArg".to_string()]);
}

#[test]
fn test_js_arrow_function_named_param_not_misnamed() {
    let source = "const increment = (prev) => prev + 1;\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["increment".to_string()],
        "must be named after the binding, not the parameter 'prev'"
    );
}

#[test]
fn test_js_anonymous_arrow_discarded() {
    // Inline callback: no enclosing variable_declarator → must be discarded, not misnamed
    let source = "const arr = [1, 2, 3];\narr.map((x) => x * 2);\n";
    let names = js_unit_names(source);
    assert!(
        !names.contains(&"x".to_string()),
        "anonymous arrow must not be named after its parameter, got: {:?}",
        names
    );
}

#[test]
fn test_js_function_declaration_name_unchanged() {
    let source = "function namedFn(a, b) { return a + b; }\n";
    let names = js_unit_names(source);
    assert_eq!(names, vec!["namedFn".to_string()]);
}

// ------------------------------------------------------------------
// Wrapped-export extraction (issue #701)
// ------------------------------------------------------------------

#[test]
fn test_js_wrapped_styled_member_tagged_template() {
    let source = "export const Button = styled.div`color: red;`\n";
    assert_eq!(
        js_unit_names(source),
        vec!["Button".to_string()],
        "styled.div tagged template must yield a unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_styled_call_tagged_template() {
    let source = "export const Icon = styled(Span)`color: blue;`\n";
    assert_eq!(
        js_unit_names(source),
        vec!["Icon".to_string()],
        "styled(...) call-tagged template must yield a unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_forward_ref_arrow() {
    let source = "export const Fwd = forwardRef((props, ref) => null);\n";
    assert_eq!(
        js_unit_names(source),
        vec!["Fwd".to_string()],
        "forwardRef(arrow) must yield exactly one unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_observer_arrow() {
    let source = "export const Obs = observer((props) => null);\n";
    assert_eq!(js_unit_names(source), vec!["Obs".to_string()]);
}

#[test]
fn test_js_wrapped_memo_arrow() {
    let source = "export const Mem = memo(() => null);\n";
    assert_eq!(js_unit_names(source), vec!["Mem".to_string()]);
}

#[test]
fn test_js_wrapped_with_hoc_identifier() {
    let source = "export const With = withTheme(Base);\n";
    assert_eq!(
        js_unit_names(source),
        vec!["With".to_string()],
        "with[A-Z] HOC wrapping an identifier must yield a unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_function_expression_arg() {
    let source = "export const Fn = memo(function(props) { return null; });\n";
    assert_eq!(
        js_unit_names(source),
        vec!["Fn".to_string()],
        "memo(function expression) must yield a unit named after the binding, not 'props'"
    );
}

#[test]
fn test_js_wrapped_nested_memo_forward_ref_single_unit() {
    let source = "export const Nested = memo(forwardRef((props, ref) => null));\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["Nested".to_string()],
        "nested wrappers must yield exactly ONE unit named after the binding, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_nested_wrapper_function_expression_single_unit() {
    let source = "export const N2 = memo(forwardRef(function(props, ref) { return null; }));\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["N2".to_string()],
        "nested wrappers around a function expression must yield exactly ONE unit, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_non_matching_exported_call_excluded() {
    let source = "export const No = compute(1);\n";
    assert!(
        js_unit_names(source).is_empty(),
        "exported const with a plain non-wrapping call must NOT yield a unit"
    );
}

#[test]
fn test_js_wrapped_separate_form_variants_yield_unit() {
    // Acceptance criterion names "React.memo, React.forwardRef" in the
    // separate-export form (issue #791) alongside the bare and nested forms.
    for src in [
        "const Card = observer((props) => props.value);\nexport default Card;\n",
        "const Card = memo((props) => props.value);\nexport default Card;\n",
        "const Card = forwardRef((props, ref) => null);\nexport default Card;\n",
        "const Card = memo(forwardRef((props, ref) => null));\nexport default Card;\n",
        "const Card = React.memo(() => null);\nexport default Card;\n",
        "const Card = React.forwardRef((props, ref) => null);\nexport default Card;\n",
    ] {
        assert_eq!(js_unit_names(src), vec!["Card".to_string()]);
    }
}

// Wrapped bindings are extracted regardless of export (issue #791).
#[test]
fn test_js_wrapped_binding_without_export_yields_unit() {
    let source = "const No = memo(() => null);\n";
    assert_eq!(
        js_unit_names(source),
        vec!["No".to_string()],
        "non-exported wrapped binding must yield a unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_styled_binding_without_export_yields_unit() {
    let source = "const No = styled.div`color: red;`\n";
    assert_eq!(
        js_unit_names(source),
        vec!["No".to_string()],
        "non-exported styled binding must yield a unit named after the binding"
    );
}

#[test]
fn test_js_wrapped_non_exported_plain_arrow_binding_unchanged() {
    // Plain arrow bindings are still extracted (the #677 rule).
    let source = "const helper = (x) => x + 1;\n";
    assert_eq!(
        js_unit_names(source),
        vec!["helper".to_string()],
        "non-exported plain arrow bindings keep their unit"
    );
}

#[test]
fn test_js_module_level_timer_with_inline_fn_not_extracted() {
    // Timer/promise callees are in the non-component blacklist (issue #791).
    for source in [
        "const timeoutId = setTimeout(() => {}, 100);\n",
        "const timer = setInterval(() => {}, 1000);\n",
        "const p = Promise.resolve((() => 1)());\n",
    ] {
        let names = js_unit_names(source);
        assert!(
            names.is_empty(),
            "timer/promise binding must NOT yield a unit, got: {:?}",
            names
        );
    }
}

#[test]
fn test_js_wrapped_function_local_binding_excluded() {
    let source =
        "function f() {\n  const Inner = memo(() => null);\n  return Inner;\n}\nexport { f };\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["f".to_string()],
        "module-scoped-only wrapping: a function-local binding must not yield a unit, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_default_forward_ref_named_after_file_stem() {
    let source = "export default forwardRef((props, ref) => null);\n";
    let names = js_unit_names_for(source, "components/Card/card.js");
    assert_eq!(names, vec!["card".to_string()]);
}

#[test]
fn test_js_wrapped_default_styled_named_after_file_stem() {
    let source = "export default styled.div`color: red;`\n";
    let names = js_unit_names_for(source, "components/Card/card.js");
    assert_eq!(names, vec!["card".to_string()]);
}

#[test]
fn test_js_wrapped_default_with_hoc_named_after_file_stem() {
    let source = "export default withTheme(Base);\n";
    let names = js_unit_names_for(source, "components/Card/card.js");
    assert_eq!(names, vec!["card".to_string()]);
}

#[test]
fn test_js_wrapped_default_memo_named_after_file_stem() {
    let source = "export default memo(() => null);\n";
    let names = js_unit_names_for(source, "components/Card/card.js");
    assert_eq!(names, vec!["card".to_string()]);
}

#[test]
fn test_js_wrapped_default_index_named_after_parent_dir() {
    let source = "export default forwardRef((props, ref) => null);\n";
    let names = js_unit_names_for(source, "components/Button/index.js");
    assert_eq!(
        names,
        vec!["Button".to_string()],
        "index.js stem must take the parent directory name"
    );
}

#[test]
fn test_js_wrapped_default_styled_stem_named_after_parent_dir() {
    let source = "export default styled.div`color: red;`\n";
    let names = js_unit_names_for(source, "components/Button/styled.js");
    assert_eq!(
        names,
        vec!["Button".to_string()],
        "styled.js stem must take the parent directory name"
    );
}

#[test]
fn test_js_wrapped_default_non_wrapped_form_excluded() {
    let source = "export default (x) => x;\n";
    assert!(
        js_unit_names(source).is_empty(),
        "export default of a bare arrow (no wrapper call/styled tag) must NOT yield a unit"
    );
}

#[test]
fn test_js_wrapped_default_identifier_excluded() {
    let source = "export default SomeComp;\n";
    assert!(js_unit_names(source).is_empty());
}

#[test]
fn test_js_wrapped_unit_spans_full_statement_and_carries_calls() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let source = "import styled from 'styled-components';\nexport const Button = styled.div`color: red;`\nexport const Fwd = forwardRef((props) => props.onClick());\n";
    let tree = parse_js(source);
    let mut units = Vec::new();
    let file_imports = extractor.extract_imports_from_file(tree.root_node(), source, "JavaScript");
    extractor.extract_functions_from_node(
        tree.root_node(),
        source,
        "JavaScript",
        "Button/index.js",
        &file_imports,
        &mut units,
    );
    extractor.extract_wrapped_default_export(
        tree.root_node(),
        source,
        "JavaScript",
        "Button/index.js",
        &file_imports,
        &mut units,
    );
    assert_eq!(units.len(), 2, "expected exactly two units");
    let button = units
        .iter()
        .find(|u| u.name == "Button")
        .expect("Button unit");
    assert!(
        button
            .code
            .as_deref()
            .unwrap_or("")
            .starts_with("const Button"),
        "unit code must span the full binding statement, got: {:?}",
        button.code
    );
    assert_eq!(button.file, "Button/index.js");
    assert_eq!(button.language, "JavaScript");
    let fwd = units.iter().find(|u| u.name == "Fwd").expect("Fwd unit");
    assert!(
        fwd.calls.iter().any(|c| c == "onClick"),
        "wrapped unit must carry calls extracted from the initializer, got: {:?}",
        fwd.calls
    );
}

#[test]
fn test_js_wrapped_plain_functions_and_arrows_unchanged() {
    let source = "function plain() {\n  return 1;\n}\nconst arrow = (x) => x * 2;\nexport const arrow2 = arrow;\n";
    let names = js_unit_names(source);
    assert!(
        names.contains(&"plain".to_string()),
        "plain function must still be extracted"
    );
    assert!(
        names.contains(&"arrow".to_string()),
        "plain arrow binding must still be extracted"
    );
    assert!(
        !names.contains(&"arrow2".to_string()),
        "export const X = identifier (no wrapping) must NOT yield a unit"
    );
}

#[test]
fn test_js_method_definition_name_unchanged() {
    let source = "class C { m() { return 1; } }\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["m".to_string()],
        "method should be named 'm', not a parameter"
    );
}

#[test]
fn test_rust_function_item_name_unchanged() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let tree = parse_rust("fn hello() { println!(\"hi\"); }\n");
    let node = find_node(tree.root_node(), "function_item").unwrap();
    let name = extractor.extract_function_name(node, "fn hello() { println!(\"hi\"); }\n", "Rust");
    assert_eq!(name.as_deref(), Some("hello"));
}

#[test]
fn test_rust_import_argument_field() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let source = "use std::collections::HashMap;\nuse crate::rust_helper::helper_label;\n";
    let tree = parse_rust(source);
    let imports = extractor.extract_imports_from_file(tree.root_node(), source, "Rust");
    assert!(
        imports.contains(&"std::collections::HashMap".to_string()),
        "Rust import should emit the argument field, got: {:?}",
        imports
    );
    assert!(
        imports.contains(&"crate::rust_helper::helper_label".to_string()),
        "Rust import should emit the argument field, got: {:?}",
        imports
    );
    // Must NOT contain whole-statement text
    assert!(
        !imports
            .iter()
            .any(|i| i.starts_with("use ") || i.contains(";")),
        "Rust imports must not contain whole-statement text, got: {:?}",
        imports
    );
}

#[test]
fn test_js_import_source_field() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let source = "import { readFileSync } from 'fs';\nimport path from \"path\";\n";
    let tree = parse_js(source);
    let imports = extractor.extract_imports_from_file(tree.root_node(), source, "JavaScript");
    assert!(
        imports.contains(&"fs".to_string()),
        "JS import should emit the source field without quotes, got: {:?}",
        imports
    );
    assert!(
        imports.contains(&"path".to_string()),
        "JS import should emit the source field without quotes, got: {:?}",
        imports
    );
    // Must NOT contain whole-statement text or quotes
    assert!(
        !imports
            .iter()
            .any(|i| i.contains("import ") || i.contains('\'') || i.contains('\"')),
        "JS imports must not contain whole-statement text or quotes, got: {:?}",
        imports
    );
}

#[test]
fn test_ts_import_source_field_relative() {
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let source = "import { sumNumbers } from './ts_sample';\n";
    let tree = parse_js(source);
    let imports = extractor.extract_imports_from_file(tree.root_node(), source, "TypeScript");
    assert_eq!(
        imports,
        vec!["./ts_sample".to_string()],
        "relative TS import should emit the bare specifier"
    );
}

// Negative tests: inline callbacks must NOT be extracted (issue #791)

#[test]
fn test_js_inline_map_callback_not_extracted() {
    let source =
        "function handler(items) {\n  const mapped = items.map(x => x * 2);\n  return mapped;\n}\n";
    let names = js_unit_names(source);
    assert!(
        !names.iter().any(|n| n == "x" || n == "mapped"),
        "inline .map callback assigned to a local variable must NOT yield a unit, got: {:?}",
        names
    );
}

#[test]
fn test_js_module_level_map_callback_not_extracted() {
    // Module-level array/iter method callees (map/filter/reduce) are in the
    // non-component blacklist (issue #791).
    for source in [
        "const items = [1, 2, 3];\nconst mapped = items.map(x => x * 2);\n",
        "const items = [1, 2, 3];\nconst filtered = items.filter(x => x > 1);\n",
        "const items = [1, 2, 3];\nconst total = items.reduce((a, b) => a + b, 0);\n",
    ] {
        let names = js_unit_names(source);
        assert!(
            names.is_empty(),
            "module-level array/iter method binding must NOT yield a unit, got: {:?}",
            names
        );
    }
}

#[test]
fn test_js_inline_event_handler_not_extracted() {
    let source =
        "function Btn() {\n  return <button onClick={() => console.log('hi')}>Hi</button>\n}\n";
    let names = js_unit_names(source);
    assert_eq!(names, vec!["Btn".to_string()]);
}

// ------------------------------------------------------------------
// TypeScript/TSX wrapped-export extraction (issue #701)
// ------------------------------------------------------------------

#[test]
fn test_ts_wrapped_styled_member_tagged_template() {
    let source = "export const Badge = styled.span`font-size: 12px;`\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["Badge".to_string()],
        "TS styled.div tagged template must yield a unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_styled_call_tagged_template() {
    let source = "export const Link = styled(Badge)`color: green;`\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["Link".to_string()],
        "TS styled(...) call-tagged template must yield a unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_forward_ref_arrow() {
    let source = "export const Fwd = forwardRef((props, ref) => props.href);\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["Fwd".to_string()],
        "TS forwardRef(arrow) must yield exactly one unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_observer_arrow() {
    let source = "export const Obs = observer((props) => 0);\n";
    assert_eq!(ts_unit_names(source), vec!["Obs".to_string()]);
}

#[test]
fn test_ts_wrapped_memo_function_expression() {
    let source = "export const Mem = memo(function (props) { return props.items.length; });\n";
    assert_eq!(ts_unit_names(source), vec!["Mem".to_string()]);
}

#[test]
fn test_ts_wrapped_with_hoc_identifier() {
    let source = "export const Themed = withTheme(Badge);\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["Themed".to_string()],
        "TS with[A-Z] HOC wrapping an identifier must yield a unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_nested_memo_forward_ref_single_unit() {
    let source =
        "export const Nested = memo(forwardRef((props: { deep: boolean }, ref) => props.deep));\n";
    let names = ts_unit_names(source);
    assert_eq!(
        names,
        vec!["Nested".to_string()],
        "TS nested wrappers must yield exactly ONE unit named after the binding, got: {:?}",
        names
    );
}

#[test]
fn test_ts_wrapped_non_exported_memo_yields_unit() {
    let source = "const Card = memo((props) => props.value);\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["Card".to_string()],
        "TS non-exported wrapped binding must yield a unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_plain_functions_unchanged() {
    let source =
        "function plain(): number { return 1; }\nconst arrow = (x: number): number => x * 2;\n";
    let names = ts_unit_names(source);
    assert!(
        names.contains(&"plain".to_string()),
        "TS plain function must still be extracted"
    );
    assert!(
        names.contains(&"arrow".to_string()),
        "TS plain arrow binding must still be extracted (the #677 rule)"
    );
}

// Incremental re-index: unchanged zero-unit files stay reported (issue #701)

#[test]
fn test_incremental_reindex_keeps_zero_unit_files_in_extracted_files() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    fs::write(
        repo.join("wrapped.js"),
        "export const A = memo(() => 1);\n", // one unit
    )
    .unwrap();
    fs::write(repo.join("empty.js"), "const x = 1;\n").unwrap(); // zero units
    let mut extractor = TreeSitterExtractor::new(repo, false).unwrap();

    // First index: everything is new, both files must be reported as scanned.
    extractor.index(false).unwrap();
    let first = extractor.extracted_files().to_vec();
    assert!(
        first.iter().any(|p| p == "empty.js"),
        "first index must report the zero-unit file, got: {:?}",
        first
    );

    // Simulate the pipeline's hand-off: the first index's per-file hashes
    // become the next run's known hashes.
    extractor.set_last_hash_state();

    // Second (incremental) index with unchanged files: no re-parse, but
    // the zero-unit file must STILL be reported for grouping to seed its
    // file entity.
    extractor.index(false).unwrap();
    let second = extractor.extracted_files().to_vec();
    assert!(
        second.iter().any(|p| p == "empty.js"),
        "incremental re-index must still report the zero-unit file, got: {:?}",
        second
    );
    assert!(
        second.iter().any(|p| p == "wrapped.js"),
        "incremental re-index must still report the unit-bearing file, got: {:?}",
        second
    );

    // The unchanged files were not re-parsed: the incremental index
    // re-records their paths (the zero-unit seam) without producing
    // units in-memory (units persist via storage, mirroring the
    // pipeline's hash-skip behavior).
    assert!(
        extractor.files_parsed().is_empty(),
        "no re-parse on incremental index"
    );
}
