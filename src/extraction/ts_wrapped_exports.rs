// JS/TS wrapped-export helpers extracted from `tree_sitter_extractor.rs`
// (issue #702: keep the extractor under the 500-line limit). Pulled in via
// `include!` from the extractor's private module — same pattern as
// `pipeline.rs`/`batch_ops.rs`. The item paths are unchanged: from
// `TreeSitterExtractor` these are `Self::…`, and the tests (included in the
// same module) call them through the same paths as before.

impl TreeSitterExtractor {
    /// JS/TS wrapped-component unit: a MODULE-LEVEL `const`/`let` binding
    /// (a `variable_declarator` under a top-level declaration — exported
    /// inline as `export const X = …` or not, issue #791) whose
    /// initializer is (a) a `styled`/`styled.*`/`styled(...)` tagged
    /// template, (b) a call whose callee is forwardRef/memo/observer/connect
    /// or a `with[A-Z]…` HOC, or (c) any call whose first argument is an
    /// inline function expression (issue #701) — except well-known
    /// non-component library functions (setTimeout/Promise etc., issue
    /// #791). The unit spans the full binding statement and is named after
    /// the binding.
    ///
    /// The trigger is the assignment to a variable binding, not the export:
    /// a wrapped component declared and exported separately
    /// (`const X = observer(…)` … `export default X`) is the same construct
    /// as one exported inline, and a wrapped binding used only within its
    /// own module is still a function worth indexing (issue #791).
    /// Function-local bindings are still excluded — module-level only.
    fn is_wrapped_component_binding(&self, node: tree_sitter::Node, source_code: &str) -> bool {
        if node.kind() != "variable_declarator" {
            return false;
        }
        let Some(declaration) = node.parent() else {
            return false;
        };
        // Module-level: the declaration (the declarator's direct parent) must
        // be a const/let declaration at the top level of the module — either
        // directly under the program root (bare `const X = …`) or inside an
        // inline export statement (`export const X = …`). Function-local
        // bindings (grandparent `function_body`, `arrow_function`, etc.) are
        // excluded. The root kind is pinned by
        // `test_js_root_node_kind_is_program`: a grammar swap would stop
        // this match silently.
        // `const`/`let` declarations map to distinct node kinds in the
        // JS/TS AST: the `const`/`let` declaration node kind starts with
        // "lex", while `var` is its own kind. Match both.
        let decl_kind = declaration.kind();
        let is_binding_decl = decl_kind.starts_with("lex") || decl_kind == "var_declaration";
        if !is_binding_decl
            || !declaration
                .parent()
                .is_some_and(|p| matches!(p.kind(), "program" | "export_statement"))
        {
            return false;
        }
        let Some(value) = node.child_by_field_name("value") else {
            return false;
        };
        // A destructured binding (`const { a, b } = memo(…)` / `const [a] = …`)
        // must not yield a unit: the `name` field would be the pattern text
        // (`"{ a, b }"`), producing a garbage entity name. The binding name
        // must be a plain identifier.
        let Some(name) = node.child_by_field_name("name") else {
            return false;
        };
        if name.kind() != "identifier" {
            return false;
        }
        Self::value_is_wrapped_form(value, source_code)
    }

