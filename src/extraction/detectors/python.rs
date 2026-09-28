// Python packages detection

use std::collections::HashMap;
use std::fs;
use std::path::Path;

const PYTHON_SKIP_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "node_modules",
    ".git",
    "__pycache__",
    ".tox",
    "migrations",
];

pub fn detect_python_packages(repo_path: &Path) -> Option<(HashMap<String, String>, usize)> {
    let mut packages = Vec::new();

    for entry in fs::read_dir(repo_path).ok()? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();

        // Look for directories at depth 1 with __init__.py,
        // skipping virtual environments and other non-package dirs.
        if path.is_dir()
            && path.join("__init__.py").exists()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            if name.starts_with('.') || PYTHON_SKIP_DIRS.contains(&name) {
                continue;
            }
            let rel_path = path
                .strip_prefix(repo_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            packages.push((rel_path, name.to_string()));
        }
    }

    // Also scan src/*/ for src-layout packages
    let src_dir = repo_path.join("src");
    if src_dir.is_dir()
        && let Ok(read_dir) = fs::read_dir(&src_dir)
    {
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();

            // Skip the src/ directory itself from being treated as a package
            if path == src_dir {
                continue;
            }

            // Look for src/*/directories with __init__.py
            if path.is_dir()
                && path.join("__init__.py").exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                if name.starts_with('.') || PYTHON_SKIP_DIRS.contains(&name) {
                    continue;
                }
                let rel_path = path
                    .strip_prefix(repo_path)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                packages.push((rel_path, name.to_string()));
            }
        }
    }
    // If src_dir exists but read fails, we skip src-layout scan entirely
    // Root packages are preserved and fall through to packages.is_empty() check

    if packages.is_empty() {
        return None;
    }

    // Infer module_depth: use 2 if any package has sub-packages (nested __init__.py).
    let has_sub_packages = packages.iter().any(|(rel_path, _)| {
        let pkg_dir = repo_path.join(rel_path);
        has_python_sub_packages(&pkg_dir)
    });
    let module_depth = if has_sub_packages { 2 } else { 1 };

    let mut subsystems = HashMap::new();

    // Don't add "." to subsystem map when we have named packages.
    // The detected packages ARE the subsystems — we don't need a catch-all "." subsystem.
    // "." would match all files and with module_depth=2 create thousands of modules for large repos.
    for (path, name) in packages {
        subsystems.insert(path, name);
    }

    // Detect test directories and map them to "testing".
    for test_dir in &["tests", "test"] {
        if repo_path.join(test_dir).is_dir() {
            subsystems.insert(test_dir.to_string(), "testing".to_string());
            break;
        }
    }

    Some((subsystems, module_depth))
}

/// Returns true if `pkg_dir` contains at least one subdirectory with `__init__.py`,
/// skipping known non-package directories.
fn has_python_sub_packages(pkg_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(pkg_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || PYTHON_SKIP_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        if path.join("__init__.py").exists() {
            return true;
        }
    }
    false
}
