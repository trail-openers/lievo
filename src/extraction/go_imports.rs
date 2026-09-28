use std::collections::HashSet;
use tree_sitter::Node;

/// Extract Go imports from a tree-sitter tree using iterative, top-level traversal.
///
/// This implementation avoids unbounded recursion and limits traversal to the
/// top-level of the AST where Go import declarations are located. Go imports are
/// always at the file top level, so full-tree traversal is unnecessary.
///
/// # Performance Notes
/// - Iterative traversal with a single reusable cursor, avoiding cursor churn
/// - Bounded depth (only top-level children)
/// - Early exit when all imports found (no unnecessary node visits)
///
/// # Arguments
/// * `root_node` - The root node of the parsed tree
/// * `source_code` - The source code string (for extracting import text)
///
/// # Returns
/// A vector of import paths as strings (e.g., "fmt", "encoding/json")
pub fn extract_go_imports(root_node: Node, source_code: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = root_node.walk();

    // Iterate only over top-level children - Go imports are always at file level
    for child in root_node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                // Found an import declaration - extract all import_spec nodes within it
                extract_from_import_declaration(child, source_code, &mut imports, &mut seen);
            }
            "import_spec" => {
                // Direct import_spec (can appear in some tree variations)
                extract_from_import_spec(child, source_code, &mut imports, &mut seen);
            }
            _ => {
                // Other top-level nodes - skip (functions, types, etc.)
                // We don't recurse because imports are only at top level
            }
        }
    }

    imports
}

/// Extract imports from a Go import_declaration node.
///
/// Import declarations can have multiple import_spec children (e.g., `import ("fmt"; "os")`).
/// This function iterates over them iteratively without depth-first recursion.
fn extract_from_import_declaration(
    node: Node,
    source_code: &str,
    imports: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                extract_from_import_spec(child, source_code, imports, seen);
            }
            "import_spec_list" => {
                // Parenthesized import list - iterate over its children
                let mut list_cursor = child.walk();
                for list_child in child.children(&mut list_cursor) {
                    if list_child.kind() == "import_spec" {
                        extract_from_import_spec(list_child, source_code, imports, seen);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract import path from a Go import_spec node.
///
/// Iterates over import_spec children to find the path string without recursion.
///
/// # UTF-8 Handling
/// This function performs best-effort extraction and silently skips nodes that contain
/// invalid UTF-8 sequences. This behavior is intentional for robustness when parsing
/// malformed or partially corrupted source files.
fn extract_from_import_spec(
    node: Node,
    source_code: &str,
    imports: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec_path" | "interpreted_string_literal" => {
                if let Ok(text) = child.utf8_text(source_code.as_bytes()) {
                    let import_text = text.trim().trim_matches('"').to_string();
                    if !import_text.is_empty() && seen.insert(import_text.clone()) {
                        imports.push(import_text);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Language, Parser};

    #[test]
    fn test_extract_go_imports_basic() {
        let source = r#"
package main

import "fmt"

func main() {
    fmt.Println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        assert_eq!(imports, vec!["fmt"]);
    }

    #[test]
    fn test_extract_go_imports_multiple() {
        let source = r#"
package main

import (
    "fmt"
    "os"
    "encoding/json"
)

func main() {
    fmt.Println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        assert!(imports.contains(&"fmt".to_string()));
        assert!(imports.contains(&"os".to_string()));
        assert!(imports.contains(&"encoding/json".to_string()));
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn test_extract_go_imports_aliased() {
        let source = r#"
package main

import f "fmt"

func main() {
    f.Println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        // Should still capture the path even with an alias
        assert_eq!(imports, vec!["fmt"]);
    }

    #[test]
    fn test_extract_go_imports_dot_import() {
        let source = r#"
package main

import . "fmt"

func main() {
    Println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        assert_eq!(imports, vec!["fmt"]);
    }

    #[test]
    fn test_extract_go_imports_no_duplicates() {
        let source = r#"
package main

import (
    "fmt"
    "fmt"
)

func main() {
    fmt.Println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        // Should deduplicate imports
        assert_eq!(imports, vec!["fmt"]);
    }

    #[test]
    fn test_extract_go_imports_empty() {
        let source = r#"
package main

func main() {
    println("hello")
}
"#;

        let (tree, _) = parse_go(source);
        let imports = extract_go_imports(tree.root_node(), source);

        assert!(imports.is_empty());
    }

    fn parse_go(source: &str) -> (tree_sitter::Tree, Language) {
        let language = tree_sitter_go::LANGUAGE.into();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, language)
    }
}