    /// True when a declarator's initializer is a wrapped form: a styled
    /// template, a known wrapper/HOC call, or a call taking an inline fn.
    fn value_is_wrapped_form(value: tree_sitter::Node, source_code: &str) -> bool {
        // `styled.div`...`` and `styled(X)`...`` parse as call_expression whose
        // callee is `styled` / `styled.*` / the `styled(...)` call, followed by
        // a template_string argument. All other wrapper shapes (forwardRef/memo/
        // observer/connect/withHOC/plain inline-fn calls) parse as
        // call_expression taking a call-like or function argument.
        if value.kind() != "call_expression" {
            return false;
        }
        let Some(callee) = value.child_by_field_name("function") else {
            return false;
        };
        match callee.kind() {
            "member_expression" | "identifier" => {
                let callee_text = callee
                    .utf8_text(source_code.as_bytes())
                    .ok()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                Self::is_styled_callee(&callee_text)
                    || Self::is_wrapper_callee(&callee_text)
                    || (!Self::is_non_component_callee(&callee_text)
                        && Self::call_takes_inline_function(value, source_code, &callee_text))
            }
            "call_expression" => {
                // `styled(Base)` used as a template tag:
                // `export const X = styled(Base)\`...\`` — the tag is itself a
                // call_expression whose callee must be the bare `styled`.
                let callee_text = callee
                    .utf8_text(source_code.as_bytes())
                    .ok()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                callee.child_by_field_name("function").is_some_and(|f| {
                    f.kind() == "identifier"
                        && f.utf8_text(source_code.as_bytes())
                            .is_ok_and(|t| t == "styled")
                }) || (!Self::is_non_component_callee(&callee_text)
                    && Self::call_takes_inline_function(value, source_code, &callee_text))
            }
            _ => false,
        }
    }

    /// Extract the single unit for `export default <wrapped form>` statements
    /// (issue #701). The unit is named after the file stem — or the parent
    /// directory name when the stem is `index` or `styled`.
    fn extract_wrapped_default_export(
        &self,
        root: tree_sitter::Node,
        source_code: &str,
        language: &str,
        file_path: &str,
        file_imports: &[String],
        units: &mut Vec<CodeUnit>,
    ) {
        if !matches!(language, "JavaScript" | "TypeScript") {
            return;
        }
        let mut cursor = root.walk();
        for statement in root.children(&mut cursor) {
            if statement.kind() != "export_statement" {
                continue;
            }
            let mut scursor = statement.walk();
            let mut has_default = false;
            let mut wrapped: Option<tree_sitter::Node> = None;
            for child in statement.children(&mut scursor) {
                if child.kind() == "default" {
                    has_default = true;
                }
                if child.kind() == "call_expression"
                    && Self::value_is_wrapped_form(child, source_code)
                {
                    wrapped = Some(child);
                }
            }
            if !has_default {
                continue;
            }
            if wrapped.is_some() {
                let name = Self::default_export_name(file_path);
                if let Some(unit) = self.build_function_unit(
                    statement,
                    &name,
                    source_code,
                    language,
                    file_path,
                    file_imports,
                ) {
                    units.push(unit);
                }
            }
        }
    }

    /// Unit name for an `export default <wrapped form>`: the file stem, except
    /// for `index`/`styled` stems, which take the parent directory name
    /// (Button/index.js → Button).
    fn default_export_name(file_path: &str) -> String {
        let stem = file_path
            .rsplit('/')
            .next()
            .and_then(|f| f.split_once('.'))
            .map(|(s, _)| s)
            .unwrap_or(file_path);
        if matches!(stem, "index" | "styled") {
            file_path
                .rfind('/')
                .and_then(|i| file_path[..i].rsplit('/').next())
                .filter(|s| !s.is_empty())
                .unwrap_or(stem)
                .to_string()
        } else {
            stem.to_string()
        }
    }

    /// True when the callee text identifies a `styled` template tag.
    fn is_styled_callee(callee_text: &str) -> bool {
        callee_text == "styled" || callee_text.starts_with("styled.")
    }

    /// True when the callee text names a known function-wrapping helper
    /// (forwardRef, memo, observer, connect) or a `with[A-Z]…` HOC.
    fn is_wrapper_callee(callee_text: &str) -> bool {
        matches!(callee_text, "forwardRef" | "memo" | "observer" | "connect")
            || (callee_text.len() > 4
                && callee_text.starts_with("with")
                && callee_text.as_bytes()[4].is_ascii_uppercase())
    }

