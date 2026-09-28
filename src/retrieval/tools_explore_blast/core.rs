//! Blast-radius (reverse dependency closure) internals for `lievo_explore`
//! (issue #834: extracted from `tools_explore_blast.rs` to keep that file
//! under the 500-line source budget). Holds the entry shape, the hop-2
//! reverse traversal, and the owning-file normalization the traversal
//! depends on. The public API lives in `tools_explore_blast.rs`; the test
//! modules (`tools_explore_blast_tests.rs`,
//! `tools_explore_blast_emit_tests.rs`, `tools_explore_lean_hint_tests.rs`)
//! are declared with plain relative module paths in `mod.rs`.

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

use crate::model::{Entity, EntityTier, RelType};
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;

/// The owning-file walk result. `NotFile` is the graceful-degradation drop
/// for endpoints whose parent chain never reaches a File-tier entity;
/// `LookupFailed(msg)` is a storage read failure, which the caller discloses
/// rather than presenting an incomplete result.
pub(crate) enum OwningFileResult {
    Found(Box<Entity>),
    NotFile,
    LookupFailed(String),
}

/// Maximum parent-chain depth `owning_file` is willing to walk before giving
/// up (corrupt/incomplete entity graphs must not hang the traversal; a
/// malformed cycle in parent_id would otherwise loop forever). Extraction
/// never nests deeper than module -> file -> function, so 16 is far above
/// any legitimate chain.
const OWNING_FILE_MAX_STEPS: usize = 16;

/// The File-tier entity that owns a reached endpoint entity: the entity
/// itself when it is a file, otherwise the first File-tier entity up its
/// FULL parent chain (bounded to `OWNING_FILE_MAX_STEPS` and cycle-guarded,
/// so file -> class -> method reaches the File rather than stopping one
/// level short). The chain is cached in `file_by_id` (pre-populated from
/// the frontier and from prior lookups), so the point-lookup path hits at
/// most once per distinct missing parent.
///
/// Storage read failures are NOT swallowed: on error the function returns
/// `OwningFileResult::LookupFailed` so the caller can disclose the
/// incomplete result instead of silently dropping the endpoint (an error is
/// not an absence).
pub(crate) fn owning_file(
    endpoint: &Entity,
    file_by_id: &mut HashMap<String, Entity>,
    storage: &dyn Storage,
) -> OwningFileResult {
    if endpoint.tier == EntityTier::File {
        return OwningFileResult::Found(Box::new(endpoint.clone()));
    }
    let Some(mut current_id) = endpoint.parent_id.clone() else {
        return OwningFileResult::NotFile;
    };
    let mut seen_in_walk: HashSet<String> = HashSet::new();
    let mut steps = 0;
    loop {
        // Cycle guard: the same id twice means a malformed parent chain.
        if !seen_in_walk.insert(current_id.clone()) {
            return OwningFileResult::NotFile;
        }
        if let Some(file) = file_by_id.get(&current_id) {
            return OwningFileResult::Found(Box::new(file.clone()));
        }
        let parent = match storage.get_entity(&current_id) {
            Ok(Some(p)) => p,
            Ok(None) => return OwningFileResult::NotFile,
            Err(e) => {
                return OwningFileResult::LookupFailed(format!("get_entity({current_id}): {e}"));
            }
        };
        if parent.tier == EntityTier::File {
            file_by_id.insert(current_id, parent.clone());
            return OwningFileResult::Found(Box::new(parent));
        }
        if steps >= OWNING_FILE_MAX_STEPS {
            return OwningFileResult::NotFile;
        }
        steps += 1;
        let Some(next) = parent.parent_id.clone() else {
            return OwningFileResult::NotFile;
        };
        current_id = next;
    }
}

/// One blast_radius entry, file-normalized. `rel_types` is a `Vec<String>`
/// so the array-ness is type-enforced (no `as_array_mut().unwrap()` on a
/// `Value`). `hop` is the 0-based BFS distance (0 = direct dependent,
/// 1 = second level) — the same convention as `get_impact`'s
/// `hop_tracked_dependents`; it is set once at first discovery and never
/// overwritten by the merge branch, so the smallest hop wins.
pub(crate) struct BlastEntry {
    pub(crate) file: Entity,
    pub(crate) rel_types: Vec<String>,
    pub(crate) hop: usize,
}

impl BlastEntry {
    pub(crate) fn new(file: &Entity, rel_type: &str, hop: usize) -> Self {
        Self {
            file: file.clone(),
            rel_types: vec![rel_type.to_string()],
            hop,
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "name": self.file.name,
            "path": self.file.path,
            "tier": self.file.tier.to_string(),
            "direction": "in",
            "hop": self.hop,
            "rel_types": self.rel_types
        })
    }
}

