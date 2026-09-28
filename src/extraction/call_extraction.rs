use tree_sitter::Node;

/// Extract function calls from a tree-sitter node for a specific language.
///
/// This function performs a depth-limited iterative traversal to find call nodes.
/// The traversal is bounded to avoid stack overflow on deep ASTs.
///
/// # Depth Limitation
/// Traversal is capped at 100 levels deep. Beyond this depth, subtrees are silently
/// truncated to avoid potential infinite loops or stack issues. This is an intentional
/// tradeoff: function calls beyond depth 100 are unlikely to represent meaningful
/// analysis targets in typical codebases. If you encounter truncated results, consider
/// refining the query scope to specific function nodes.
///
/// # Arguments
/// * `node` - The root node to search within (typically a function node)
/// * `source_code` - Source code string (for extracting call names)
/// * `language` - Programming language name ("Rust", "Python", "JavaScript", "TypeScript", "Go")
///
/// # Returns
/// A vector of function call names
pub fn extract_calls(node: Node, source_code: &str, language: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let call_kinds: &[&str] = match language {
        "Rust" | "JavaScript" | "TypeScript" | "Go" => &["call_expression"],
        "Python" => &["call"],
        _ => return calls,
    };

    // Iterative DFS with bounded depth (max 100 levels)
    let mut stack = vec![(node, 0)];

    while let Some((current, depth)) = stack.pop() {
        // Depth limit to prevent stack overflow - subtrees beyond depth 100 are
        // intentionally truncated. This is a bounded API; unbounded recursion would
        // risk stack overflow on pathological inputs. Calls at extreme depths are
        // unlikely to represent meaningful analysis targets.
        if depth >= 100 {
            continue;
        }

        // Check if this node is a call expression
        if call_kinds.contains(&current.kind())
            && let Some(name) = extract_call_name(current, source_code)
        {
            calls.push(name);
        }

        // Push children for DFS traversal
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push((child, depth + 1));
        }
    }

    calls
}

/// Extract the function name from a call node.
///
/// Handles:
/// - Direct function calls: `foo()`
/// - Member/field expressions: `obj.method()`, `obj.method.nested()`
/// - Selector expressions (Go): `obj.Method()`
/// - Attribute expressions (Python): `obj.method()`
///
/// For member/field/selector/attribute expressions the callee is the
/// property of the OUTERMOST expression (e.g. `a.b.c()` → `c`), so
/// `property_identifier`/`field_identifier`/`attribute` are preferred
/// over the receiver `identifier`.
///
/// # UTF-8 Handling
/// This function performs best-effort extraction and silently skips nodes that contain
/// invalid UTF-8 sequences. This behavior is intentional for robustness when parsing
/// malformed or partially corrupted source files.
///
/// # Arguments
/// * `call_node` - The call_expression node
/// * `source_code` - Source code string
///
/// # Returns
/// The call name if found, None otherwise
fn extract_call_name(call_node: Node, source_code: &str) -> Option<String> {
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        match child.kind() {
            "field_expression" | "member_expression" | "selector_expression" => {
                // Prefer the property/field name over the receiver:
                // member children are e.g. `[identifier g, property_identifier greet]`.
                // Return the property first; fall back to identifier for
                // languages whose member node carries the callee as identifier.
                let mut inner_cursor = child.walk();
                let mut fallback: Option<String> = None;
                for grandchild in child.children(&mut inner_cursor) {
                    match grandchild.kind() {
                        "property_identifier" | "field_identifier" => {
                            if let Ok(text) = grandchild.utf8_text(source_code.as_bytes()) {
                                return Some(text.to_string());
                            }
                        }
                        "identifier" if fallback.is_none() => {
                            fallback = grandchild
                                .utf8_text(source_code.as_bytes())
                                .ok()
                                .map(|s| s.to_string());
                        }
                        _ => {}
                    }
                }
                if let Some(name) = fallback {
                    return Some(name);
                }
            }
            "attribute" => {
                // Python `attribute` is a LEAF node: `obj.method()` parses to
                // one `attribute` node whose text is `obj.method`. The method
                // name is the last segment after the final dot.
                if let Ok(text) = child.utf8_text(source_code.as_bytes())
                    && let Some(dot) = text.rfind('.')
                {
                    let method = &text[dot + 1..];
                    if !method.is_empty() {
                        return Some(method.to_string());
                    }
                }
            }
            "identifier" => {
                return child
                    .utf8_text(source_code.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_calls_unknown_language() {
        // Unknown language should return empty calls
        use tree_sitter::Parser;
        let source = r#"fn main() { foo(); }"#;
        let language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();

        let calls = extract_calls(tree.root_node(), source, "UnknownLanguage");
        assert!(calls.is_empty());
    }

    fn calls_in_source(source: &str, language_name: &str) -> Vec<String> {
        use tree_sitter::Parser;
        let language = match language_name {
            "Rust" => tree_sitter_rust::LANGUAGE.into(),
            "Python" => tree_sitter_python::LANGUAGE.into(),
            "JavaScript" | "TypeScript" => tree_sitter_javascript::LANGUAGE.into(),
            "Go" => tree_sitter_go::LANGUAGE.into(),
            _ => panic!("unknown test language: {language_name}"),
        };
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_calls(tree.root_node(), source, language_name)
    }

    #[test]
    fn test_plain_call_returns_identifier() {
        let source = r#"function main() { foo(); }"#;
        let calls = calls_in_source(source, "JavaScript");
        assert_eq!(calls, vec!["foo"]);
    }

    #[test]
    fn test_js_member_call_returns_method_not_receiver() {
        let source = r#"function main() { const g = new Greeting(); g.greet(); }"#;
        let calls = calls_in_source(source, "JavaScript");
        // Member call yields the method name, never the receiver.
        assert_eq!(calls, vec!["greet"]);
    }

    #[test]
    fn test_js_chained_call_returns_outermost_method() {
        let source = r#"function main() { a.b.c(); }"#;
        let calls = calls_in_source(source, "JavaScript");
        // Chained member call: callee is the property of the OUTERMOST
        // member_expression.
        assert_eq!(calls, vec!["c"]);
    }

    #[test]
    fn test_rust_method_call_returns_field_identifier() {
        let source = r#"fn main() { let s = ""; s.len(); }"#;
        let calls = calls_in_source(source, "Rust");
        assert_eq!(calls, vec!["len"]);
    }

    #[test]
    fn test_python_attribute_call_returns_attribute_name() {
        let source = "def main():\n    obj.method()\n";
        let calls = calls_in_source(source, "Python");
        assert_eq!(calls, vec!["method"]);
    }

    #[test]
    fn test_go_selector_call_returns_field_identifier() {
        let source = "func main() { fmt.Println(1) }\n";
        let calls = calls_in_source(source, "Go");
        assert_eq!(calls, vec!["Println"]);
    }
}
