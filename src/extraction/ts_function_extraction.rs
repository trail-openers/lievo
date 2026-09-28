// Function extraction methods for TreeSitterExtractor, extracted to
// ts_function_extraction.rs to keep the main extractor file under the
// 500-line limit (issue #702). Pulled in via include! from the
// extractor module — same pattern as pipeline.rs/batch_ops.rs.

impl TreeSitterExtractor {
    fn extract_functions_from_node(
        &self,
        node: tree_sitter::Node,
        source_code: &str,
        language: &str,
        file_path: &str,
        file_imports: &[String],
        units: &mut Vec<CodeUnit>,
    ) {
        // Check if this node is a function (or, for JS/TS, a wrapped-component
        // declarator such as `const X = styled.div\`...\``).
        if self.is_function_node(node, language, source_code) {
            if let Some(unit) =
                self.extract_function_unit(node, source_code, language, file_path, file_imports)
            {
                units.push(unit);
            }
            // A wrapped-component unit spans its whole statement; recursing
            // into its children would re-extract the inner anonymous
            // arrow/function as a duplicate unit (issue #701). Plain function
            // declarations contain no matching children, so stopping only
            // affects wrapped components. The guard fires on the same
            // assignment-keyed rule as the unit itself (issue #791), keeping
            // both paths consistent.
            if self.is_wrapped_component_binding(node, source_code) {
                return;
            }
        }

        // Recursively process children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_functions_from_node(
                child,
                source_code,
                language,
                file_path,
                file_imports,
                units,
            );
        }
    }

    fn is_function_node(&self, node: tree_sitter::Node, language: &str, source_code: &str) -> bool {
        match language {
            "Rust" => node.kind() == "function_item",
            "Python" => node.kind() == "function_definition",
            "JavaScript" | "TypeScript" => {
                matches!(
                    node.kind(),
                    "function_declaration" | "method_definition" | "arrow_function"
                ) || self.is_wrapped_component_binding(node, source_code)
            }
            "Go" => node.kind() == "function_declaration",
            _ => false,
        }
    }

    fn extract_function_unit(
        &self,
        node: tree_sitter::Node,
        source_code: &str,
        language: &str,
        file_path: &str,
        file_imports: &[String],
    ) -> Option<CodeUnit> {
        let fn_name = self.extract_function_name(node, source_code, language)?;
        let span = if self.is_wrapped_component_binding(node, source_code) {
            // Unit spans the binding statement, not just the declarator.
            // The span is the const/let/var declaration itself (the
            // declarator's direct parent) — NOT the declaration's parent:
            // under the assignment-keyed rule that parent is either the
            // `export_statement` (inline form) or the plain `program` root
            // (separate form, where a grandparent hop would span the whole
            // file). A variable_declarator always has a declaration parent
            // in a healthy parse; the expect surfaces a malformed tree
            // rather than silently narrowing the span to the declarator.
            node.parent().expect(
                "variable_declarator must have a declaration parent",
            )
        } else {
            node
        };
        self.build_function_unit(
            span,
            &fn_name,
            source_code,
            language,
            file_path,
            file_imports,
        )
    }

    /// Build a CodeUnit for a function-shaped node (or, for wrapped JS/TS
    /// component bindings, the enclosing binding declaration) with an explicit
    /// name. The code/signature span the full statement so wrapped bindings
    /// like `const X = ...` carry the whole declaration (issue #701).
    fn build_function_unit(
        &self,
        node: tree_sitter::Node,
        fn_name: &str,
        source_code: &str,
        language: &str,
        file_path: &str,
        file_imports: &[String],
    ) -> Option<CodeUnit> {
        let byte_range = node.byte_range();
        let source_bytes = source_code.as_bytes();
        let start_byte = byte_range.start;
        let end_byte = byte_range.end;

        // Count newlines to approximate line numbers
        let start_line = source_bytes[..start_byte]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as i64
            + 1;
        let end_line = source_bytes[..end_byte]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as i64
            + 1;

        let signature = node.child(0).and_then(|c| {
            let text = c.utf8_text(source_code.as_bytes()).ok()?;
            Some(text.lines().next()?.to_string())
        });

        let code = Some(node.utf8_text(source_code.as_bytes()).ok()?.to_string());

        let calls = call_extraction::extract_calls(node, source_code, language);
        let metrics = ts_metrics::collect_metrics(node, language);
        let complexity = metrics.complexity;
        let has_branches = metrics.has_branches;
        let has_loops = metrics.has_loops;
        let has_error_handling = metrics.has_error_handling;

        Some(CodeUnit {
            name: fn_name.to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_string(),
            line: start_line,
            end_line,
            language: language.to_string(),
            signature,
            code,
            calls,
            imports: file_imports.to_vec(),
            complexity,
            has_branches,
            has_loops,
            has_error_handling,
            qualified_name: fn_name.to_string(),
            docstring: None,
            parent_class: None,
        })
    }

    fn extract_function_name(
        &self,
        node: tree_sitter::Node,
        source_code: &str,
        language: &str,
    ) -> Option<String> {
        if matches!(language, "JavaScript" | "TypeScript") {
            return self.js_function_name(node, source_code);
        }

        // Rust function_item, Python function_definition, Go function_declaration:
        // the name is the first identifier child.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "property_identifier" {
                return child
                    .utf8_text(source_code.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
        }
        None
    }

    /// Name a JavaScript/TypeScript function node.
    ///
    /// `arrow_function` nodes carry no name field — the name is the binding in
    /// the enclosing `variable_declarator` (e.g. `const Pagination = ({ page }) => …`).
    /// Arrows without such a binding (inline callbacks, IIFEs) return `None` and
    /// the unit is dropped. `function_declaration` and `method_definition` read the
    /// `name` field when present, falling back to the first identifier child.
    fn js_function_name(&self, node: tree_sitter::Node, source_code: &str) -> Option<String> {
        if node.kind() == "variable_declarator" {
            // Wrapped-component declarator: the unit is named after the binding.
            return node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source_code.as_bytes()).ok())
                .map(|s| s.to_string());
        }

        if node.kind() == "arrow_function" {
            let parent = node.parent()?;
            if parent.kind() == "variable_declarator" {
                return parent
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source_code.as_bytes()).ok())
                    .map(|s| s.to_string());
            }
            return None;
        }

        if let Some(name) = node.child_by_field_name("name")
            && let Ok(text) = name.utf8_text(source_code.as_bytes()) {
                return Some(text.to_string());
            }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "property_identifier" {
                return child
                    .utf8_text(source_code.as_bytes())
                    .ok()
                    .map(|s| s.to_string());
            }
        }
        None
    }
}
