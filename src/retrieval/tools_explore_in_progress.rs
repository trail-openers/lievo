//! Success-shaped "indexing in progress" response for `lievo_explore`
//! (issue #864).
//!
//! When a background first index or incremental refresh is running — in
//! this process or any other `lievo mcp` process on the same repo — the
//! server must report it honestly instead of telling the agent to run
//! `lievo refresh` (which the agent cannot do while the MCP server is
//! running). This module probes the per-repo cross-process lock (the only
//! authoritative signal that a live holder exists) and either:
//!
//! - Returns `None` if no live holder — let the call fall through;
//! - Returns a success-shaped JSON with `indexing: true`, the `trigger`,
//!   `started_at` (unix epoch secs) and `elapsed_secs`, and (for a first
//!   index with no partial index yet) an explicit "use built-in tools for
//!   now and retry later" hint. If a partial index exists (entities
//!   already present), the response also carries an `index_incomplete:
//!   true` flag so the caller can still use what is there without
//!   presenting a partial index as complete (#838/#848).

use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use serde_json::json;
use std::path::Path;

/// Build the success-shaped response (or `None` if indexing is not running).
///
/// The `ctx.repo_path` must be a valid, existing path — if it is empty the
/// caller is in zero-repo mode, which is handled by `zero_repo_guidance`
/// before this function is reached.
pub(crate) fn indexing_in_progress_response<S: Storage>(
    ctx: &ToolContext<S>,
) -> Option<crate::Result<String>> {
    if ctx.repo_path.as_os_str().is_empty() {
        return None;
    }
    let status = match crate::refresh::indexing_status(Path::new(ctx.repo_path.as_os_str())) {
        Ok(s) => s,
        Err(_) => return None, // Probe failed: fall through silently.
    };
    let (started_at, trigger_label, elapsed) = match &status {
        crate::refresh::IndexingStatus::NotRunning => return None,
        crate::refresh::IndexingStatus::Running {
            started_at,
            trigger,
        } => (
            Some(*started_at),
            Some(trigger.as_str()),
            status.elapsed_secs(),
        ),
        crate::refresh::IndexingStatus::RunningUnknown => (None, None, None),
    };

    // Determine whether a partial index exists (entities already present).
    // A poisoned storage lock is not an error worth surfacing here — fall
    // back to the no-partial-index guidance (the in-progress case is the
    // primary signal).
    let partial = {
        let guard = ctx
            .storage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !guard
            .list_entities(&ctx.project_id, None)
            .unwrap_or_default()
            .is_empty()
    };

    // Guidance text: the first-index-with-no-partial-index case must tell
    // the agent to use its built-in tools and retry later; the partial case
    // can mention the partial data. Never mention `lievo refresh` (issue
    // #864 binding decision: the agent cannot run lievo commands while
    // the MCP server is running, and indexing is automatic).
    let guidance = if partial {
        "Indexing is in progress; results may be incomplete until it finishes. Retry in a moment for complete data.".to_string()
    } else {
        "Indexing is in progress — no results are available yet. Use your built-in tools for now and retry this call in a moment.".to_string()
    };

    let mut response = json!({
        "symbols": [],
        "indexing": true,
        "warning": guidance,
    });
    if let Some(s) = started_at {
        response["started_at"] = json!(s);
    }
    if let Some(t) = trigger_label {
        response["trigger"] = json!(t);
    }
    if let Some(e) = elapsed {
        response["elapsed_secs"] = json!(e);
    }
    if partial {
        response["index_incomplete"] = json!(true);
    }

    Some(Ok(response.to_string()))
}
