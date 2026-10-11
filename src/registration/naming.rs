//! Project-name candidates for registration (issue #29). Naming only proposes
//! a name; `attach.rs` decides whether that name is created, reused, or
//! re-picked.
//!
//! Rules: without an identity, `<dir>`, `<dir>-2`, `<dir>-3`, …. With an
//! identity, fresh names are the bare name, then `<owner>/<repo>`,
//! `<owner>/<repo>-2`, …. Fresh naming always finds a free name. The live-twin
//! name is `<repo>@<dir>`, then `<repo>@<dir>-2`, `-3`, … (the first name that
//! is free or already holds this identity).

use super::lookup::project_holds_live_identity;
use crate::storage::Storage;
use std::path::Path;

/// The owner-prefixed name (`<owner>/<repo>`) from an identity key: the last
/// two segments, so `github.com/owner/repo` and `owner/repo` both yield
/// `owner/repo`. `None` when the key has no owner segment.
fn owner_prefixed_name(identity: &str) -> Option<String> {
    let parts: Vec<&str> = identity.split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    let owner = parts[parts.len() - 2];
    let repo = parts[parts.len() - 1];
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// First free `base`, `base-2`, `base-3`, … (no identity, so no owner names).
fn first_free_suffix(storage: &dyn Storage, base: &str) -> crate::Result<String> {
    let mut candidate = base.to_string();
    let mut n = 2;
    while storage.get_project(&candidate)?.is_some() {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    Ok(candidate)
}

/// Name for a fresh (non-twin) registration. An explicit name never reaches
/// here. Only a free name is returned, never a project reused on identity
/// alone (a vanished-only identity holder is not reattached; issue #30).
/// Without an identity: `dir_name`, then `-2`, `-3`, …. With an identity:
/// `dir_name` if free, else the first free of `<owner>/<repo>`,
/// `<owner>/<repo>-2`, …. A derivable owner is always present for real
/// identities; a key without one falls back to the identity-less suffixes.
pub(super) fn pick_fresh_name(
    storage: &dyn Storage,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    if storage.get_project(dir_name)?.is_none() {
        return Ok(dir_name.to_string());
    }
    match identity.and_then(owner_prefixed_name) {
        Some(prefixed) => first_free_suffix(storage, &prefixed),
        None => first_free_suffix(storage, dir_name),
    }
}

/// Name for a live same-identity twin (`<repo>@<dir>`). Returns the first of
/// `base`, `base-2`, `base-3`, … that is free or already held by `identity` at
/// a live checkout, where the checkout joins it. A name held by any other
/// identity, or by a vanished-only holder of this identity, is skipped.
pub(super) fn pick_name(
    storage: &dyn Storage,
    base: &str,
    identity: &str,
) -> crate::Result<String> {
    let mut candidate = base.to_string();
    let mut n = 2;
    loop {
        let usable = match storage.get_project(&candidate)? {
            None => true,
            Some(project) => project_holds_live_identity(storage, &project.id, Some(identity))?,
        };
        if usable {
            return Ok(candidate);
        }
        candidate = format!("{base}-{n}");
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

#[cfg(test)]
mod tests {
    use super::owner_prefixed_name;

    #[test]
    fn owner_prefixed_name_takes_owner_from_second_to_last_segment() {
        assert_eq!(
            owner_prefixed_name("github.com/acme/tool").as_deref(),
            Some("acme/tool")
        );
        assert_eq!(
            owner_prefixed_name("acme/tool").as_deref(),
            Some("acme/tool"),
            "a host-less two-segment key must not yield tool/tool"
        );
        assert_eq!(owner_prefixed_name("tool"), None);
    }
}
