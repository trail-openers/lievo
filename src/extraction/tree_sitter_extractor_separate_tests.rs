// Issue #791: separate-export / assignment-keyed rule tests.
//
// Extracted from `tree_sitter_extractor_tests.rs` to keep that file under
// the 800-line test budget (issue #702 file-size policy).

use super::*;
use tree_sitter::Parser;

/// Extract all function-unit names for a JavaScript source string.
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

/// Extract all function-unit names for a TypeScript source string.
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

/// Parse a JavaScript/TypeScript source string into a tree-sitter tree.
fn parse_js(source: &str) -> tree_sitter::Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}

// ------------------------------------------------------------------
// JS separate-form fixtures
// ------------------------------------------------------------------

// The wrapped-binding gate matches the module-level invariant on the tree's
// root node kind ("program"). Pin that kind: if the JS/TS grammar is ever
// swapped for one with a different root node name, the gate would silently
// stop matching module-level bindings — this test fails loudly instead.
#[test]
fn test_js_root_node_kind_is_program() {
    let tree = parse_js("const x = 1;\n");
    assert_eq!(
        tree.root_node().kind(),
        "program",
        "JS/TS grammar root node kind must stay \"program\" (the wrapped-binding gate matches on it)"
    );
}

#[test]
fn test_js_wrapped_separate_export_observer() {
    let source = "const Obs = observer(() => null);\nexport default Obs;\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["Obs".to_string()],
        "separate form must yield exactly ONE unit named after the binding, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_separate_export_nested_memo_forward_ref_single_unit() {
    let source = "const Nested = memo(forwardRef((props, ref) => null));\nexport default Nested;\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["Nested".to_string()],
        "separate-form nested wrappers must yield exactly ONE unit, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_separate_export_styled() {
    let source = "const Btn = styled.div`color: red;`\nexport default Btn;\n";
    assert_eq!(js_unit_names(source), vec!["Btn".to_string()]);
}

#[test]
fn test_js_wrapped_separate_export_with_hoc_identifier() {
    let source = "const With = withTheme(Base);\nexport default With;\n";
    assert_eq!(js_unit_names(source), vec!["With".to_string()]);
}

#[test]
fn test_js_wrapped_never_exported_extracted() {
    // A wrapped component used only within its own module is still a
    // function worth indexing (issue #791: export presence is irrelevant).
    let source = "const Hidden = observer(() => null);\nfunction useIt() { return Hidden; }\nexport { useIt };\n";
    let names = js_unit_names(source);
    assert!(
        names.contains(&"Hidden".to_string()),
        "never-exported wrapped binding must yield a unit, got: {:?}",
        names
    );
    assert!(
        names.contains(&"useIt".to_string()),
        "the regular function must still be extracted, got: {:?}",
        names
    );
}

#[test]
fn test_js_wrapped_unit_spans_full_binding_statement_for_separate_form() {
    // Span re-validation (issue #791 pitfall): under the assignment-keyed
    // rule the wrapped declarator's grandparent is the plain
    // `lexical_declaration`, so the unit's code must cover the
    // `const X = ...` statement — not the whole file, and not just the
    // declarator.
    let extractor = TreeSitterExtractor::new(Path::new("."), false).unwrap();
    let source = "const Card = observer((props) => {\n    const total = props.total;\n    return total;\n});\nexport default Card;\n";
    let tree = parse_js(source);
    let mut units = Vec::new();
    let file_imports = extractor.extract_imports_from_file(tree.root_node(), source, "JavaScript");
    extractor.extract_functions_from_node(
        tree.root_node(),
        source,
        "JavaScript",
        "Card/index.js",
        &file_imports,
        &mut units,
    );
    assert_eq!(
        units.len(),
        1,
        "expected exactly one unit, got: {:?}",
        units
    );
    let unit = &units[0];
    assert_eq!(unit.name, "Card");
    assert_eq!(unit.file, "Card/index.js");
    let code = unit.code.as_deref().unwrap_or("");
    assert!(
        code.starts_with("const Card = observer"),
        "unit code must span the full const declaration, got: {:?}",
        unit.code
    );
    assert!(
        !code.contains("export default"),
        "the separate export statement must NOT be part of the span, got: {:?}",
        unit.code
    );
    // The span must cover the whole declaration — a multi-line component
    // body must not be clipped to the first line.
    let (start, end) = (unit.line, unit.end_line);
    assert!(
        end > start,
        "unit must span its full declaration, got {}..{}",
        start,
        end
    );
}

