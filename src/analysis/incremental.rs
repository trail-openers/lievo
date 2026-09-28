// Incremental change detection via git2.
// Provides: head_commit() for current HEAD hash, changed_files() for diff between commits.

use crate::error::{LievoError, Result};
use std::path::Path;

/// Get the current HEAD commit hash for a repository.
pub fn head_commit(repo_path: &Path) -> Result<String> {
    let repo = git2::Repository::open(repo_path)?;
    let head = repo.head()?;
    let commit = head.peel_to_commit()?;
    Ok(commit.id().to_string())
}

/// Get the list of changed file paths between two commits.
///
/// When `from_commit` is `None`, returns all files in `to_commit` (first-run case).
pub fn changed_files(
    repo_path: &Path,
    from_commit: Option<&str>,
    to_commit: &str,
) -> Result<Vec<String>> {
    let repo = git2::Repository::open(repo_path)?;

    let to_oid = repo
        .revparse_single(to_commit)
        .map_err(LievoError::Git)?
        .peel_to_commit()
        .map_err(LievoError::Git)?;
    let to_tree = to_oid.tree().map_err(LievoError::Git)?;

    let old_tree = match from_commit {
        Some(from) => {
            let from_oid = repo
                .revparse_single(from)
                .map_err(LievoError::Git)?
                .peel_to_commit()
                .map_err(LievoError::Git)?;
            Some(from_oid.tree().map_err(LievoError::Git)?)
        }
        None => None,
    };

    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&to_tree), None)?;

    let mut files = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            // For deleted files use old_file path; for all others use new_file path.
            // Deleted files matter because entities from them must be removed.
            let path = if delta.status() == git2::Delta::Deleted {
                delta.old_file().path()
            } else {
                delta.new_file().path()
            };
            if let Some(p) = path
                && let Some(s) = p.to_str()
            {
                files.push(s.to_string());
            }
            true
        },
        None,
        None,
        None,
    )?;

    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{add_commit, make_repo_with_commit};
    use super::*;

    #[test]
    fn test_head_commit_returns_hash() {
        let (dir, expected) = make_repo_with_commit(&[("a.rs", "fn main() {}")]);
        let hash = head_commit(dir.path()).unwrap();
        assert_eq!(hash, expected);
        assert_eq!(hash.len(), 40);
    }

    #[test]
    fn test_head_commit_invalid_path_returns_error() {
        let result = head_commit(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_changed_files_no_from_returns_all_files() {
        let (dir, commit) = make_repo_with_commit(&[("a.rs", "x"), ("b.rs", "y")]);
        let files = changed_files(dir.path(), None, &commit).unwrap();
        assert!(files.contains(&"a.rs".to_string()));
        assert!(files.contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_changed_files_between_commits_returns_diff() {
        let (dir, first) = make_repo_with_commit(&[("a.rs", "x"), ("b.rs", "y")]);
        let second = add_commit(dir.path(), &[("c.rs", "z")]);
        let files = changed_files(dir.path(), Some(&first), &second).unwrap();
        assert!(files.contains(&"c.rs".to_string()));
        assert!(!files.contains(&"a.rs".to_string()));
        assert!(!files.contains(&"b.rs".to_string()));
    }

    #[test]
    fn test_changed_files_same_commit_returns_empty() {
        let (dir, commit) = make_repo_with_commit(&[("a.rs", "x")]);
        let files = changed_files(dir.path(), Some(&commit), &commit).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_changed_files_invalid_commit_returns_error() {
        let (dir, _) = make_repo_with_commit(&[("a.rs", "x")]);
        let result = changed_files(dir.path(), None, "deadbeefdeadbeefdeadbeef");
        assert!(result.is_err());
    }

    #[test]
    fn test_changed_files_result_is_sorted_and_deduplicated() {
        let (dir, first) = make_repo_with_commit(&[("a.rs", "x"), ("b.rs", "y"), ("c.rs", "z")]);
        let second = add_commit(dir.path(), &[("d.rs", "w"), ("a.rs", "updated")]);
        let files = changed_files(dir.path(), Some(&first), &second).unwrap();
        // Result must be sorted
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
        // No duplicates
        let unique: std::collections::HashSet<_> = files.iter().collect();
        assert_eq!(files.len(), unique.len());
    }
}
