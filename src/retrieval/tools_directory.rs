// Directory/file tool implementations.

use serde_json::{Value, json};
use std::path::Component;

use crate::model::EntityTier;
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

use super::super::{ListDirectoryTool, ReadFileTool};
use super::tools_search::{lock_storage, truncate};

/// Maximum characters returned for raw file content.
const MAX_FILE_CONTENT_CHARS: usize = 32_768;

// ---------------------------------------------------------------------------
// ListDirectoryTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for ListDirectoryTool<S> {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let path_input = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'path'".into()))?;

        // Path traversal check: reject `..` components and absolute paths.
        let path = std::path::Path::new(path_input);
        for component in path.components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Ok(
                        json!({"error": format!("path traversal not allowed: {path_input}")})
                            .to_string(),
                    );
                }
                _ => {}
            }
        }

        if self.ctx.repo_path == std::path::PathBuf::new() {
            return Ok(
                json!({"error": "list_directory not available: repo path not configured"})
                    .to_string(),
            );
        }

        let full_path = self.ctx.repo_path.join(path_input);

        let canonical_repo = match self.ctx.repo_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Ok(
                    json!({"error": "list_directory not available: repo path not configured"})
                        .to_string(),
                );
            }
        };
        let canonical_full = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Ok(json!({"error": format!("path not found: {}", path_input)}).to_string());
            }
        };
        if !canonical_full.starts_with(&canonical_repo) {
            return Ok(json!({"error": "path escapes repo root"}).to_string());
        }

        if full_path.is_file() {
            return Ok(
                json!({"error": format!("path is a file, not a directory: {path_input}")})
                    .to_string(),
            );
        }

        struct DirEntry {
            name: String,
            display_path: String,
            entity_id: Option<String>,
            is_dir: bool,
        }

        // Resolve repo_id once before iterating entries.
        let repo_id = {
            let guard = lock_storage!(self.ctx.storage);
            guard
                .list_repos(&self.ctx.project_id)
                .ok()
                .and_then(|repos| repos.into_iter().next())
                .map(|r| r.id)
        };

        // Phase 1: collect directory entries (no storage access)
        struct RawEntry {
            name: String,
            is_dir: bool,
        }

        let raw_entries: Vec<RawEntry> = std::fs::read_dir(&full_path)
            .map_err(|e| {
                crate::LievoError::InvalidInput(format!(
                    "failed to read directory {path_input}: {e}"
                ))
            })?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.path().is_dir();
                Some(RawEntry { name, is_dir })
            })
            .collect();

        struct EntryInfo {
            entry_path: String,
            display_path: String,
            is_dir: bool,
            name: String,
        }

        // Phase 2: Collect all paths for batch lookup
        let mut paths_to_lookup: Vec<String> = Vec::new();
        let mut entry_data: Vec<EntryInfo> = Vec::new();

        // Map from entry index to dot_path for O(1) lookups
        let mut dot_path_to_entry_index: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();

        let effective_path = if path_input.is_empty() || path_input == "." || path_input == "./" {
            ""
        } else {
            path_input
        };

        for raw in &raw_entries {
            let entry_path = if effective_path.is_empty() {
                raw.name.clone()
            } else {
                format!("{}/{}", effective_path.trim_end_matches('/'), raw.name)
            };

            paths_to_lookup.push(entry_path.clone());
            let display_path = if raw.is_dir {
                format!("{}/", raw.name)
            } else {
                raw.name.clone()
            };
            entry_data.push(EntryInfo {
                entry_path,
                display_path,
                is_dir: raw.is_dir,
                name: raw.name.clone(),
            });
        }

        // Phase 3: Batch lookup entity_ids (max 2 DB round-trips)
        let guard = lock_storage!(self.ctx.storage);
        let mut entries: Vec<DirEntry> = Vec::new();

        if let Some(rid) = repo_id.as_deref() {
            // First batch: direct paths
            let paths_slice: Vec<&str> = paths_to_lookup.iter().map(|s| s.as_str()).collect();
            let path_to_id = guard.entity_ids_for_paths(rid, &paths_slice)?;

            // Second batch: for misses at root level, try with "./" prefix
            let effective_path_is_root = effective_path.is_empty();
            let mut dot_paths_to_lookup: Vec<String> = Vec::new();

            if effective_path_is_root {
                for (i, info) in entry_data.iter().enumerate() {
                    if !path_to_id.contains_key(&info.entry_path) {
                        let dot_path = format!("./{}", info.name);
                        dot_paths_to_lookup.push(dot_path.clone());
                        dot_path_to_entry_index.insert(i, dot_path);
                    }
                }
            }

            let dot_path_to_id = if !dot_paths_to_lookup.is_empty() {
                let dot_slice: Vec<&str> = dot_paths_to_lookup.iter().map(|s| s.as_str()).collect();
                guard.entity_ids_for_paths(rid, &dot_slice)?
            } else {
                std::collections::HashMap::new()
            };

            // Build DirEntry results
            for (i, info) in entry_data.into_iter().enumerate() {
                let entity_id = path_to_id.get(&info.entry_path).cloned().or_else(|| {
                    if effective_path_is_root {
                        // Check if this entry had a dot_path lookup
                        dot_path_to_entry_index
                            .get(&i)
                            .and_then(|dot_path| dot_path_to_id.get(dot_path))
                            .cloned()
                    } else {
                        None
                    }
                });

                entries.push(DirEntry {
                    name: info.name.clone(),
                    display_path: info.display_path,
                    entity_id,
                    is_dir: info.is_dir,
                });
            }
        } else {
            // No repo_id - all entries have None entity_id
            for info in entry_data {
                entries.push(DirEntry {
                    name: info.name,
                    display_path: info.display_path,
                    entity_id: None,
                    is_dir: info.is_dir,
                });
            }
        }
        drop(guard);

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.display_path.cmp(&b.display_path))
        });

        let entries: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "path": e.display_path,
                    "name": e.name,
                    "entity_id": e.entity_id,
                    "indexed": e.entity_id.is_some(),
                    "is_dir": e.is_dir,
                })
            })
            .collect();

        let count = entries.len();

        Ok(json!({
            "path": path_input,
            "entries": entries,
            "count": count
        })
        .to_string())
    }
}