/// The reverse dependency closure traversal (issue #834): walks
/// `target_file`'s Function-tier children plus the file entity up to
/// `max_hops` hops over the dependency set (the `should_expand` gate),
/// creating/merging an entry only for edges where `should_emit` holds.
///
/// `entity_id` is intentionally absent from emitted entries: for a File-tier
/// endpoint it is derivable as `{project_id}:{repo_name}:file:{normalized_path}`
/// and adds no signal the caller cannot reconstruct (issue #834).
///
/// A `visited` set bounds the traversal (cycles terminate): each entity is
/// expanded at most once across all hops. Storage read failures do not
/// abort the traversal — they are returned as strings so the tool can
/// disclose a truncated result rather than presenting an incomplete
/// closure as complete.
pub(crate) fn reverse_blast_radius(
    storage: &dyn Storage,
    target_file: &Entity,
    seed_entities: &[Entity],
    max_hops: usize,
    entry_cap: usize,
    should_expand: impl Fn(&RelType) -> bool,
    should_emit: impl Fn(&RelType) -> bool,
) -> (Vec<Value>, Vec<String>) {
    let target_id = &target_file.id;
    // Project boundary: a blast-radius closure must not cross project
    // boundaries (two projects can share one database; the relationships
    // table has no per-project column, so the traversal itself is the
    // enforcement point — issue #764). Entity ids embed the project id, but
    // the source ENTITY row carries it explicitly — trust the row, not the
    // id.
    let target_project_id = &target_file.project_id;
    let mut file_by_id: HashMap<String, Entity> = HashMap::new();
    if target_file.tier == EntityTier::File {
        file_by_id.insert(target_file.id.clone(), target_file.clone());
    }
    let mut visited: HashSet<String> = HashSet::new();
    // Hop 0 (direct dependents): expand the file entity itself plus every
    // Function-tier entity in the file. A self-edge (entity == target file)
    // is filtered below; a sibling-function edge from the same file is
    // filtered by the owning-file == target check.
    let mut frontier: Vec<Entity> = seed_entities.to_vec();
    // The file entity itself is always a seed (it is File-tier and was
    // filtered out of `seed_entities`, which is Function-tier only).
    if target_file.tier == EntityTier::File {
        frontier.push(target_file.clone());
    }
    let mut entries: Vec<BlastEntry> = Vec::new();
    // file_id -> entry index (for merging rel_types when a file is reached
    // through two different emitting rel types).
    let mut entry_index: HashMap<String, usize> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();
    // Omitted-dependent accounting (issue #854): every dependent the
    // traversal reaches but cannot emit because the entry cap is full is
    // counted here, and `omitted_reached_hop0` tracks whether the omission
    // bit a direct (hop-0) dependent. Fed into the single cap disclosure.
    let mut omitted = 0usize;
    let mut omitted_reached_hop0 = false;
    for hop in 0..max_hops {
        let mut next_frontier: Vec<Entity> = Vec::new();
        for node in &frontier {
            // Per-node fetch: one storage read per frontier node, bounded by
            // the visited set (each node expands at most once across all hops)
            // and the hop cap.
            let incoming = match storage.relationships_to(&node.id) {
                Ok(rows) => rows,
                // Deliberate, not silent: the closure is incomplete —
                // disclose it rather than presenting it as complete.
                Err(e) => {
                    errors.push(format!("relationships_to({}): {e}", node.id));
                    continue;
                }
            };
            for (rel, source) in incoming {
                if !should_expand(&rel.rel_type)
                    || visited.contains(&source.id)
                    // Project boundary: skip cross-project sources (the
                    // traversal is the enforcement point — issue #764).
                    || !same_project(target_project_id, &source.project_id)
                {
                    continue;
                }
                visited.insert(source.id.clone());
                // Normalize to the owning file and skip anything that
                // resolves to the target file itself (self-edges, sibling
                // functions). NotFile: the graceful-degradation drop for
                // endpoints whose parent chain never reaches a File-tier
                // entity (documented in `owning_file`).
                let Some(file) = (match owning_file(&source, &mut file_by_id, storage) {
                    OwningFileResult::Found(f) => Some(*f),
                    OwningFileResult::NotFile => None,
                    OwningFileResult::LookupFailed(msg) => {
                        errors.push(msg);
                        None
                    }
                }) else {
                    continue;
                };
                if file.id == *target_id {
                    continue;
                }
                if file.tier == EntityTier::File {
                    file_by_id.insert(file.id.clone(), file.clone());
                }
                // Emission is gated on the narrower emit set (issue #834),
                // not the broader traversal set: a calls-only edge still
                // pushed the source into `visited` (above) and `next_frontier`
                // (below) so its next-hop importers are still reached, but
                // it no longer labels or creates a blast entry. The guard
                // applies to BOTH the new-entry branch and the merge branch
                // below — a file reached first via an import and then via a
                // call must not get "calls" appended to its rel_types.
                if !should_emit(&rel.rel_type) {
                    if hop + 1 < max_hops {
                        next_frontier.push(source.clone());
                    }
                    continue;
                }
                let rel_label = rel.rel_type.to_string();
                match entry_index.get(&file.id) {
                    Some(idx) => {
                        // Same file reached again through a different edge
                        // type: merge rather than duplicate.
                        let entry = &mut entries[*idx];
                        if !entry.rel_types.iter().any(|t| t.as_str() == rel_label) {
                            entry.rel_types.push(rel_label.clone());
                        }
                    }
                    None => {
                        if entries.len() >= entry_cap {
                            // Cap bound: count every omitted dependent and
                            // record whether the omission reached hop 0 —
                            // disclosed once, below, with the count.
                            omitted += 1;
                            if hop == 0 {
                                omitted_reached_hop0 = true;
                            }
                        } else {
                            entry_index.insert(file.id.clone(), entries.len());
                            entries.push(BlastEntry::new(&file, &rel_label, hop));
                        }
                    }
                }
                if hop + 1 < max_hops {
                    next_frontier.push(source.clone());
                }
            }
        }
        frontier = next_frontier;
        if frontier.is_empty() {
            break;
        }
    }
    // Cap disclosure (issue #854): pushed once, with the count of dependents
    // reached but omitted and whether the omission reached hop 0. Emitted
    // after the traversal so the count is final.
    if omitted > 0 {
        let message = if omitted_reached_hop0 {
            format!(
                "blast_radius capped at {entry_cap} dependents; {omitted} dependents omitted, including direct (hop 0) dependents"
            )
        } else {
            format!(
                "blast_radius capped at {entry_cap} dependents; {omitted} dependents omitted (all second level)"
            )
        };
        errors.push(message);
    }
    (entries.into_iter().map(|e| e.to_json()).collect(), errors)
}
