//! Scope-membership listing mode for `lievo_explore` (issue #712).
//!
//! When the caller passes an explicit `scope` (repo-relative directory
//! prefix), the tool switches from word-match search to a recursive listing
//! of the INDEXED file entities under that prefix — the shape CodeGraph
//! calls `codegraph_files` — sorted, repo-relative, with no symbol-building.
//! `scope` is the SOLE deterministic trigger (PM decision 2026-09-12): a
//! path-like `query` string never activates this mode on its own.
//!
//! Extracted from `tools_explore.rs` to keep that file under the 500-line
//! budget (issue #712 edge-case note).

use std::path::Component;

use serde_json::{Value, json};

use crate::model::{Entity, EntityTier};
use crate::retrieval::explore_cap::cap_response;
use crate::retrieval::explore_common::{continuation_pointer, lock_storage, should_exclude_entity};
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;
use crate::storage::Storage;

/// Same containment check as the test-only `is_within_scope` convenience
/// wrapper below, but takes a pre-computed `scope_slash` (`"{scope}/"`) so
/// `scope_listing`'s per-entity hot loop allocates the anchored prefix once
/// per request instead of once per entity comparison (issue #712 perf
/// finding: ~2N `format!` allocations otherwise).
fn is_within_scope_with_prefix(path: &str, scope: &str, scope_slash: &str) -> bool {
    path == scope || path.starts_with(scope_slash)
}

/// True when `path` is exactly `scope` or nested under it (slash-anchored
/// containment, distinct from `should_exclude_entity`'s output-dir filter):
/// scope "src" matches "src/main.rs" and "src" itself, but not "src2/x.rs".
/// Single-comparison convenience wrapper over `is_within_scope_with_prefix`
/// for the boundary tests; production code (`scope_listing`) precomputes
/// the prefix once per request instead of calling this per entity.
#[cfg(test)]
pub(crate) fn is_within_scope(path: &str, scope: &str) -> bool {
    is_within_scope_with_prefix(path, scope, &format!("{scope}/"))
}

