// Top-level src/ subdirectory detection

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Detect top-level src/ subdirectories
pub fn detect_top_level_src(repo_path: &Path) -> Option<HashMap<String, String>> {
    let src_dir = repo_path.join("src");
    if !src_dir.exists() || !src_dir.is_dir() {
        return None;
    }

    let mut subdirs = Vec::new();

    for entry in fs::read_dir(&src_dir).ok()? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();

        if path.is_dir()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            let rel_path = path
                .strip_prefix(repo_path)
                .ok()?
                .to_string_lossy()
                .to_string();
            subdirs.push((rel_path, name.to_string()));
        }
    }

    if subdirs.is_empty() {
        return None;
    }

    let mut subsystems = HashMap::new();
    subsystems.insert(".".to_string(), "root".to_string());
    for (path, name) in subdirs {
        subsystems.insert(path, name);
    }

    Some(subsystems)
}
