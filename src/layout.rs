//! On-disk layout of per-repo state under the lievo data dir (issue #30).
//!
//! The single owner of the path scheme. Relocation moves a checkout's state by
//! hash, so every module that names these paths must agree on them here.

use std::path::{Path, PathBuf};

use crate::error::{LievoError, Result};
use sha2::{Digest, Sha256};

/// Hash a repository path to a unique identifier.
pub fn repo_hash(repo_path: &Path) -> Result<String> {
    let path_str = repo_path
        .canonicalize()
        .map_err(|e| LievoError::InvalidInput(format!("Failed to canonicalize repo path: {}", e)))?
        .to_string_lossy()
        .to_string();
    Ok(hash_canonical_path(&path_str))
}

/// Hash an already-canonical path string. Relocation (issue #30) needs this for
/// a stored path that has vanished, which `canonicalize` cannot resolve.
///
/// The input must be canonical: a non-canonical spelling (symlink, older
/// format) hashes to a different key.
pub fn hash_canonical_path(canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
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
