//! Call-path and blast-radius orchestration for `lievo_explore` (issues
//! #680/#711). The blast-radius traversal internals live in `core.rs`
//! (extracted, issue #834, to keep the module under the 500-line source
//! budget); this file owns the edge-set constants, the seed/entry helpers,
//! the lean dependent-count hint, and the public
//! [`call_paths_and_blast_radius`] entry point.
//!
//! Blast-radius semantics — see `reverse_blast_radius` for the contract.
//! Parent normalization: every endpoint is normalized to its OWNING FILE
//! entity (`core::owning_file`), walking the full containment chain.

use serde_json::{Value, json};

use std::collections::HashSet;

use crate::model::{Entity, RelType};
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;

use super::core::{OwningFileResult, owning_file};

/// Reverse (incoming) traversal depth for `blast_radius` (issue #759 operator
/// decision 1: an internal constant, NOT a caller-configurable param). Two
/// hops covers the measured regression use case — the task needed a 2-hop
/// reverse import closure. A file imported by A, where A is imported by B,
/// makes B a hop-2 dependent of the target.
pub(crate) const BLAST_RADIUS_MAX_HOPS: usize = 2;

/// Maximum blast_radius entries per symbol; a hub imported by thousands of
/// files must not push the 24K output cap into its O(n²) shrink loop (DoS
/// guard, mirroring `MAX_MAX_FILES`). Excess dependents are dropped in BFS
/// (hop) order — the whole hop-0 pass completes before hop 1 begins — and
/// the omission is disclosed once via the returned error string with the
/// count of dependents reached but omitted (issue #854). The lean
/// dependent-count hint (issue #767) also names this cap so a caller knows
/// `include_depth=true` returns at most this many entries even when the
/// true count is higher.
pub const MAX_BLAST_ENTRIES: usize = 25;

/// Relation types treated as "one entity depends on another" for
/// blast_radius purposes (issue #759 operator decision: include the rel
/// types that genuinely mean depends-on; exclude purely structural ones).
/// Included: Imports, Calls, DependsOn, Implements — all four represent a
/// dependency that would break if the target changed. Excluded: Contains —
/// pure structural containment (module contains file), not a dependency.
/// This set GATES TRAVERSAL: every one of these edges is walked so a
/// call-source at hop 1 still has its hop-2 importers found (issue #834).
const DEPENDENCY_REL_TYPES: &[RelType] = &[
    RelType::Imports,
    RelType::Calls,
    RelType::DependsOn,
    RelType::Implements,
];

/// Relation types whose presence on a blast-radius entry is worth emitting
/// (issue #834: `calls`-derived entries carry no signal the agent uses —
/// they are 23.7% of the blast_radius byte share with zero distinct meaning
/// beyond the file's own call graph, which `call_paths` already carries).
/// The emit set is a STRICT SUBSET of [`DEPENDENCY_REL_TYPES`]: `Calls`
/// still drives traversal (a call-source at hop 1 is walked so its hop-2
/// importers are found) but is never labelled on a blast entry.
/// `DependsOn`/`Implements` remain in the emit set because they are the
/// genuine dependency signals the payload exists to convey.
const EMIT_REL_TYPES: &[RelType] = &[RelType::Imports, RelType::DependsOn, RelType::Implements];

/// The canonical "is this a dependency edge" predicate (Imports, Calls,
/// DependsOn, Implements) — the FULL dependency set. Used by both the
/// blast-radius traversal and `get_impact`'s `hop_tracked_dependents`
/// (issue #840: one shared predicate, no drift).
///
/// Do NOT confuse with [`is_emit_edge`], the STRICT SUBSET that only gates
/// blast-entry labelling (`Calls` drives traversal but is never labelled).
pub(crate) fn is_dependency_edge(rel_type: &RelType) -> bool {
    DEPENDENCY_REL_TYPES.contains(rel_type)
}

/// True when a relationship edge should be LABELLED on a blast_radius entry
/// (as opposed to only driving traversal). Gates entry creation and the
/// rel_types merge push, never `visited`/`next_frontier` (issue #834).
fn is_emit_edge(rel_type: &RelType) -> bool {
    EMIT_REL_TYPES.contains(rel_type)
}

