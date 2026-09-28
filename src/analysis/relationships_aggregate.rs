// Import/call-edge aggregation for RelationshipBuilder (extracted from
// relationships.rs for the 500-line source budget — issue #714 refactor).
//
// Holds the (src, tgt, rel_type) → (weight, evidence, provenance) bookkeeping
// that turns raw resolution hits into deduplicated relationships, plus the
// final loop that materialises those maps into `Relationship` values.
// Included via #[path] from relationships.rs.

use crate::extraction::grouping::GroupingResult;

use super::*;
use crate::analysis::import_resolver::{JsResolverContext, UnresolvedKind};
use crate::analysis::relationship_helpers::{
    aggregate_depends_on, aggregate_flat_depends_on, build_fn_map, build_import_map,
    entity_parent_map, resolve_import_for,
};

/// Per-edge-key bookkeeping maps that turn raw resolution hits into
/// deduplicated relationships: saturated weight per (src, tgt, rel_type),
/// the accumulated evidence JSON array, and the resolved-wins provenance.
pub(crate) struct EdgeAggregation {
    pub(crate) edge_weights: HashMap<(String, String, RelType), u32>,
    pub(crate) evidence_map: HashMap<(String, String, RelType), Vec<serde_json::Value>>,
    pub(crate) provenance_map: HashMap<(String, String, RelType), EdgeProvenance>,
}

impl EdgeAggregation {
    fn new() -> Self {
        Self {
            edge_weights: HashMap::new(),
            evidence_map: HashMap::new(),
            // Resolved-wins precedence (issue #714): the same (src, tgt,
            // RelType) key can be produced by both a resolved path
            // (import_map / JsResolverContext) and a heuristic path
            // (bare_name_map / fn_map) — e.g. a Rust `use` statement that
            // also has a matching bare module name. `merge_provenance`
            // enforces resolved-wins before any upsert.
            provenance_map: HashMap::new(),
        }
    }

    /// Emit one (source → target) edge with evidence, skipping self-edges.
    /// Shared by the import and call-evidence branches so the
    /// saturation/evidence bookkeeping stays in one place.
    pub(crate) fn emit(
        &mut self,
        source_id: &str,
        target_id: &str,
        rel_type: RelType,
        evidence: serde_json::Value,
        provenance: EdgeProvenance,
    ) {
        if source_id == target_id {
            return;
        }
        let key = (source_id.to_string(), target_id.to_string(), rel_type);
        let w = self.edge_weights.entry(key.clone()).or_insert(0);
        // Issue 4 fix: saturating add to avoid u32 overflow
        *w = w.saturating_add(1);
        self.evidence_map
            .entry(key.clone())
            .or_default()
            .push(evidence);
        merge_provenance(&mut self.provenance_map, key, provenance);
    }
}

/// Resolved-wins merge for the per-edge-key provenance map (issue #714 PM
/// decision: precedence is enforced at the builder aggregation stage, before
/// upsert). `EdgeProvenance::rank` is the single source of truth for the
/// resolved > heuristic order.
fn merge_provenance(
    provenance_map: &mut HashMap<(String, String, RelType), EdgeProvenance>,
    key: (String, String, RelType),
    provenance: EdgeProvenance,
) {
    let should_replace = provenance_map
        .get(&key)
        .is_none_or(|existing| provenance.rank() > existing.rank());
    if should_replace {
        provenance_map.insert(key, provenance);
    }
}

/// Resolve a unit's `imports` and (Rust-only) `calls` lists into file→file
/// Imports edges, folding them into `agg`.
struct FileEdgeContext<'a> {
    code_units: &'a [CodeUnit],
    path_map: &'a HashMap<&'a str, &'a str>,
    import_map: &'a HashMap<String, String>,
    fn_map: &'a HashMap<&'a str, &'a str>,
    bare_name_map: &'a HashMap<&'a str, &'a str>,
    js_resolver: &'a mut Option<JsResolverContext>,
    files: &'a [Entity],
    repo_root: &'a Path,
}

