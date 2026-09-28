//! Discovery and budget-based selection of existing project documentation.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{LievoError, Result};

use super::doc_discovery_helpers::{
    collect_markdown_recursive, file_modified_unix, is_markdown_file,
};

const LARGE_DOC_THRESHOLD_BYTES: u64 = 50 * 1024;
const LARGE_DOC_PENALTY: i32 = 20;

/// Default context budget for selected source docs.
pub const DEFAULT_DOC_SOURCE_BUDGET: usize = 200000;

/// Candidate documentation file discovered on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDoc {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub included_bytes: Option<u64>,
    pub relevance_score: i32,
    pub selected: bool,
    pub skip_reason: Option<String>,
}

/// Excerpt from an existing documentation file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingDocExcerpt {
    pub path: String,
    pub content: String,
    pub relevance_score: f64,
}

/// Discover existing documentation files for the given project path.
pub fn discover_existing_docs(project_path: &Path) -> Result<Vec<DiscoveredDoc>> {
    if !project_path.is_dir() {
        return Err(LievoError::InvalidInput(format!(
            "project path '{}' is not a directory",
            project_path.display()
        )));
    }

    let canonical_root = match project_path.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            return Err(LievoError::InvalidInput(format!(
                "failed to read project root '{}': {err}",
                project_path.display()
            )));
        }
        Err(err) => return Err(LievoError::InvalidInput(err.to_string())),
    };

    let mut paths = HashSet::new();
    let mut visited_dirs = HashSet::new();

    let root_candidates = [
        "README.md",
        "CONTRIBUTING.md",
        "ARCHITECTURE.md",
        "SECURITY.md",
    ];
    for candidate in &root_candidates {
        let candidate_path = project_path.join(candidate);
        if candidate_path.is_file() {
            paths.insert(candidate_path);
        }
    }

    for entry in fs::read_dir(project_path)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!(
                        "Warning: skipping unreadable entry in project root '{}': {err}",
                        project_path.display()
                    );
                }
                continue;
            }
        };
        let path = entry.path();
        if is_markdown_file(&path) {
            paths.insert(path);
        }
    }

    for dir in ["docs", "doc", "design", "adr"] {
        collect_markdown_recursive(
            project_path.join(dir),
            &canonical_root,
            &mut paths,
            &mut visited_dirs,
        )?;
    }

    let mut docs = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            continue;
        }
        let size_bytes = metadata.len();
        let canonical_path = path.canonicalize().unwrap_or(path);
        let mut doc = DiscoveredDoc {
            path: canonical_path,
            size_bytes,
            included_bytes: None,
            relevance_score: 0,
            selected: false,
            skip_reason: None,
        };
        doc.relevance_score = calculate_relevance_score(&doc.path);
        docs.push(doc);
    }

    docs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(docs)
}

/// Score a documentation file by filename relevance.
pub fn calculate_relevance_score(path: &Path) -> i32 {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let score = if file_name == "readme.md" || file_name == "readme" {
        100
    } else if file_name.contains("architecture") {
        90
    } else if file_name == "contributing.md" || file_name == "contributing" {
        80
    } else {
        10
    };

    let size_penalty =
        if fs::metadata(path).map(|m| m.len()).unwrap_or(0) > LARGE_DOC_THRESHOLD_BYTES {
            LARGE_DOC_PENALTY
        } else {
            0
        };

    (score - size_penalty).max(0)
}

pub fn select_within_budget(docs: &mut [DiscoveredDoc], max_total_chars: usize) {
    let mut remaining_chars = max_total_chars;

    for doc in docs.iter_mut() {
        doc.selected = false;
        doc.skip_reason = None;
        doc.included_bytes = None;
    }

    docs.sort_by(|a, b| {
        b.relevance_score
            .cmp(&a.relevance_score)
            .then_with(|| file_modified_unix(&b.path).cmp(&file_modified_unix(&a.path)))
            .then_with(|| a.path.cmp(&b.path))
    });

    for doc in docs.iter_mut() {
        if remaining_chars == 0 {
            doc.selected = false;
            doc.skip_reason = Some("Skipped due to budget limit".to_string());
            continue;
        }
        let doc_size = usize::try_from(doc.size_bytes).unwrap_or(usize::MAX);
        if doc_size <= remaining_chars {
            doc.selected = true;
            doc.included_bytes = Some(doc.size_bytes);
            remaining_chars -= doc_size;
        } else {
            doc.selected = true;
            doc.skip_reason = Some("Truncated to fit budget".to_string());
            doc.included_bytes = Some(u64::try_from(remaining_chars).unwrap_or(0));
            remaining_chars = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_readme_scores_100() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("README.md");
        fs::write(&readme_path, "# README").unwrap();
        let score = calculate_relevance_score(&readme_path);
        assert_eq!(score, 100);
    }

    #[test]
    fn test_architecture_scores_90() {
        let temp = TempDir::new().unwrap();
        let arch_path = temp.path().join("ARCHITECTURE.md");
        fs::write(&arch_path, "# Architecture").unwrap();
        let score = calculate_relevance_score(&arch_path);
        assert_eq!(score, 90);
    }
}
