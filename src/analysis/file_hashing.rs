// File hashing utilities for incremental change detection.

use crate::error::Result;
use crate::extraction::grouping::GroupingResult;
use crate::model::Repository;
use crate::storage::Storage;
use std::path::Path;

/// Compute and store file content hashes for incremental detection.
/// Uses DefaultHasher (same as cache.rs) to hash file contents.
pub fn store_file_hashes(
    storage: &dyn Storage,
    repo: &Repository,
    repo_path: &Path,
    grouping: &GroupingResult,
) -> Result<()> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    for file_entity in &grouping.files {
        if let Some(file_path) = &file_entity.path {
            let full_path = repo_path.join(file_path);
            if let Ok(bytes) = std::fs::read(&full_path) {
                let mut hasher = DefaultHasher::new();
                bytes.hash(&mut hasher);
                let content_hash = format!("{:x}", hasher.finish());
                storage.upsert_file_hash(&repo.id, file_path, &content_hash)?;
            }
        }
    }

    Ok(())
}
