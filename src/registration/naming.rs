//! Project-name candidates for registration (issue #29). Naming only proposes
//! a name; `attach.rs` decides whether that name is created, reused, or
//! re-picked.

use super::lookup::project_holds_identity;
use crate::storage::Storage;
use std::path::Path;

/// The owner-prefixed name (`<owner>/<repo>`) extracted from an identity key
/// (`"github.com/owner/repo"` → `"owner/repo"`); falls back to `<dir>-2`
/// (the directory name, `dir_name`) when the owner cannot be determined.
fn owner_prefixed_name(identity: &str, dir_name: &str) -> String {
    let parts: Vec<&str> = identity.split('/').collect();
    let repo = parts.last().copied().unwrap_or("");
    if let Some(owner) = parts.get(1)
        && !owner.is_empty()
        && !repo.is_empty()
    {
        return format!("{owner}/{repo}");
    }
    format!("{dir_name}-2")
}

/// Candidate `n` of `base` in the single suffix sequence: `base`, then
/// `base-2`, `base-3`, …
fn candidate_name(base: &str, n: usize) -> String {
    if n == 0 {
        base.to_string()
    } else {
        format!("{base}-{}", n + 1)
    }
}

/// Name for a fresh (non-twin) registration. An explicit name never reaches
/// here. Precedence: the bare `dir_name` when free and an identity exists or
/// none does; with an identity, `<owner>/<repo>` once `dir_name` is taken;
/// otherwise the suffix sequence on `dir_name`.
pub(super) fn pick_fresh_name(
    storage: &dyn Storage,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    let base = match identity {
        None => dir_name.to_string(),
        Some(identity) => {
            if storage.get_project(dir_name)?.is_none() {
                return Ok(dir_name.to_string());
            }
            owner_prefixed_name(identity, dir_name)
        }
    };
    pick_name(storage, &base, identity)
}

/// First candidate of `base` that attach will accept: a name no project holds,
/// or one already held by `identity`. Selects only; it creates and attaches
/// nothing.
pub(super) fn pick_name(
    storage: &dyn Storage,
    base: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    let base = if base.is_empty() { "repo" } else { base };
    let mut n = 0;
    loop {
        let candidate = candidate_name(base, n);
        match storage.get_project(&candidate)? {
            None => return Ok(candidate),
            Some(project) => {
                if project_holds_identity(storage, &project.id, identity)? {
                    return Ok(candidate);
                }
            }
        }
        n += 1;
    }
}

/// The directory name for a repo root (used as the repo row's `name`, stored
/// raw).
pub(super) fn repo_dir_name(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

/// The short repo name extracted from an identity key
/// (`"github.com/owner/repo"` → `"repo"`). Falls back to `"repo"` for
/// malformed keys.
pub(super) fn repo_short_name(identity: &str) -> String {
    identity
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "repo".to_string())
}