/// Resolve the Function-tier entity id for a matched file entity.
///
/// Preference: an already-stored Function-tier child of the file
/// (parent_id == file id). Fallback: derive via function_id (the same
/// formula function_preservation uses when persisting).
///
/// Internal to the Explore tool; do not call from outside the retrieval module.
pub(crate) fn file_function_entity_id(storage: &dyn Storage, file: &Entity) -> Option<String> {
    use crate::model::EntityTier;
    let file_id = file.id.clone();
    let children = storage.entities_by_parent(&file_id).unwrap_or_default();
    if let Some(child) = children.iter().find(|c| c.tier == EntityTier::Function) {
        return Some(child.id.clone());
    }
    let name = file
        .path
        .as_deref()
        .and_then(|p| p.rsplit('/').next())
        .filter(|n| n.contains('.'))?;
    let stem = name.split('.').next().unwrap_or(name);
    Some(crate::extraction::function_preservation::function_id(
        &file_id, stem,
    ))
}

/// The seed set for a hop-0 reverse scan of `target_file`: the file entity
/// itself (when File-tier) plus every Function-tier child — the same
/// frontier `reverse_blast_radius` expands at hop 0 (issue #767), shared
/// so the lean count and the tier-2 closure stay in lockstep by
/// construction.
pub(crate) fn direct_seeds(storage: &dyn Storage, target_file: &Entity) -> Vec<Entity> {
    use crate::model::EntityTier;
    let mut seeds: Vec<Entity> = Vec::new();
    if target_file.tier == EntityTier::File {
        seeds.push(target_file.clone());
    }
    for child in storage
        .entities_by_parent(&target_file.id)
        .unwrap_or_default()
    {
        if child.tier == EntityTier::Function {
            seeds.push(child);
        }
    }
    seeds
}

/// Hop-0 direct-dependent count for the lean hint (issue #767): one
/// `relationships_to` read per seed, hop-0 only (no 2-hop traversal, so the
/// lean path keeps #731's token win). Same semantics as
/// `reverse_blast_radius` at hop 0, GATED ON THE EMIT SET (issue #834): a
/// dependent reached only through a Calls edge is not emitted by
/// `reverse_blast_radius`, so it is not counted here either — the hint must
/// stay a lower bound on what `include_depth=true` can actually return.
/// Cross-project excluded (#764), owning-file normalization, self/sibling
/// excluded. Count is UNBOUNDED (states the true scale; may exceed
/// [`MAX_BLAST_ENTRIES`]). Zero dependents or a storage read failure →
/// `None`.
pub(crate) fn count_direct_dependents(
    storage: &dyn Storage,
    target_file: &Entity,
    seed_entities: &[Entity],
) -> Option<usize> {
    use crate::model::EntityTier;
    let target_id = &target_file.id;
    let target_project_id = &target_file.project_id;
    let mut cache: std::collections::HashMap<String, Entity> = std::collections::HashMap::new();
    if target_file.tier == EntityTier::File {
        cache.insert(target_file.id.clone(), target_file.clone());
    }
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for node in seed_entities {
        let incoming = storage.relationships_to(&node.id).ok()?;
        for (rel, source) in incoming {
            // Same emit-set gate as `reverse_blast_radius` (issue #834):
            // calls-only dependents are not emittable, so they are not
            // counted — the hint would otherwise overstate what
            // `include_depth=true` can return.
            if !is_emit_edge(&rel.rel_type) || !same_project(target_project_id, &source.project_id)
            {
                continue;
            }
            let owning = owning_file(&source, &mut cache, storage);
            if let OwningFileResult::LookupFailed(_) = &owning {
                // Storage read error mid-walk: the count would be incomplete
                // — not disclosed rather than presented as a true count.
                return None;
            }
            let OwningFileResult::Found(file) = owning else {
                continue;
            };
            if file.id != *target_id {
                seen_files.insert(file.id.clone());
            }
        }
    }
    (!seen_files.is_empty()).then_some(seen_files.len())
}

