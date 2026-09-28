// Query tool implementations — GetImpactTool, GetHotspotsTool.

use serde_json::{Value, json};

use crate::model::EntityTier;
use crate::retrieval::project_boundary::same_project;
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{GetHotspotsTool, GetImpactTool};
use crate::retrieval::tools_explore_blast::is_dependency_edge;
use crate::storage::Storage;

use super::tools_search::lock_storage;

// ---------------------------------------------------------------------------
// GetImpactTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetImpactTool<S> {
    fn name(&self) -> &str {
        "get_impact"
    }

    fn description(&self) -> &str {
        GET_IMPACT_DESCRIPTION
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of file paths relative to the repo root",
                }
            },
            "required": ["files"]
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let files = input
            .get("files")
            .and_then(|v| v.as_array())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'files' array".into()))?;

        if files.is_empty() {
            // Empty-input early return: carry the same resolution fields as
            // the full path (null — no repo resolved, so no coverage claim;
            // the response shape must not vary by input).
            return Ok(json!({
                "files": [],
                "dependents": [],
                "unresolved_imports": null,
                "resolution_coverage": null
            })
            .to_string());
        }

        let file_paths: Vec<String> = files
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let guard = lock_storage!(self.ctx.storage);
        let storage: &S = &guard;
        let repos = storage.list_repos(&self.ctx.project_id)?;

        let mut repo_id = None;
        for repo in &repos {
            for path in &file_paths {
                if resolve_entity_path(storage, repo, path)?.is_some() {
                    repo_id = Some(repo.id.clone());
                    break;
                }
            }
            if repo_id.is_some() {
                break;
            }
        }
        let repo_id = repo_id.ok_or_else(|| {
            crate::LievoError::EntityNotFound(format!(
                "no entity found for paths in project {}: {}",
                self.ctx.project_id,
                file_paths.join(", ")
            ))
        })?;

        // Resolve every requested path to the stored entity in one pass.
        let repo = repos.into_iter().find(|r| r.id == repo_id).ok_or_else(|| {
            crate::LievoError::EntityNotFound(format!("repo {} not found", repo_id))
        })?;

        let changed_files: Vec<crate::model::Entity> = file_paths
            .iter()
            .filter_map(|path| resolve_entity_path(storage, &repo, path).ok().flatten())
            .collect();

        // Hop-tracked reverse traversal (issue #840), mirroring the
        // `reverse_blast_radius` pattern from the explore blast-radius path:
        // two-hop cap, file-normalized entries, SMALLEST hop kept, only the
        // dependency edge set (Imports/Calls/DependsOn/Implements) walked.
        let dependents = hop_tracked_dependents(storage, &changed_files, &self.ctx.project_id)?;

        // Repo-scoped unresolved-import signal (#681 side-channel). `None`
        // (pre-#681 index, no recorded counts) degrades to null — never a
        // false full-coverage claim.
        let signal = crate::query::dependency::impact_resolution_for_repo(storage, &repo_id);

        let file_paths_out = changed_files
            .iter()
            .map(|e| repo_relative_path(&repo, e))
            .collect::<Vec<_>>();
        let dependents_out: Vec<serde_json::Value> = dependents
            .into_iter()
            .map(|d| {
                let v = json!({"path": repo_relative_path(&repo, &d.file), "hop": d.hop});
                v
            })
            .collect();

        Ok(json!({
            "files": file_paths_out,
            "dependents": dependents_out,
            // #690 honesty signals (preserved by #840): null — not 0, not
            // 1.0 — when no unresolved-import counts were recorded.
            "unresolved_imports": signal.unresolved_internal.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
            "resolution_coverage": resolution_coverage_value(&signal),
        })
        .to_string())
    }
}

/// One hop-tracked, file-normalized dependent: the owning file entity and
/// the SMALLEST hop distance at which it was reached.
struct HopDependent {
    file: crate::model::Entity,
    hop: u8,
}

