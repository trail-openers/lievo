/// Tree-sitter node metrics and complexity analysis helpers.
use tree_sitter::Node;

const BRANCH_KINDS: &[&str] = &[
    "if_expression",
    "if_statement",
    "match_expression",
    "switch_statement",
];

const LOOP_KINDS: &[&str] = &[
    "for_expression",
    "for_statement",
    "for_in_statement",
    "while_expression",
    "while_statement",
    "loop_expression",
];

/// Metrics collected in a single pass through the AST.
pub struct FunctionMetrics {
    pub complexity: i64,
    pub has_branches: bool,
    pub has_loops: bool,
    pub has_error_handling: bool,
}

/// Collect all metrics in a single pass through the AST.
pub fn collect_metrics(node: Node, language: &str) -> FunctionMetrics {
    let mut branch_count = 0i64;
    let mut loop_count = 0i64;
    let mut error_count = 0i64;

    collect_recursive(
        node,
        language,
        &mut branch_count,
        &mut loop_count,
        &mut error_count,
    );

    FunctionMetrics {
        complexity: 1 + branch_count + loop_count,
        has_branches: branch_count > 0,
        has_loops: loop_count > 0,
        has_error_handling: error_count > 0,
    }
}

fn collect_recursive(
    node: Node,
    language: &str,
    branches: &mut i64,
    loops: &mut i64,
    errors: &mut i64,
) {
    let kind = node.kind();

    if BRANCH_KINDS.contains(&kind) {
        *branches += 1;
    }
    if LOOP_KINDS.contains(&kind) {
        *loops += 1;
    }

    let error_kinds: &[&str] = match language {
        "Rust" => &["match_expression"],
        "Python" => &["try_statement", "except_clause"],
        "JavaScript" | "TypeScript" => &["try_statement", "catch_clause"],
        "Go" => &["if_statement"],
        _ => &[],
    };
    if error_kinds.contains(&kind) {
        *errors += 1;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_recursive(child, language, branches, loops, errors);
    }
}
