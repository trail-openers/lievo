// JS/TS top-level subsystem detection

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// File extensions that count as JavaScript/TypeScript source.
const JS_TS_EXTENSIONS: &[&str] = &["js", "jsx", "ts", "tsx", "mjs", "cjs"];

/// Directories (by name) that are generated, vendored, or hidden and must not
/// be counted as JS/TS source or emitted as subsystems.
const GENERATED_DIRS: &[&str] = &["node_modules", "dist", "build", "coverage"];

/// Cap on total entries (dirs + files) visited by `contains_js_ts_source`.
/// Bounds worst-case cost on large vendor trees and keeps the scan from
/// scanning indefinitely. Real source files appear within the first few
/// levels in practice, so the cap does not affect normal repos.
const MAX_SCAN_ENTRIES: usize = 50_000;

/// Cap on directory depth visited by `contains_js_ts_source`.
const MAX_SCAN_DEPTH: usize = 32;

/// Detect top-level directories that contain at least one JS/TS source file.
///
/// Skips hidden directories and generated directories (node_modules, dist,
/// build, .next, .turbo, .nuxt, .output, coverage, .git). Entries are sorted
/// for deterministic ordering. Returns `None` when no top-level JS/TS
/// source directory exists (never `Some` with an empty set).
pub fn detect_js_ts_subsystems(repo_path: &Path) -> Option<Vec<(String, String)>> {
    let canon_repo = repo_path.canonicalize().ok()?;

    let mut results = Vec::new();
    for entry in fs::read_dir(repo_path).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || GENERATED_DIRS.contains(&name) {
            continue;
        }

        // Security: verify the directory stays within the repo root
        // (rejects symlink escapes).
        let canon_dir = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !canon_dir.starts_with(&canon_repo) {
            continue;
        }

        if contains_js_ts_source(&path, &canon_repo) {
            let owned = name.to_string();
            results.push((owned.clone(), owned));
        }
    }

    // Sort for deterministic order (read_dir iteration order is OS-dependent)
    results.sort_by(|a, b| a.0.cmp(&b.0));
    if results.is_empty() {
        return None;
    }
    Some(results)
}

/// Returns true if `dir` contains at least one JS/TS source file.
///
/// Recursive DFS, bounded by `MAX_SCAN_ENTRIES` total entries and
/// `MAX_SCAN_DEPTH` directory depth. Skips hidden/generated subdirectories
/// and files (vendor code under node_modules/.git etc. never counts).
///
/// `root` is the repo's canonical path; entries whose canonical form does not
/// stay under `root` are skipped, and already-canonicalized directories are
/// not re-visited (defeats symlink cycles such as `a/lnk -> a`).
///
/// `read_dir` errors (e.g. EACCES) are intentionally swallowed: a denied
/// directory is treated as containing no JS/TS source. The output is
/// heuristic grouping metadata only, and the pattern matches the other
/// detectors in this module.
fn contains_js_ts_source(dir: &Path, root: &Path) -> bool {
    let mut stack: VecDeque<(PathBuf, usize)> = VecDeque::new();
    stack.push_back((dir.to_path_buf(), 0));
    let mut visited: Vec<PathBuf> = Vec::new();
    let mut visited_entries: usize = 0;

    while let Some((current, depth)) = stack.pop_back() {
        if let Some(name) = current.file_name().and_then(|n| n.to_str())
            && (name.starts_with('.') || GENERATED_DIRS.contains(&name))
        {
            continue;
        }

        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            visited_entries += 1;
            if visited_entries > MAX_SCAN_ENTRIES {
                return false;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if name.starts_with('.') || GENERATED_DIRS.contains(&name) {
                    continue;
                }
                if depth + 1 > MAX_SCAN_DEPTH {
                    continue;
                }
                // Symlink escape / cycle guard: resolve and dedupe by canonical path.
                let canon = match path.canonicalize() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if !canon.starts_with(root) {
                    continue;
                }
                if visited.iter().any(|p| p == &canon) {
                    continue;
                }
                visited.push(canon.clone());
                stack.push_back((path, depth + 1));
            } else if path.is_file() && is_js_ts_extension(name) {
                return true;
            }
        }
    }

    false
}

fn is_js_ts_extension(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        // Case-insensitive without a per-file allocation (extensions are
        // matched case-insensitively, e.g. `Button.TS`).
        Some((_, ext)) => JS_TS_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_js_ts(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), "// stub\n").unwrap();
    }

    #[test]
    fn test_detects_top_level_dirs_with_js_ts_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_js_ts(&tmp.path().join("app/javascript"), "main.js");
        write_js_ts(&tmp.path().join("cypress"), "spec.ts");
        write_js_ts(&tmp.path().join("lib"), "util.tsx");

        let result = detect_js_ts_subsystems(tmp.path());
        assert!(result.is_some());
        let dirs = result.unwrap();
        assert_eq!(
            dirs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["app", "cypress", "lib"]
        );
    }

    #[test]
    fn test_returns_none_when_no_js_ts_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_js_ts(&tmp.path().join("docs"), "README.md");
        fs::create_dir_all(tmp.path().join("empty")).unwrap();

        let result = detect_js_ts_subsystems(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_skips_generated_and_hidden_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_js_ts(&tmp.path().join("node_modules"), "vendor.js");
        write_js_ts(&tmp.path().join(".next"), "chunk.js");
        write_js_ts(&tmp.path().join("dist"), "bundle.js");
        write_js_ts(&tmp.path().join("coverage"), "lcov.js");
        write_js_ts(&tmp.path().join("build"), "out.js");

        let result = detect_js_ts_subsystems(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_nested_file_counts() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_js_ts(&tmp.path().join("src").join("components"), "Button.tsx");

        let result = detect_js_ts_subsystems(tmp.path());
        let dirs = result.unwrap();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, "src");
    }

    #[test]
    fn test_symlink_cycle_terminates() {
        // A repo with a symlink loop (inner/lnk -> inner) used to loop the
        // recursive scan forever: the JS/TS file sits past the cycle, so the
        // early-out on first file never fires. With the canonicalized
        // visited-set guard, the scan must terminate.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let tmp = tempfile::TempDir::new().unwrap();
            let inner = tmp.path().join("inner");
            write_js_ts(&inner, "main.js");
            symlink(inner.as_os_str(), inner.join("lnk").as_os_str()).unwrap();

            let result = detect_js_ts_subsystems(tmp.path());
            let dirs = result.unwrap();
            assert_eq!(dirs.len(), 1);
            assert_eq!(dirs[0].0, "inner");
        }
    }

    #[test]
    fn test_node_modules_nested_inside_source_dir_does_not_count_alone() {
        // A source dir that ONLY has JS/TS under a nested node_modules/ must
        // not be reported.
        let tmp = tempfile::TempDir::new().unwrap();
        write_js_ts(&tmp.path().join("app").join("node_modules"), "dep.js");

        let result = detect_js_ts_subsystems(tmp.path());
        assert!(result.is_none());
    }
}