    /// True when the callee names a well-known library function or array/iter
    /// method whose callback idiom is NOT a component — timer/scheduling
    /// functions, Promise methods, and the common collection methods whose
    /// callback-first idiom is data transformation, not component definition
    /// (issue #791). These are excluded from the `call_takes_inline_function`
    /// catch-all so `const t = setTimeout(() => …)`, `const p = Promise.resolve(…)`,
    /// `const mapped = arr.map(x => x * 2)`, `const filtered = list.filter(…)`
    /// etc. do not become graph entities, while project-specific wrappers
    /// (debounce, withRetry, …) still qualify.
    ///
    /// Member-expression callees (`arr.map`, `list.filter`) are matched on
    /// the property name (the last segment after `.`), so both bare and
    /// namespaced forms are covered.
    ///
    /// This is a partial list, not an exhaustive class definition: other
    /// callback-first library idioms (e.g. `requestAnimationFrame`, `queueFn`,
    /// project-internal scheduling helpers) can still slip through. A full
    /// whitelist ("only these callees qualify") risks under-extracting
    /// project-specific wrappers, so the blacklist is a pragmatic stopgap
    /// (issue #791 round 1).
    fn is_non_component_callee(callee_text: &str) -> bool {
        // For member expressions like `arr.map` / `Promise.resolve`, match on
        // the property name (the segment after the last `.`).
        let effective = callee_text.rsplit('.').next().unwrap_or(callee_text);
        matches!(
            effective,
            "setTimeout"
                | "setInterval"
                | "queueMicrotask"
                | "map"
                | "filter"
                | "reduce"
                | "forEach"
                | "find"
                | "some"
                | "every"
        ) || callee_text.starts_with("Promise")
    }

    /// True when the first argument of a call expression is an inline
    /// arrow function or function expression AND the call's own callee names
    /// a component wrapper.
    ///
    /// The callee narrowing exists because the catch-all alone would
    /// flood the graph with non-component entities the moment the gate
    /// became assignment-keyed: `const mapped = arr.map(x => …)`-style
    /// iterator/callback assignments are not components. The callee must
    /// therefore be one of the bare wrappers, or the documented `React.*`
    /// member form (property a wrapper AND object `React`); everything
    /// else (`.map`, `.then`, `foo.memo`, event handlers) stays excluded
    /// (pinned by `test_js_wrapped_inline_callback_not_extracted`).
    ///
    /// `callee_text` is the call's callee text, pre-computed by the caller
    /// (`value_is_wrapped_form`); it is only used for the `identifier`
    /// arm — the `member_expression` arm re-reads the property and object
    /// fields and ignores it.
    fn call_takes_inline_function(
        call: tree_sitter::Node,
        source_code: &str,
        callee_text: &str,
    ) -> bool {
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = arguments.walk();
        let has_inline_fn = arguments
            .children(&mut cursor)
            .find(|c| !matches!(c.kind(), "(" | ")"))
            .is_some_and(|c| matches!(c.kind(), "arrow_function" | "function"));
        if !has_inline_fn {
            return false;
        }
        // The first argument is inline — but the call itself must be a
        // wrapper call, not an arbitrary call that happens to take a
        // callback. Check the callee: either a bare wrapper identifier, or a
        // `React.*` member whose property is a wrapper.
        let Some(callee) = call.child_by_field_name("function") else {
            return false;
        };
        match callee.kind() {
            "identifier" => Self::is_wrapper_callee(callee_text),
            // `React.memo` / `React.forwardRef` (the documented member form):
            // the property must be a wrapper AND the object must be the
            // `React` identifier — matching on the property alone would let
            // `foo.memo(…)` / `arr.withX(…)` over-extract.
            "member_expression" => {
                let property_is_wrapper = callee
                    .child_by_field_name("property")
                    .and_then(|p| p.utf8_text(source_code.as_bytes()).ok())
                    .is_some_and(Self::is_wrapper_callee);
                let object_is_react = callee
                    .child_by_field_name("object")
                    .is_some_and(|o| {
                        o.kind() == "identifier"
                            && o.utf8_text(source_code.as_bytes())
                                .is_ok_and(|t| t == "React")
                    });
                property_is_wrapper && object_is_react
            }
            _ => false,
        }
    }
}
