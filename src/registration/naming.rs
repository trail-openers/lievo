//! Project-name derivation for registration (issue #29).

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

/// Resolve the project name for a fresh registration.
///
/// Precedence (issue #29 naming rules):
/// - An explicit project name (already trimmed and non-empty) is used as-is.
///   The taken-by-different-identity collision is checked in `register`
///   before reaching here.
/// - Bare `<dir>` if free.
/// - `<owner>/<repo>` (with a deterministic suffix if taken) when the short
///   name is taken by a different identity AND a derivable identity exists.
/// - `name-2`, `name-3`, … for repos with no derivable identity.
///
/// This only *selects* a name; the caller attaches via `attach_or_create`.
pub(super) fn pick_fresh_project_name(
    storage: &dyn Storage,
    explicit_name: Option<&str>,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    if let Some(explicit) = explicit_name {
        return Ok(explicit.to_string());
    }
    if storage.get_project(dir_name)?.is_none() {
        return Ok(dir_name.to_string());
    }
    if let Some(identity) = identity {
        let owner_prefixed = owner_prefixed_name(identity, dir_name);
        return pick_available_project_name(storage, &owner_prefixed, Some(identity));
    }
    pick_available_project_name(storage, dir_name, None)
}

/// Pick a free project name starting from `base`, trying `base-2`,
/// `base-3`, … if needed. If `identity` is `Some` and a candidate of that
/// name already exists owned by a repo with the SAME identity, reuse it
/// (idempotent: an already-taken suffix slot carrying the same identity is
/// attached to rather than re-suffixed); otherwise keep suffixing.
pub(super) fn pick_available_project_name(
    storage: &dyn Storage,
    base: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    let base = if base.is_empty() {
        "repo".to_string()
    } else {
        base.to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2;
    loop {
        match storage.get_project(&candidate)? {
            None => return Ok(candidate),
            Some(p) => {
                let repos = storage.list_repos(&p.id)?;
                let same_identity = identity
                    .is_some_and(|id| repos.iter().any(|r| r.git_url.as_deref() == Some(id)));
                if same_identity {
                    return Ok(candidate);
                }
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
        }
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
