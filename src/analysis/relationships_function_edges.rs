// Function-level edge collection (section 6 of the original builder),
// extracted from relationships_aggregate.rs for the 500-line source budget
// (issue #714 refactor).
// Included via #[path] from relationships.rs.

use crate::extraction::function_preservation::{
    build_function_map, extract_cfg_test_functions, is_function_unit, is_test_file_path,
};
use std::collections::HashSet;

use super::relationships_aggregate::EdgeAggregation;
use super::*;
use crate::analysis::import_resolver::JsResolverContext;
use crate::analysis::relationship_helpers::resolve_import_for;

/// Function-level edge collection inputs (section 6) — the per-file fn-map
/// lookups, cfg(test) filter, and the function→function/function→file edge
/// emission.
pub(crate) struct FunctionEdgeContext<'a> {
    pub(crate) code_units: &'a [CodeUnit],
    pub(crate) functions: &'a [Entity],
    pub(crate) files: &'a [Entity],
    pub(crate) path_map: &'a HashMap<&'a str, &'a str>,
    pub(crate) import_map: &'a HashMap<String, String>,
    pub(crate) repo_root: &'a Path,
}

/// Emit function-level Calls and Imports edges (section 6 of the original
/// builder), folding them into `agg`.
pub(crate) fn collect_function_edges(ctx: &mut FunctionEdgeContext<'_>, agg: &mut EdgeAggregation) {
    // Cache for cfg(test) function names per file to avoid reparsing
    // HashMap<file_path, HashSet<function_name>>
    let mut cfg_test_cache: HashMap<String, HashSet<String>> = HashMap::new();
    let fn_map = build_function_map(ctx.functions);
    // Build function name → (file_id, function_id) lookup for name matching
    let fn_name_lookup: HashMap<&str, Vec<(&str, &str)>> = ctx
        .functions
        .iter()
        .filter_map(|f| {
            f.parent_id
                .as_ref()
                .map(|parent_id| (f.name.as_str(), (parent_id.as_str(), f.id.as_str())))
        })
        .fold(HashMap::new(), |mut acc, (name, info)| {
            acc.entry(name).or_default().push(info);
            acc
        });

    // JS/TS tier-1 resolver context (lazy: built only when the repo
    // actually contains JavaScript/TypeScript function units).
    let mut js_resolver: Option<JsResolverContext> = None;

    for unit in ctx.code_units {
        // Only function-type units have corresponding Function entities in the graph
        if !is_function_unit(&unit.unit_type) {
            continue;
        }

        // Skip test files - they don't have Function entities created during extraction
        if is_test_file_path(&unit.file) {
            continue;
        }

        // Check if this CodeUnit is a cfg(test) function and skip it
        // These are excluded from entity creation during extraction,
        // so they won't have Function entities to match against
        let cfg_test_fns = cfg_test_cache
            .entry(unit.file.clone())
            .or_insert_with(|| extract_cfg_test_functions(&unit.file, ctx.repo_root));
        if cfg_test_fns.contains(&unit.name) {
            continue; // Skip cfg(test) functions - no entity exists
        }

        let source_file_id = match ctx.path_map.get(unit.file.as_str()) {
            Some(id) => *id,
            None => continue,
        };

        // Match CodeUnit to its owning function by name, not all functions
        // Each CodeUnit represents a single function body and has a name field
        let source_fn_ids: Vec<&str> = fn_name_lookup
            .get(unit.name.as_str())
            .map(|candidates| {
                candidates
                    .iter()
                    .filter(|(file_id, _)| *file_id == source_file_id)
                    .map(|(_, fn_id)| *fn_id)
                    .collect()
            })
            .unwrap_or_default();

        if source_fn_ids.is_empty() {
            let safe_name = unit
                .name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect::<String>();
            let safe_file = unit
                .file
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '/' || *c == '.' || *c == '-' || *c == '_')
                .collect::<String>();
            // Only log when there are calls to skip
            if !unit.calls.is_empty() {
                tracing::debug!(
                    "call graph: no function entity matched unit name '{}' in file '{}', skipping {} calls",
                    safe_name,
                    safe_file,
                    unit.calls.len()
                );
            }
            continue;
        }

        for source_fn_id in source_fn_ids {
            // Function→Function Calls edges
            for call in &unit.calls {
                let target_ids = fn_map
                    .get(call.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                if target_ids.len() > 10 {
                    let safe_call = call
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                        .collect::<String>();
                    tracing::debug!(
                        "call graph: call '{}' matches {} candidates - graph may be verbose",
                        safe_call,
                        target_ids.len()
                    );
                }

                for target_fn_id in target_ids {
                    if target_fn_id != source_fn_id {
                        // tree-sitter Calls edges are name-matched
                        // (fn_map lookup, not a resolver) — heuristic
                        // (PM decision #714).
                        agg.emit(
                            source_fn_id,
                            target_fn_id,
                            RelType::Calls,
                            serde_json::json!({
                                "source_function": source_fn_id,
                                "source_file": unit.file,
                                "caller_line": unit.line,
                                "callee_name": call,
                            }),
                            EdgeProvenance::Heuristic,
                        );
                    }
                }
            }

            // Function→File Imports edges
            for import in &unit.imports {
                let target_id = if unit.language == "JavaScript" || unit.language == "TypeScript" {
                    let ctx = js_resolver
                        .get_or_insert_with(|| JsResolverContext::new(ctx.files, ctx.repo_root));
                    ctx.resolve(import, &unit.file).map(|id| id.to_string())
                } else if unit.language == "Rust" {
                    // Language gate at the call site: the relative-specifier
                    // walk is Rust-only; other languages pass no module
                    // context (spec #742 — the gate lives here, not inside
                    // the resolver).
                    resolve_import_for(import, ctx.import_map, Some(&unit.file))
                        .map(|id| id.to_string())
                } else {
                    resolve_import_for(import, ctx.import_map, None).map(|id| id.to_string())
                };
                if let Some(target_id) = target_id
                    && target_id != source_file_id
                {
                    // Function→file Imports resolved via the same
                    // exact-resolution paths as the file-level
                    // loop above — resolved.
                    agg.emit(
                        source_fn_id,
                        &target_id,
                        RelType::Imports,
                        serde_json::json!({
                            "source_function": source_fn_id,
                            "source_file": unit.file,
                            "source_line": unit.line,
                            "import_path": import,
                        }),
                        EdgeProvenance::Resolved,
                    );
                }
            }
        }
    }
}