/// Reject scope prefixes that attempt to traverse outside the repo root
/// (`..`, absolute paths, Windows drive prefixes) — mirrors
/// `ListDirectoryTool`'s traversal guard (`tools_directory.rs`).
fn is_path_traversal(scope: &str) -> bool {
    std::path::Path::new(scope).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// Normalize a scope prefix: trim a trailing slash and a leading `./` so
/// "src/", "./src", and "src" all compare equal against stored paths.
/// A scope that is blank (whitespace-only) or normalizes to `.` or `./`
/// (the repo root itself, spelled as a literal dot rather than absent) is
/// treated as EMPTY: `.` carries no discriminating information — every
/// indexed file is "under" `.` — so it takes the same invalid/absent-scope
/// guidance path as `""` or `"   "`, instead of falling through to a
/// confusing "No indexed files under '.'" warning (issue #712).
fn normalize_scope(scope: &str) -> &str {
    let trimmed = scope.trim().trim_end_matches('/');
    let stripped = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if stripped.is_empty() || stripped == "." {
        ""
    } else {
        stripped
    }
}

/// Success-shaped guidance for a scope with no indexed files under it —
/// either the prefix does not exist in the index, or every match was
/// excluded (output_dir). `is_error` stays false per #680's error contract.
fn empty_scope_guidance(scope: &str, warning: String) -> Value {
    json!({
        "files": [],
        "returned": 0,
        "total": 0,
        "scope": scope,
        "warning": warning,
    })
}

/// Entry point: list indexed files under `scope`, paged by `offset`/`max_files`.
/// Returns the serialized JSON response body (never `Err` for a recoverable
/// condition — unknown/excluded scopes are success-shaped guidance).
pub(crate) fn scope_listing<S: Storage>(
    storage: &S,
    ctx: &ToolContext<S>,
    scope: &str,
    offset: usize,
    max_files: usize,
) -> crate::Result<String> {
    if is_path_traversal(scope) {
        return Ok(empty_scope_guidance(
            scope,
            format!("Invalid scope '{scope}': path traversal is not allowed."),
        )
        .to_string());
    }

    let normalized = normalize_scope(scope);
    if normalized.is_empty() {
        return Ok(empty_scope_guidance(
            scope,
            "Invalid scope: empty scope after normalization.".to_string(),
        )
        .to_string());
    }

    // Real storage/index faults (DB failure, poisoned index) propagate as
    // `Err` instead of being swallowed into "no files indexed" guidance
    // (issue #712): that guidance is reserved for a genuinely-empty result,
    // not an infrastructure fault masquerading as one (#680's success-shaped
    // contract covers recoverable conditions, not storage errors).
    let all_files = storage.list_entities(&ctx.project_id, Some(EntityTier::File))?;

    // Single scan over `all_files`: scope-containment is checked once per
    // entity (with the anchored prefix precomputed once, not once per
    // entity) and the output-dir exclusion filter runs only on that already-
    // narrowed subset, instead of two full passes over every indexed file.
    let scope_slash = format!("{normalized}/");
    let prefix_matched: Vec<&Entity> = all_files
        .iter()
        .filter(|e| matches!(e.path.as_deref(), Some(p) if is_within_scope_with_prefix(p, normalized, &scope_slash)))
        .collect();

    let mut matched: Vec<&Entity> = prefix_matched
        .iter()
        .copied()
        .filter(|e| !should_exclude_entity(e.path.as_deref(), &ctx.output_dir))
        .collect();

    if matched.is_empty() {
        // `any_prefix_hit` is only needed in this (empty-result) branch, so
        // it is derived from the already-computed `prefix_matched` count
        // rather than a separate `.any()` scan over `all_files`.
        let any_prefix_hit = !prefix_matched.is_empty();
        let warning = if any_prefix_hit {
            format!(
                "No indexed files under '{scope}' after excluding the output directory. \
                 All matches were excluded."
            )
        } else {
            format!(
                "No indexed files under '{scope}'. Run 'lievo refresh' to rebuild the index, \
                 or use a word-match query."
            )
        };
        return Ok(empty_scope_guidance(scope, warning).to_string());
    }

    // Deterministic total order: lexicographic repo-relative path (git
    // ls-files shape), independent of storage insertion order.
    matched.sort_by(|a, b| {
        a.path
            .as_deref()
            .unwrap_or("")
            .cmp(b.path.as_deref().unwrap_or(""))
    });

    let total = matched.len();
    let page: Vec<&Entity> = matched.into_iter().skip(offset).take(max_files).collect();
    let returned = page.len();

    let files: Vec<Value> = page
        .iter()
        .map(|e| {
            json!({
                "entity_id": e.id,
                "path": e.path,
            })
        })
        .collect();

    let mut response = json!({
        "files": files,
        "returned": returned,
        "total": total,
        "scope": scope,
    });

    if offset >= total && total > 0 {
        // Paged past the end: distinguish this from a genuinely-empty scope
        // (that guidance already returned above) so an agent doesn't read a
        // bare empty page as "nothing under this scope" (issue #712).
        response["warning"] = json!(format!(
            "offset {offset} is past the end of the scope listing ({total} total files). \
             Retry with a smaller offset."
        ));
    }

    let next_offset = offset + returned;
    if next_offset < total {
        let next_tool =
            format!("lievo_explore(scope='{scope}', offset={next_offset}, max_files={max_files})");
        response["continuation"] = json!(continuation_pointer(returned, total, &next_tool));
        response["completeness"] = json!(format!("showing {returned} of {total} files in scope"));
    }

    let serialized = response.to_string();
    if serialized.chars().count() > MAX_EXPLORE_OUTPUT_CHARS
        && let Some(capped) = cap_response(&response)
    {
        return Ok(capped);
    }

    Ok(serialized)
}

/// Read `scope`/`offset` from the call() input and, when `scope` is present
/// and non-empty, run scope-membership listing instead of word-match. Called
/// at the top of `ExploreTool::call` before `matching_entities` runs.
/// Returns `None` when `scope` is absent (or blank), signaling the caller to
/// fall through to the word-match path unchanged.
pub(crate) fn maybe_scope_listing<S: Storage>(
    ctx: &std::sync::Arc<ToolContext<S>>,
    input: &Value,
    max_files: usize,
) -> Option<crate::Result<String>> {
    let scope = input.get("scope").and_then(|v| v.as_str())?;
    if scope.trim().is_empty() {
        return None;
    }

    let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let result = (|| -> crate::Result<String> {
        let guard = lock_storage!(ctx.storage);
        scope_listing(&*guard, ctx, scope, offset, max_files)
    })();
    Some(result)
}

#[cfg(test)]
#[path = "tools_explore_scope_tests.rs"]
mod tools_explore_scope_tests;

#[cfg(test)]
#[path = "tools_explore_scope_listing_tests.rs"]
mod tools_explore_scope_listing_tests;