// Inline callbacks are NOT components: an inline arrow passed to a
// non-wrapper call (arr.map, Promise.then, etc.) is a handler/iterator
// callback, not a wrapped component. The assignment-keyed gate must keep
// the module-level component distinction (issue #791 over-extraction
// constraint). The tightened catch-all also pins `React.memo` /
// `React.forwardRef` as recognized wrappers in the separate form.
//
// Note: a plain arrow assigned to a binding (`const handler = () => …`) is
// still extracted under the #677 rule — the negative constraint applies to
// arrows *passed to non-wrapper calls*, not to top-level bindings.
#[test]
fn test_js_wrapped_inline_callback_not_extracted() {
    // Inline event-handler / iterator-callback shapes: none of these are
    // wrapped-component bindings and none may yield a unit.
    let source = "const items = [1, 2, 3];\nconst mapped = items.map((x) => x * 2);\nconst thened = Promise.resolve(1).then((x) => x);\n";
    let names = js_unit_names(source);
    let banned = ["mapped", "thened", "x"];
    for banned in banned {
        assert!(
            !names.contains(&banned.to_string()),
            "inline callback binding '{}' must NOT yield a unit, got: {:?}",
            banned,
            names
        );
    }
}

#[test]
fn test_js_wrapped_react_member_wrapper_separate_form() {
    // `React.memo` / `React.forwardRef` in the separate form (issue #791
    // acceptance criterion: the same for React.memo, React.forwardRef).
    let source = "const RM = React.memo((props) => null);\nexport default RM;\nconst RF = React.forwardRef((props, ref) => null);\nexport { RF };\n";
    let names = js_unit_names(source);
    assert_eq!(
        names,
        vec!["RM".to_string(), "RF".to_string()],
        "React.memo / React.forwardRef separate form must yield exactly one unit per binding, got: {:?}",
        names
    );
}

// Round-2 precision pins: the gate must NOT widen to (a) destructured
// bindings, whose `name` field is the pattern text and would yield a unit
// named `"{ a, b }"`, or (b) member-expression callees other than the
// documented `React.*` form (`foo.memo` / `arr.withFoo` are not React
// component wrappers).
#[test]
fn test_js_wrapped_destructured_binding_not_extracted() {
    let source = "const { a, b } = memo((p) => null);\nconst [first] = observer((p) => null);\n";
    let names = js_unit_names(source);
    for banned in ["{ a, b }", "[first]", "a", "b", "first"] {
        assert!(
            !names.contains(&banned.to_string()),
            "destructured binding '{}' must NOT yield a unit, got: {:?}",
            banned,
            names
        );
    }
}

#[test]
fn test_js_wrapped_non_react_member_callee_not_extracted() {
    // `foo.memo` / `arr.withFoo` / `ns.withTheme` are not the documented
    // `React.*` wrappers: the member arm must require the object to be
    // `React`, not just the property to be a wrapper name.
    let source = "const FM = foo.memo((p) => null);\nconst AW = arr.withTheme((p) => null);\n";
    let names = js_unit_names(source);
    for banned in ["FM", "AW"] {
        assert!(
            !names.contains(&banned.to_string()),
            "non-React member callee binding '{}' must NOT yield a unit, got: {:?}",
            banned,
            names
        );
    }
}

// ------------------------------------------------------------------
// TS separate-form fixtures
// ------------------------------------------------------------------

// Repurposed from the #701 `non_exported_*` negative tests: the #791
// assignment-keyed rule yields a unit for non-exported wrapped bindings.
#[test]
fn test_ts_wrapped_non_exported_memo_extracted() {
    let source = "const No = memo(() => null);\n";
    assert_eq!(
        ts_unit_names(source),
        vec!["No".to_string()],
        "TS non-exported wrapped binding must yield a unit named after the binding"
    );
}

#[test]
fn test_ts_wrapped_separate_export_observer() {
    let source = "const Obs = observer((props) => props.value);\nexport default Obs;\n";
    let names = ts_unit_names(source);
    assert_eq!(
        names,
        vec!["Obs".to_string()],
        "TS separate form must yield exactly ONE unit named after the binding, got: {:?}",
        names
    );
}

#[test]
fn test_ts_wrapped_separate_export_nested_memo_forward_ref_single_unit() {
    let source =
        "const Nested = memo(forwardRef((props, ref) => props.deep));\nexport default Nested;\n";
    let names = ts_unit_names(source);
    assert_eq!(
        names,
        vec!["Nested".to_string()],
        "TS separate-form nested wrappers must yield exactly ONE unit, got: {:?}",
        names
    );
}