fn collect_file_edges(ctx: &mut FileEdgeContext<'_>, agg: &mut EdgeAggregation) {
    for unit in ctx.code_units {
        let source_file_id = match ctx.path_map.get(unit.file.as_str()) {
            Some(id) => *id,
            None => continue, // file not in grouping — skip
        };

        // imports edges — JS/TS specifiers go through the tier-1
        // path-normalisation resolver (language-gated; the Rust path
        // below is untouched for all other languages).
        for import in &unit.imports {
            let target_id = if unit.language == "JavaScript" || unit.language == "TypeScript" {
                let ctx = ctx
                    .js_resolver
                    .get_or_insert_with(|| JsResolverContext::new(ctx.files, ctx.repo_root));
                ctx.resolve(import, &unit.file).map(|id| id.to_string())
            } else if unit.language == "Rust" {
                // Language gate at the call site: the relative-specifier
                // walk is Rust-only; other languages pass no module context
                // (spec #742 — the gate lives here, not inside the resolver).
                resolve_import_for(import, ctx.import_map, Some(&unit.file))
                    .map(|id| id.to_string())
            } else {
                resolve_import_for(import, ctx.import_map, None).map(|id| id.to_string())
            };
            if let Some(target_id) = target_id {
                // Both branches above are exact resolution paths
                // (normalised-path import_map lookup, or the JS/TS
                // JsResolverContext) — resolved.
                agg.emit(
                    source_file_id,
                    &target_id,
                    RelType::Imports,
                    serde_json::json!({
                        "source_file": unit.file,
                        "source_line": unit.line,
                        "source_end_line": unit.end_line,
                        "import_path": import,
                    }),
                    EdgeProvenance::Resolved,
                );
            }
        }

        // calls edges → file→file Imports (Rust only).
        //
        // For JS/TS the calls list is function-level call evidence, not
        // module-level dependency declarations: the JS/TS extractor
        // records every call site (including same-file and external
        // library calls) verbatim in `calls`, while true cross-file
        // dependencies are already carried by `imports` (resolved above).
        // Feeding JS/TS call names into fn_map/bare_name_map turned call
        // sites into bogus import edges — bare function names and file
        // stems matched repo-wide with no path normalisation, producing
        // cross-tree edges (issue #706). Only Rust uses mod/use semantics
        // that put module dependencies in `calls`, so the branch is
        // language-gated.
        if unit.language != "JavaScript" && unit.language != "TypeScript" {
            for call in &unit.calls {
                // First check if it's a bare module name (no ::) matching a known file stem.
                // This handles `mod embedding;` declarations the extractor puts in `calls`.
                if !call.contains("::")
                    && !call.contains('(')
                    && let Some(target_id) = ctx.bare_name_map.get(call.as_str())
                {
                    // bare_name_map is a name-matching heuristic (file
                    // stem lookup, not a resolver) — heuristic.
                    agg.emit(
                        source_file_id,
                        target_id,
                        RelType::Imports,
                        serde_json::json!({
                            "source_file": unit.file,
                            "caller_line": unit.line,
                            "callee_name": call,
                        }),
                        EdgeProvenance::Heuristic,
                    );
                    continue; // Don't also check fn_map for this bare name
                }
                // Function call lookup — resolves to file→file edges.
                // At file tier, all cross-file dependencies are "imports" (file A uses/imports file B),
                // not "calls" — files don't call files, they depend on them.
                // Function→function calls use RelType::Calls separately (section 6 in relationships.rs).
                if let Some(target_id) = ctx.fn_map.get(call.as_str()) {
                    // fn_map is a name-matching heuristic (function name
                    // lookup, not a resolver) — heuristic.
                    agg.emit(
                        source_file_id,
                        target_id,
                        RelType::Imports,
                        serde_json::json!({
                            "source_file": unit.file,
                            "caller_line": unit.line,
                            "callee_name": call,
                        }),
                        EdgeProvenance::Heuristic,
                    );
                }
            }
        }
    }
}

/// Convert ALL aggregated edges (file-level + function-level) into
/// `Relationship` structs and append them to `rels`.
fn materialize_edges(agg: &EdgeAggregation, rels: &mut Vec<Relationship>) {
    for ((src, tgt, rel_type), count) in &agg.edge_weights {
        let key = (src.clone(), tgt.clone(), *rel_type);
        let evidence_json = agg
            .evidence_map
            .get(&key)
            .and_then(|entries| serde_json::to_string(entries).ok());
        // Resolved-wins: default to Heuristic only if somehow no
        // provenance was recorded for this key (defensive; every
        // `emit`/Calls/Imports call site records one).
        let provenance = agg
            .provenance_map
            .get(&key)
            .copied()
            .unwrap_or(EdgeProvenance::Heuristic);
        rels.push(Relationship {
            source_id: src.clone(),
            target_id: tgt.clone(),
            rel_type: *rel_type,
            weight: *count as f64,
            evidence_json,
            provenance,
        });
    }
}

