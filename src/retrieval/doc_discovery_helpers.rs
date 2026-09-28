use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::Result;

const MAX_DOC_DISCOVERY_DEPTH: usize = 10;

pub(super) fn collect_markdown_recursive(
    dir: PathBuf,
    project_root: &Path,
    discovered: &mut HashSet<PathBuf>,
    visited_dirs: &mut HashSet<PathBuf>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let mut queue = VecDeque::from([(dir, 0usize)]);

    while let Some((current, depth)) = queue.pop_front() {
        if depth > MAX_DOC_DISCOVERY_DEPTH {
            continue;
        }

        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!(
                        "Warning: cannot access directory '{}': {err}",
                        current.display()
                    );
                }
                continue;
            }
        };

        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }

        let canonical_current = match current.canonicalize() {
            Ok(canonical_current) => canonical_current,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!(
                        "Warning: cannot canonicalize directory '{}': {err}",
                        current.display()
                    );
                }
                continue;
            }
        };

        if !canonical_current.starts_with(project_root) {
            continue;
        }

        if !visited_dirs.insert(canonical_current) {
            continue;
        }

        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!(
                        "Warning: cannot read directory '{}': {err}",
                        current.display()
                    );
                }
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    if err.kind() == ErrorKind::PermissionDenied {
                        eprintln!(
                            "Warning: skipping unreadable entry in directory '{}': {err}",
                            current.display()
                        );
                    }
                    continue;
                }
            };
            let path = entry.path();

            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(err) => {
                    if err.kind() == ErrorKind::PermissionDenied {
                        eprintln!(
                            "Warning: skipping unreadable entry '{}': {err}",
                            path.display()
                        );
                    }
                    continue;
                }
            };

            if metadata.is_symlink() {
                continue;
            }

            if metadata.is_dir() {
                if depth < MAX_DOC_DISCOVERY_DEPTH {
                    queue.push_back((path, depth + 1));
                }
            } else if is_markdown_file(&path) {
                discovered.insert(path);
            }
        }
    }

    Ok(())
}

pub(super) fn is_markdown_file(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            !metadata.is_symlink()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        }
        Err(_) => false,
    }
}

pub(super) fn file_modified_unix(path: &Path) -> std::time::Duration {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default()
}
