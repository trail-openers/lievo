// Framework-specific subsystem detectors (Django apps, Go layout)

use std::fs;
use std::path::Path;

/// Scan for Django apps — directories containing models.py + (views.py or apps.py).
///
/// Checks immediate subdirectories and one level deeper (e.g. apps/users/).
/// Skips hidden dirs and common non-app dirs (venv, node_modules, .git, __pycache__).
/// Symlinks that escape the repo boundary are silently skipped.
pub fn detect_django_apps(repo_path: &Path) -> Vec<(String, String)> {
    const SKIP: &[&str] = &[
        "venv",
        ".venv",
        "node_modules",
        ".git",
        "__pycache__",
        ".tox",
        "migrations",
    ];

    let is_django_app = |dir: &Path| -> bool {
        dir.join("models.py").is_file()
            && (dir.join("views.py").is_file() || dir.join("apps.py").is_file())
    };

    let mut apps = Vec::new();

    let Ok(entries) = fs::read_dir(repo_path) else {
        return apps;
    };

    // Canonicalize repo root once for symlink boundary checks.
    let canon_repo = fs::canonicalize(repo_path).ok();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Guard: skip symlinks that escape the repo boundary.
        if let Some(ref cr) = canon_repo
            && let Ok(canon_entry) = fs::canonicalize(&path)
            && !canon_entry.starts_with(cr)
        {
            continue;
        }

        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };
        if name.starts_with('.') || SKIP.contains(&name.as_str()) {
            continue;
        }

        // Depth-1 check
        if is_django_app(&path) {
            apps.push((name.clone(), name.clone()));
            continue;
        }

        // Depth-2 check (e.g. apps/users/)
        let Ok(sub_entries) = fs::read_dir(&path) else {
            continue;
        };
        for sub in sub_entries.flatten() {
            let sub_path = sub.path();
            if !sub_path.is_dir() {
                continue;
            }

            // Guard: skip symlinks that escape the repo boundary.
            if let Some(ref cr) = canon_repo
                && let Ok(canon_sub) = fs::canonicalize(&sub_path)
                && !canon_sub.starts_with(cr)
            {
                continue;
            }

            let sub_name = match sub_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            if sub_name.starts_with('.') || SKIP.contains(&sub_name.as_str()) {
                continue;
            }
            if is_django_app(&sub_path) {
                let rel = format!("{}/{}", name, sub_name);
                apps.push((rel, sub_name));
            }
        }
    }

    apps.sort_by(|a, b| a.0.cmp(&b.0));
    apps
}

/// Detect Go standard layout: cmd/, internal/, pkg/ directories.
///
/// Each found directory becomes a subsystem entry. When any are present,
/// a "." infrastructure entry is also added.
pub fn detect_go_layout(repo_path: &Path) -> Vec<(String, String)> {
    const LAYOUT_DIRS: &[(&str, &str)] =
        &[("cmd", "cmd"), ("internal", "internal"), ("pkg", "pkg")];

    let mut subsystems = Vec::new();
    for (dir_name, display_name) in LAYOUT_DIRS {
        let path = repo_path.join(dir_name);
        if path.is_dir() {
            subsystems.push((dir_name.to_string(), display_name.to_string()));
        }
    }

    if !subsystems.is_empty() {
        subsystems.insert(0, (".".to_string(), "infrastructure".to_string()));
    }

    subsystems
}