/// The hop-tracked reverse dependency walk behind the lean get_impact
/// contract (issue #840).
///
/// Semantics (documented in the tool description; consistent with
/// `reverse_blast_radius` in the explore blast-radius path):
/// - Seeds are the requested changed files plus every Function-tier child of
///   each file, so function-tier callers fold into their owning file.
/// - Each hop walks `relationships_to` over the dependency edge set only
///   (Imports, Calls, DependsOn, Implements) — the same set that gates
///   `reverse_blast_radius` traversal; all other edge types are ignored.
/// - Every source endpoint is file-normalized via `owning_file` (function
///   tier rolls up into its containing file); endpoints with no File-tier
///   ancestor are dropped.
/// - Two-hop cap (hop 0 = direct dependent, hop 1 = dependent of a
///   dependent), matching `BLAST_RADIUS_MAX_HOPS`.
/// - The SMALLEST hop wins when a file is reached at more than one distance.
/// - Project boundary (issue #764): each hop's endpoints are filtered
///   against the caller's project — the relationships table has no
///   project_id column, so the traversal is the enforcement point.
fn hop_tracked_dependents<S: Storage>(
    storage: &S,
    changed_files: &[crate::model::Entity],
    project_id: &str,
) -> crate::Result<Vec<HopDependent>> {
    // Self-normalization guard (checked by id): the set of every requested
    // changed file, used by the per-edge closure below.
    let changed_ids: std::collections::HashSet<String> =
        changed_files.iter().map(|c| c.id.clone()).collect();

    // Seed frontier: the file entities themselves plus their Function-tier
    // children (function-tier callers must roll up into the file, mirroring
    // the blast-radius path's seed construction).
    let mut file_by_id: std::collections::HashMap<String, crate::model::Entity> =
        std::collections::HashMap::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut frontier: Vec<crate::model::Entity> = Vec::new();
    for file in changed_files {
        if visited.insert(file.id.clone()) {
            file_by_id.insert(file.id.clone(), file.clone());
            frontier.push(file.clone());
        }
        for child in storage.entities_by_parent(&file.id)? {
            if child.tier == crate::model::EntityTier::Function && visited.insert(child.id.clone())
            {
                frontier.push(child);
            }
        }
    }

    // Self-normalization guard: a file that is itself one of the requested
    // changed files cannot be its own dependent — but its incoming edges
    // must still be walked (it may be an intermediate hop toward a further
    // dependent), so skip the emission, not the traversal.
    let is_changed_file = |file_id: &str| changed_ids.contains(file_id);

    let mut hops: std::collections::HashMap<String, u8> = std::collections::HashMap::new();
    for hop in 0..2u8 {
        let mut next_frontier: Vec<crate::model::Entity> = Vec::new();
        for node in &frontier {
            let incoming = match storage.relationships_to(&node.id) {
                Ok(rows) => rows,
                Err(e) => {
                    return Err(crate::LievoError::RetrievalError(format!(
                        "relationships_to({}): {e}",
                        node.id
                    )));
                }
            };
            for (rel, source) in incoming {
                if !is_dependency_edge(&rel.rel_type)
                    || !same_project(project_id, &source.project_id)
                    || visited.contains(&source.id)
                {
                    continue;
                }
                visited.insert(source.id.clone());
                // File-normalization (issue #840 gap-gate #4): one entry per
                // owning file, function-tier callers merged into their
                // containing file; endpoints with no File-tier ancestor drop
                // (graceful degradation, same as reverse_blast_radius).
                let file = match crate::retrieval::tools_explore_blast::owning_file(
                    &source,
                    &mut file_by_id,
                    storage,
                ) {
                    crate::retrieval::tools_explore_blast::OwningFileResult::Found(f) => {
                        // Ensure the owning file is in file_by_id so the
                        // final hops→entity mapping can find it (issue #840).
                        // `owning_file` for File-tier endpoints returns the
                        // endpoint directly without touching file_by_id, so
                        // this insert is critical.
                        file_by_id.entry(f.id.clone()).or_insert((*f).clone());
                        *f
                    }
                    crate::retrieval::tools_explore_blast::OwningFileResult::NotFile => continue,
                    crate::retrieval::tools_explore_blast::OwningFileResult::LookupFailed(_) => {
                        continue;
                    }
                };
                if !is_changed_file(&file.id) {
                    // SMALLEST hop wins: keep the earliest distance recorded.
                    hops.entry(file.id.clone()).or_insert(hop);
                }
                // A changed file is still traversed (it may be an
                // intermediate hop toward a further dependent), never emitted.
                next_frontier.push(source);
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    let mut out: Vec<HopDependent> = hops
        .into_iter()
        .filter_map(|(id, hop)| {
            file_by_id
                .get(&id)
                .cloned()
                .map(|file| HopDependent { file, hop })
        })
        .collect();
    out.sort_by(|a, b| a.file.id.cmp(&b.file.id));
    Ok(out)
}

/// Repo-relative form of an entity path: the stored path is typically
/// absolute (under `repo.local_path`, issue #644), but relative stored
/// paths are already repo-relative and pass through unchanged. Relative
/// paths that cannot be stripped (e.g. Windows drives on Linux) fall back
/// to the stored form rather than emitting garbage.
fn repo_relative_path(repo: &crate::model::Repository, entity: &crate::model::Entity) -> String {
    let path = entity.path.clone().unwrap_or_else(|| entity.name.clone());
    if path.starts_with(&repo.local_path) {
        return path[repo.local_path.len()..]
            .trim_start_matches('/')
            .to_string();
    }
    path
}

// ---------------------------------------------------------------------------
// GetHotspotsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetHotspotsTool<S> {
    fn name(&self) -> &str {
        "get_hotspots"
    }

    fn description(&self) -> &str {
        "Return the highest-complexity entities in the codebase sorted by complexity score. Use to identify risky code, focus refactoring effort, or orient in an unfamiliar codebase."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 10, max: 50)",
                    "default": 10
                },
                "tier": {
                    "type": "string",
                    "description": "Entity tier to filter by. Valid values: 'file', 'module', 'subsystem'. Default: 'file'.",
                    "default": "file",
                    "enum": ["file", "module", "subsystem"]
                }
            }
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
        if limit == 0 {
            return Err(crate::LievoError::InvalidInput(
                "limit must be greater than 0".into(),
            ));
        }
        let limit = limit.min(50) as usize;
        let tier_filter = input.get("tier").and_then(|v| v.as_str()).unwrap_or("file");

        // Parse and validate tier value at SQL level
        let entity_tier = match tier_filter.to_lowercase().as_str() {
            "file" => EntityTier::File,
            "module" => EntityTier::Module,
            "subsystem" => EntityTier::Subsystem,
            _ => {
                return Err(crate::LievoError::InvalidInput(format!(
                    "invalid tier '{}': must be one of 'file', 'module', 'subsystem'",
                    tier_filter
                )));
            }
        };

        let guard = lock_storage!(self.ctx.storage);
        let mut entities = guard.list_entities(&self.ctx.project_id, Some(entity_tier))?;

        entities.sort_by(|a, b| {
            let ca = extract_complexity(a);
            let cb = extract_complexity(b);
            cb.partial_cmp(&ca)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });
        entities.truncate(limit);

        let lines: Vec<String> = entities
            .iter()
            .map(|e| {
                let complexity = extract_complexity(e);
                json!({
                    "id": e.id,
                    "name": e.name,
                    "tier": e.tier.to_string(),
                    "path": e.path,
                    "complexity_max": complexity,
                })
                .to_string()
            })
            .collect();

        Ok(lines.join("\n"))
    }
}

