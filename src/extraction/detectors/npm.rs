// NPM workspace detection

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::js_ts::detect_js_ts_subsystems; // called by this module's supplement below

pub fn detect_npm_workspace(repo_path: &Path) -> Option<HashMap<String, String>> {
    let package_json = repo_path.join("package.json");
    if !package_json.exists() {
        return None;
    }

    let content = fs::read_to_string(&package_json).ok()?;
    let json_value: serde_json::Value = serde_json::from_str(&content).ok()?;

    let workspaces = json_value.get("workspaces")?;

    // Handle both array form and object form
    let mem_array = if let Some(arr) = workspaces.as_array() {
        arr
    } else {
        let obj = workspaces.as_object()?;
        obj.get("packages")?.as_array()?
    };

    if mem_array.is_empty() {
        return None;
    }

    let mut subsystems = HashMap::new();
    subsystems.insert(".".to_string(), "root".to_string());

    for member in mem_array {
        if let Some(member_str) = member.as_str() {
            for (rel_path, name) in expand_workspace_glob(repo_path, member_str) {
                subsystems.insert(rel_path, name);
            }
        }
    }

    // Supplement: workspace members may leave the bulk of the repo's JS/TS
    // source outside the map (www-shaped repos: workspaces: ["ssr"] plus bulk
    // source in top-level dirs like app/, cypress/, lib/). When the number of
    // unmatched top-level JS/TS dirs is >= max(3, 25% of matched members),
    // add those dirs to the map so they don't collapse into ".".
    let matched_members = subsystems.len() - 1;
    // Ceiling division (no float roundtrip): 25% of members, min 3.
    let threshold = 3.max(matched_members.div_ceil(4));
    // Cheap pre-filter: one non-recursive read_dir of the repo root. If
    // fewer than `threshold` top-level dirs are not already covered by the
    // workspace map, the recursive JS/TS scan cannot reach the threshold and
    // is skipped (it is pure waste when the npm map covers the layout).
    let uncovered_top_level = fs::read_dir(repo_path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let path = e.path();
                    path.is_dir()
                        && !path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with('.'))
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    if uncovered_top_level >= threshold
        && let Some(js_ts_dirs) = detect_js_ts_subsystems(repo_path)
    {
        let unmatched: Vec<(String, String)> = js_ts_dirs
            .into_iter()
            .filter(|(rel_path, _)| !subsystems.contains_key(rel_path))
            .collect();
        if unmatched.len() >= threshold {
            for (rel_path, name) in unmatched {
                subsystems.insert(rel_path, name);
            }
        }
    }

    Some(subsystems)
}

pub(crate) fn expand_workspace_glob(repo_path: &Path, pattern: &str) -> Vec<(String, String)> {
    if !pattern.contains('*') {
        let name = pattern.rsplit('/').next().unwrap_or(pattern).to_string();
        return vec![(pattern.to_string(), name)];
    }

    let prefix = pattern.trim_end_matches('*').trim_end_matches('/');

    // Reject bare "*" pattern — would enumerate entire repo root
    if prefix.is_empty() {
        return Vec::new();
    }

    let dir = repo_path.join(prefix);

    // Security: verify expanded path stays within repo root (prevents path traversal)
    let canon_repo = match repo_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let canon_dir = match dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return Vec::new(), // dir doesn't exist
    };
    if !canon_dir.starts_with(&canon_repo) {
        return Vec::new();
    }

    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                let rel_path = format!("{prefix}/{name}");
                results.push((rel_path, name));
            }
        }
    }
    // Sort for deterministic order (read_dir iteration order is OS-dependent)
    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}