/// Inner build inputs shared across the `build_inner` sections — keeps the
/// orchestration below under the 7-argument clippy limit.
pub(crate) struct BuildContext<'a> {
    pub(crate) code_units: &'a [CodeUnit],
    pub(crate) grouping: &'a GroupingResult,
    pub(crate) project_id: &'a str,
    pub(crate) repo_name: &'a str,
    pub(crate) repo_root: &'a Path,
    pub(crate) function_entities: Option<&'a [Entity]>,
}

impl RelationshipBuilder {
    /// Inner implementation of relationship building. Returns the edges plus
    /// the unresolved-import side-channel counter (file→file and function→file
    /// loops both contribute — the same specifier seen in N units counts N
    /// times, matching the pre-amendment counting behaviour).
    pub(crate) fn build_inner(
        ctx: &BuildContext<'_>,
    ) -> Result<(Vec<Relationship>, UnresolvedCounts)> {
        let BuildContext {
            code_units,
            grouping,
            project_id,
            repo_name,
            repo_root,
            function_entities,
        } = ctx;
        let code_units = *code_units;
        let grouping = *grouping;
        let project_id = *project_id;
        let repo_name = *repo_name;
        let repo_root = *repo_root;
        let function_entities = *function_entities;
        let mut rels: Vec<Relationship> = Vec::new();

        // 1. contains: subsystem→module
        for module in &grouping.modules {
            if let Some(parent_id) = &module.parent_id {
                let evidence = module.path.as_deref().and_then(|p| {
                    serde_json::to_string(&serde_json::json!({ "child_path": p })).ok()
                });
                rels.push(Relationship {
                    source_id: parent_id.clone(),
                    target_id: module.id.clone(),
                    rel_type: RelType::Contains,
                    weight: 1.0,
                    evidence_json: evidence,
                    // Structural edge (grouping parent_id) — exact, not name-matched.
                    provenance: EdgeProvenance::Resolved,
                });
            }
        }

        // 2. contains: module→file
        for file in &grouping.files {
            if let Some(parent_id) = &file.parent_id {
                let evidence = file.path.as_deref().and_then(|p| {
                    serde_json::to_string(&serde_json::json!({ "child_path": p })).ok()
                });
                rels.push(Relationship {
                    source_id: parent_id.clone(),
                    target_id: file.id.clone(),
                    rel_type: RelType::Contains,
                    weight: 1.0,
                    evidence_json: evidence,
                    // Structural edge (grouping parent_id) — exact, not name-matched.
                    provenance: EdgeProvenance::Resolved,
                });
            }
        }

        // Build lookup maps for import/call resolution.
        // import_path → file entity ID  (e.g. "crate::utils" → file id)
        let import_map = build_import_map(&grouping.files, project_id, repo_name);
        // function name → file entity ID
        let fn_map = build_fn_map(code_units, &grouping.files);
        // file path → file entity ID
        let path_map: HashMap<&str, &str> = grouping
            .files
            .iter()
            .filter_map(|f| f.path.as_deref().map(|p| (p, f.id.as_str())))
            .collect();

        // bare module name → file entity ID
        // Maps file stem (e.g., "embedding" from "src/embedding.rs") to file entity ID.
        // Only unambiguous (stem appears in exactly one file) — multi-file stems are omitted.
        let bare_name_map: HashMap<&str, &str> = {
            let mut stem_counts: HashMap<&str, usize> = HashMap::new();
            let mut stem_to_id: HashMap<&str, &str> = HashMap::new();
            for file in &grouping.files {
                if let Some(path) = file.path.as_deref() {
                    // Extract stem: "src/embedding.rs" → "embedding"
                    if let Some(stem) = std::path::Path::new(path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .filter(|s| !s.is_empty() && *s != "mod")
                    // skip "mod.rs" → "mod"
                    {
                        *stem_counts.entry(stem).or_insert(0) += 1;
                        stem_to_id.insert(stem, file.id.as_str());
                    }
                }
            }
            // Keep only unambiguous (count == 1) stems
            stem_to_id.retain(|stem, _| stem_counts.get(stem).copied() == Some(1));
            stem_to_id
        };

        // 3 & 4. Collect file→file edges, aggregating weight per (source, target, type).
        // The aggregation struct accumulates all evidence entries per edge key as
        // a JSON array, preserving every call/import instance rather than only
        // the first seen, and enforces resolved-wins provenance precedence
        // (issue #714) before any upsert.
        let mut agg = EdgeAggregation::new();

        // JS/TS tier-1 resolver context (lazy: built only when the repo
        // actually contains JavaScript/TypeScript units).
        let mut js_resolver: Option<JsResolverContext> = None;

        let mut file_ctx = FileEdgeContext {
            code_units,
            path_map: &path_map,
            import_map: &import_map,
            fn_map: &fn_map,
            bare_name_map: &bare_name_map,
            js_resolver: &mut js_resolver,
            files: &grouping.files,
            repo_root,
        };
        collect_file_edges(&mut file_ctx, &mut agg);

        // 5. depends_on: aggregate file-level imports+calls up to module and subsystem tier
        // Must happen before function-level edges are added, using only file-level edges
        let file_to_module = entity_parent_map(&grouping.files);
        let module_to_subsystem = entity_parent_map(&grouping.modules);

        let module_edges = aggregate_depends_on(&agg.edge_weights, &file_to_module);
        for ((src, tgt), count) in &module_edges {
            rels.push(Relationship {
                source_id: src.clone(),
                target_id: tgt.clone(),
                rel_type: RelType::DependsOn,
                weight: *count as f64,
                evidence_json: None,
                // Aggregated DependsOn (module/subsystem tier) is a
                // structural roll-up of file-level edges, not a
                // name-matching heuristic — resolved (PM decision #714).
                provenance: EdgeProvenance::Resolved,
            });
        }

        let subsystem_edges = aggregate_flat_depends_on(&module_edges, &module_to_subsystem);
        for ((src, tgt), count) in &subsystem_edges {
            rels.push(Relationship {
                source_id: src.clone(),
                target_id: tgt.clone(),
                rel_type: RelType::DependsOn,
                weight: *count as f64,
                evidence_json: None,
                provenance: EdgeProvenance::Resolved,
            });
        }

        // 6. Function-level edges (optional): Calls and Imports between functions
        if let Some(functions) = function_entities {
            let mut fn_ctx = relationships_function_edges::FunctionEdgeContext {
                code_units,
                functions,
                files: &grouping.files,
                path_map: &path_map,
                import_map: &import_map,
                repo_root,
            };
            relationships_function_edges::collect_function_edges(&mut fn_ctx, &mut agg);
        }

        // Materialise every aggregated edge (file-level + function-level)
        // into Relationship structs.
        materialize_edges(&agg, &mut rels);

        // Unresolved-import side-channel (#690 amendment): derive the
        // internal/external split from the same language/specifier rules the
        // resolver above used. "Internal" = relative ("./…") specifiers that
        // resolved to nothing; "external" = everything else (bare JS package
        // names, Node built-ins, Rust specifiers not starting with
        // crate::/self::/super::). No edges are produced either way.
        let mut unresolved = UnresolvedCounts::default();
        for unit in code_units {
            if unit.language != "JavaScript" && unit.language != "TypeScript" {
                for import in &unit.imports {
                    // Same call-site language gate as the resolution loops
                    // above: the module context (and therefore the relative
                    // walk) is threaded only for Rust units.
                    let resolved = if unit.language == "Rust" {
                        resolve_import_for(import, &import_map, Some(&unit.file))
                    } else {
                        resolve_import_for(import, &import_map, None)
                    };
                    if resolved.is_none() {
                        // Unresolved-kind classification (side-channel that
                        // selfcheck_ops gates on, #724): relative specifiers
                        // are Internal. The Rust `super::`/`self::` forms are
                        // internal relative specifiers that do NOT start with
                        // '.', so they must be classified before the '.'
                        // check (review fix on PR #747 — classifying them
                        // External silently corrupted the counter).
                        let kind = if import.starts_with('.')
                            || import.starts_with("super::")
                            || import.starts_with("self::")
                            || import == "super"
                            || import == "self"
                        {
                            UnresolvedKind::Internal
                        } else {
                            UnresolvedKind::External
                        };
                        kind.apply(&mut unresolved);
                    }
                }
            }
        }
        if let Some(ctx) = &js_resolver {
            for unit in code_units
                .iter()
                .filter(|u| u.language == "JavaScript" || u.language == "TypeScript")
            {
                for import in &unit.imports {
                    if ctx.resolve(import, &unit.file).is_none() {
                        let kind = if import.starts_with('.') || ctx.matches_source_root(import) {
                            UnresolvedKind::Internal
                        } else {
                            UnresolvedKind::External
                        };
                        kind.apply(&mut unresolved);
                    }
                }
            }
        }

        Ok((rels, unresolved))
    }
}