/// Resolve a user-supplied file path to the stored file entity of a repository.
///
/// Exact match first (absolute paths, or relative paths when the DB was seeded
/// with relative paths). When that misses, a non-absolute path is resolved
/// against the repo root and looked up again — the real storage format is the
/// absolute path under `repo.local_path` (issue #644).
fn resolve_entity_path<S: Storage>(
    guard: &S,
    repo: &crate::model::Repository,
    path: &str,
) -> crate::Result<Option<crate::model::Entity>> {
    if let Some(entity) = guard.entity_by_path(&repo.id, path)? {
        return Ok(Some(entity));
    }
    let trimmed = path.strip_prefix("./").unwrap_or(path);
    if std::path::Path::new(trimmed).is_absolute() {
        return Ok(None);
    }
    let absolute = std::path::Path::new(&repo.local_path)
        .join(trimmed)
        .to_string_lossy()
        .to_string();
    guard.entity_by_path(&repo.id, &absolute)
}

/// Shape the repo-scoped resolution signal into a JSON `resolution_coverage`
/// value. Per the #690 PM decision, coverage is reported as `null` whenever
/// the unresolved-import counts were not recorded (pre-#681 index) — never a
/// false `1.0` (full-coverage) claim. Once counts exist, coverage is
/// `1.0` for a fully-resolved repo and `0.0` otherwise, the minimum honest
/// claim without a per-import resolved/total side-channel (out of scope for
/// #690 per the PM data-source decision — #681 exposes only unresolved counts).
pub fn resolution_coverage_value(
    signal: &crate::query::dependency::ResolutionSignal,
) -> serde_json::Value {
    match signal.unresolved_internal {
        None => serde_json::Value::Null,
        Some(0) => serde_json::json!(1.0),
        Some(_) => serde_json::json!(0.0),
    }
}

/// The get_impact tool description shared verbatim with the MCP
/// `#[tool(description = ...)]` attribute at `src/mcp/tools.rs` (issue #840
/// gap-gate #5: one literal, no drift).
///
/// `hop` counts the four dependency edge types (Imports, Calls, DependsOn,
/// Implements), 0 = direct dependent, 1 = second-hop, consistent with the
/// blast-radius traversal in `lievo_explore`.
pub(crate) const GET_IMPACT_DESCRIPTION: &str = "Analyse the impact of changing one or more files. Returns {files, dependents:[{path, hop}], unresolved_imports, resolution_coverage}. Each dependent is a repo-relative path with hop 0 (direct) or 1 (second-hop), counting import, call, depends-on, and implements edges. unresolved_imports/resolution_coverage are null when not recorded; an empty dependents list with unresolved_imports > 0 does NOT mean dead code.";

fn extract_complexity(entity: &crate::model::Entity) -> f64 {
    let json = match entity.metrics_json.as_deref() {
        Some(s) => s,
        None => return 0.0,
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("complexity_max").and_then(|c| c.as_f64()))
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "tools_query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_query_lean_tests.rs"]
mod tools_query_lean_tests;
