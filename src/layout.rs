//! On-disk layout of per-repo state under the lievo data dir (issue #30).
//!
//! The single owner of the path scheme: the data-dir root, the repo hash, and
//! the subpaths derived from both. Relocation moves a checkout's state by
//! hash, so every module that names these paths must agree on them here.
//!
//! Public contract: [`lievo_data_dir`] and [`repo_hash`] are public and stable.
//! Changing the hash scheme renames every on-disk index, so it is an external
//! change, not an internal refactor.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::{LievoError, Result};
use sha2::{Digest, Sha256};

/// Get the lievo data directory (~/.lievo/).
pub fn lievo_data_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        LievoError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "Home directory not found",
        ))
    })?;
    Ok(home.join(".lievo"))
}

/// Hash a repository path to a unique identifier.
pub fn repo_hash(repo_path: &Path) -> Result<String> {
    let canon = repo_path.canonicalize().map_err(|e| {
        LievoError::InvalidInput(format!("Failed to canonicalize repo path: {}", e))
    })?;
    let path_str = canon.to_string_lossy();
    let canonical = CanonicalPath::new(&path_str).ok_or_else(|| {
        LievoError::InvalidInput("canonicalized repo path is not normalised".to_string())
    })?;
    Ok(hash_canonical_path(&canonical))
}

/// An absolute path in normalised spelling: no `.`, no `..`, no repeated or
/// trailing separators. Only this spelling hashes to the key that
/// [`repo_hash`] produces, so the type is the proof a path is safe to hash.
/// Relocation needs it for a stored path that has vanished, which
/// `canonicalize` cannot resolve. Symlinks are not detected lexically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalPath(String);

impl CanonicalPath {
    /// `Some` when `s` is absolute and already in normalised spelling.
    pub(crate) fn new(s: &str) -> Option<Self> {
        let path = Path::new(s);
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return None;
        }
        // `components()` silently drops `.` and repeated separators, so a
        // rebuild that differs from the input means the input was not normalised.
        let rebuilt: PathBuf = path.components().collect();
        (rebuilt.to_str() == Some(s)).then(|| Self(s.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Hash a canonical path to a 16-hex-char identifier.
pub(crate) fn hash_canonical_path(canonical: &CanonicalPath) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_str().as_bytes());
    let hash = hasher.finalize();
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    hex[0..16].to_string()
}

/// Index directory for a repo hash: `<data_dir>/indices/<hash>`.
pub(crate) fn index_dir_for_hash(data_dir: &Path, hash: &str) -> PathBuf {
    data_dir.join("indices").join(hash)
}

/// Lock file path for a repo hash: `<data_dir>/lock-<hash>.lock`.
pub(crate) fn lock_path_for_hash(data_dir: &Path, hash: &str) -> PathBuf {
    data_dir.join(format!("lock-{hash}.lock"))
}

/// Status file path for a repo hash: `<data_dir>/status-<hash>.json`.
pub(crate) fn status_path_for_hash(data_dir: &Path, hash: &str) -> PathBuf {
    data_dir.join(format!("status-{hash}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_path_accepts_only_normalised_absolute_spelling() {
        assert!(CanonicalPath::new("/home/u/repo").is_some());
        assert!(CanonicalPath::new("relative/repo").is_none());
        assert!(CanonicalPath::new("/home/u/../repo").is_none());
        assert!(CanonicalPath::new("/home/u/./repo").is_none());
        assert!(CanonicalPath::new("/home/u//repo").is_none());
        assert!(CanonicalPath::new("/home/u/repo/").is_none());
    }
}
