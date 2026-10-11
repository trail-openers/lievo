use crate::error::Result;
use crate::layout::{index_dir_for_hash, lievo_data_dir, repo_hash};
use std::path::{Path, PathBuf};

/// Get the ts-index directory for a repository.
pub fn ts_index_dir_for_repo(repo_path: &Path) -> Result<PathBuf> {
    let hash = repo_hash(repo_path)?;
    let data_dir = lievo_data_dir()?;
    Ok(index_dir_for_hash(&data_dir, &hash).join("ts-index"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_hash_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().canonicalize().unwrap();

        let hash1 = repo_hash(&repo_path).unwrap();
        let hash2 = repo_hash(&repo_path).unwrap();

        assert_eq!(hash1, hash2, "repo hash should be deterministic");
    }

    #[test]
    fn test_repo_hash_unique_for_different_paths() {
        let temp1 = tempfile::tempdir().unwrap();
        let temp2 = tempfile::tempdir().unwrap();

        let hash1 = repo_hash(temp1.path()).unwrap();
        let hash2 = repo_hash(temp2.path()).unwrap();

        assert_ne!(hash1, hash2, "different repos should have different hashes");
    }

    #[test]
    fn test_index_dir_in_lievo_data() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path();

        // Create a dummy Rust file (to make it look like a repo)
        std::fs::write(
            repo_path.join("test.rs"),
            r#"fn test() { println!("hello"); }"#,
        )
        .unwrap();

        let index_dir = ts_index_dir_for_repo(repo_path).unwrap();

        // Index dir should be in ~/.lievo/indices/
        let expected_parent = lievo_data_dir().unwrap().join("indices");
        assert!(
            index_dir.starts_with(&expected_parent),
            "index_dir {:?} should start with {:?}",
            index_dir,
            expected_parent
        );
    }
}
