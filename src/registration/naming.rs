//! Project-name derivation helpers for fresh registration (issue #29).

/// The owner-prefixed name (`<owner>/<repo>`) extracted from an identity key
/// (`"github.com/owner/repo"` → `"owner/repo"`); falls back to `<dir>-2`
/// (the directory name, `dir_name`) when the owner cannot be determined.
pub(super) fn owner_prefixed_name(identity: &str, dir_name: &str) -> String {
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