/// The lean (include_depth=false) dependent-count hint (issue #767):
/// structured completeness field mirroring #743. `None` when zero direct
/// dependents or the count is unavailable; hop-0 and project-scoped.
/// `retrieval_ceiling` set when count exceeds [`MAX_BLAST_ENTRIES`].
/// Constant-size by construction (fixed remedy string, one counter) — a hub
/// with hundreds of dependents produces the same-sized hint as one with one.
#[derive(serde::Serialize, PartialEq, Debug)]
pub(crate) struct DependentsHint {
    pub complete: bool,
    pub omitted_direct_dependents: usize,
    pub remedy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_ceiling: Option<usize>,
}

pub(crate) fn lean_dependents_hint(storage: &dyn Storage, file: &Entity) -> Option<DependentsHint> {
    let seeds = direct_seeds(storage, file);
    let count = count_direct_dependents(storage, file, &seeds)?;
    Some(DependentsHint {
        complete: false,
        omitted_direct_dependents: count,
        remedy: "pass include_depth=true to retrieve them".to_string(),
        retrieval_ceiling: (count > MAX_BLAST_ENTRIES).then_some(MAX_BLAST_ENTRIES),
    })
}

/// Build call-path and blast-radius entries for a matched file entity.
///
/// `call_paths`: every Function-tier entity in the file (the file entity
/// when it is Function-tier, plus its Function-tier children) contributes
/// its Calls edges, outgoing and incoming, each with an explicit
/// `direction` ("out"/"in") — the reference shape, unchanged by issue
/// #759. Edges are deduplicated on the id triple
/// `(entity_id, target_entity_id, direction)` over the WHOLE file (issue
/// #835) and emitted sorted by that triple: the id triple is the identity
/// of a call edge (keying on the callee NAME would merge distinct same-
/// named callees in different files), and mutual call pairs (one "out"
/// row, one "in" row) are preserved because direction is part of the key.
/// `target_entity_id` (an `fn-<hash>` id that embeds no path and cannot
/// be resolved by the caller) and `rel_type` (the constant "calls") are
/// dropped from the emitted row; `entity_id` (the source function) is
/// retained. Storage read failures leave that entity's call paths absent
/// (best-effort, pre-existing semantics) but are disclosed in
/// `call_path_errors`.
///
/// `blast_radius`: see `reverse_blast_radius` for the contract. Storage
/// read failures are disclosed in `blast_radius_errors`.
///
/// Returns `(call_paths, blast_radius, call_path_errors, blast_radius_errors, complete)`;
/// the error vectors are empty in the happy path and surfaced only when
/// non-empty. `complete` is the per-symbol blast-radius completeness signal
/// (issue #854): true when the two-level traversal finished without
/// truncation (the cap did not bind and no storage read failed), so the
/// returned `blast_radius` IS the complete two-level reverse dependency
/// closure — the caller need not re-query the dependents for depth. False
/// when the cap bound or a traversal error occurred; the omission is
/// quantified in `blast_radius_errors`.
pub(crate) fn call_paths_and_blast_radius(
    storage: &dyn Storage,
    file: &Entity,
) -> (Vec<Value>, Vec<Value>, Vec<String>, Vec<String>, bool) {
    use crate::model::EntityTier;
    let children = storage.entities_by_parent(&file.id).unwrap_or_default();
    let func_entities: Vec<Entity> = std::iter::once(file.clone())
        .chain(children.iter().cloned())
        .filter(|e| e.tier == EntityTier::Function)
        .collect();
    // Project boundary: a call path that leaves the project is corruption,
    // not a call (issue #764 — the traversal is the enforcement point).
    let file_project_id = &file.project_id;

    // Dedup key for call_paths rows (issue #835): the id triple
    // (entity_id, target_entity_id, direction). The key is carried
    // alongside each row into a Vec so the sort below can be done on
    // the ACTUAL ids before dropping the target_id column, and the
    // dedup set is checked as rows are built.
    struct CallRow {
        key: (String, String, String),
        json: Value,
    }
    let mut seen_call_edges: HashSet<(String, String, String)> = HashSet::new();
    let mut rows: Vec<CallRow> = Vec::new();
    let mut call_path_errors: Vec<String> = Vec::new();

    // call_paths: outgoing and incoming Calls edges, function granularity,
    // explicit direction (reference shape from issue #759).
    for fe in &func_entities {
        match storage.relationships_from(&fe.id) {
            Ok(outgoing) => {
                for (rel, target) in outgoing {
                    if rel.rel_type != RelType::Calls
                        || !same_project(file_project_id, &target.project_id)
                    {
                        continue;
                    }
                    let key = (fe.id.clone(), rel.target_id.clone(), "out".to_string());
                    if !seen_call_edges.insert(key.clone()) {
                        continue;
                    }
                    rows.push(CallRow {
                        key,
                        json: json!({
                            "entity_id": fe.id,
                            "name": fe.name,
                            "path": fe.path,
                            "calls": target.name,
                            "direction": "out"
                        }),
                    });
                }
            }
            Err(e) => call_path_errors.push(format!("relationships_from({}): {e}", fe.id)),
        }
        match storage.relationships_to(&fe.id) {
            Ok(incoming) => {
                for (rel, source) in incoming {
                    if rel.rel_type != RelType::Calls
                        || !same_project(file_project_id, &source.project_id)
                    {
                        continue;
                    }
                    let key = (source.id.clone(), fe.id.clone(), "in".to_string());
                    if !seen_call_edges.insert(key.clone()) {
                        continue;
                    }
                    rows.push(CallRow {
                        key,
                        json: json!({
                            "entity_id": source.id,
                            "name": source.name,
                            "path": source.path,
                            "calls": fe.name,
                            "direction": "in"
                        }),
                    });
                }
            }
            Err(e) => call_path_errors.push(format!("relationships_to({}): {e}", fe.id)),
        }
    }

    // Deterministic emission order (issue #835): sort by the dedup id
    // triple, which is independent of SQLite iteration order.
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let call_paths: Vec<Value> = rows.into_iter().map(|r| r.json).collect();

    // blast_radius: the reverse dependency closure (issue #834: traversal
    // walks the full dependency set, emission is gated on the emit set).
    let (blast_radius, blast_radius_errors) = reverse_blast_radius(storage, file, &func_entities);
    // Per-symbol completeness signal (issue #854): the closure is complete
    // iff the traversal hit neither the entry cap nor a storage error — the
    // cap disclosure and the error strings both land in
    // `blast_radius_errors`, so a single emptiness check carries both.
    let complete = blast_radius_errors.is_empty();

    (
        call_paths,
        blast_radius,
        call_path_errors,
        blast_radius_errors,
        complete,
    )
}