// ---------------------------------------------------------------------------
// ReadFileTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for ReadFileTool<S> {
    fn name(&self) -> &str {
        "read_file"
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let entity_id = input
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'entity_id'".into()))?;

        let guard = lock_storage!(self.ctx.storage);
        let entity = match guard.get_entity(entity_id)? {
            Some(e) => e,
            None => {
                return Ok(json!({"error": format!("entity not found: {entity_id}")}).to_string());
            }
        };

        if entity.tier != EntityTier::File {
            return Ok(
                json!({"error": format!("entity {} is not a file (tier: {})", entity_id, entity.tier)})
                    .to_string(),
            );
        }

        let rel_path = match &entity.path {
            Some(p) => p.clone(),
            None => {
                return Ok(json!({"error": format!("entity {entity_id} has no path")}).to_string());
            }
        };

        drop(guard);

        let full_path = self.ctx.repo_path.join(&rel_path);

        let canonical_repo = match self.ctx.repo_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Ok(
                    json!({"error": "read_file not available: repo path not configured"})
                        .to_string(),
                );
            }
        };
        let canonical_full = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Ok(
                    json!({"error": format!("file not found on disk: {}", rel_path)}).to_string(),
                );
            }
        };
        if !canonical_full.starts_with(&canonical_repo) {
            return Ok(json!({"error": "path escapes repo root"}).to_string());
        }

        let content = match std::fs::read_to_string(&canonical_full) {
            Ok(c) => c,
            Err(_) => {
                return Ok(
                    json!({"error": format!("file not found on disk: {}", rel_path)}).to_string(),
                );
            }
        };

        let truncated = truncate(&content, MAX_FILE_CONTENT_CHARS);

        Ok(json!({
            "entity_id": entity.id,
            "name": entity.name,
            "path": rel_path,
            "content": truncated
        })
        .to_string())
    }
}