/// Transitive reverse closure of the dependency edges (Imports, Calls,
/// DependsOn, Implements — never the structural Contains) pointing at the
/// target file entity or any of its Function-tier child entities, to
/// [`BLAST_RADIUS_MAX_HOPS`] hops.
///
/// Traversal walks the full dependency set (`Calls` included, so a
/// call-source at hop 1 is expanded and its hop-2 importers are found).
/// EMISSION, however, is gated on the narrower emit set (issue #834): a
/// file is only listed when it is reached through at least one of Imports,
/// DependsOn, or Implements, and only those rel types are labelled on it.
/// A file reachable ONLY via a calls edge therefore contributes to
/// traversal but produces no blast entry of its own.
///
/// Every emitted entry is normalized to the OWNING FILE entity (deduped on
/// file id, merged across relation types), carries an explicit
/// `direction: "in"` plus the relation type(s) that produced it, and the
/// target file itself can never appear. `entity_id` is intentionally
/// absent: for a File-tier endpoint it is derivable as
/// `{project_id}:{repo_name}:file:{normalized_path}` and adds no signal the
/// caller cannot reconstruct.
///
/// Storage read failures do not abort the traversal — they are returned as
/// strings so the tool can disclose a truncated result rather than
/// presenting an incomplete closure as complete. Entries are capped at
/// [`MAX_BLAST_ENTRIES`] per symbol; when the cap binds, the overflow is
/// disclosed via the returned error strings (one disclosure, with the
/// omitted count and whether it reached hop 0 — issue #854).
///
/// Returns `(entries, errors)`.
pub(crate) fn reverse_blast_radius(
    storage: &dyn Storage,
    target_file: &Entity,
    seed_entities: &[Entity],
) -> (Vec<Value>, Vec<String>) {
    super::core::reverse_blast_radius(
        storage,
        target_file,
        seed_entities,
        BLAST_RADIUS_MAX_HOPS,
        MAX_BLAST_ENTRIES,
        is_dependency_edge,
        is_emit_edge,
    )
}
