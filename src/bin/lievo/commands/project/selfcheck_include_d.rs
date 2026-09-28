// Include!d-file detection for the structural super:: probe (issue #788).
//
// Every file pulled into the corpus via an `include!(…)` call has its
// logical module at the INCLUDING parent (Rust's `include!` semantics),
// not at the directory the probe's walk reads. The walk therefore cannot
// confirm or fail an include!d file's `super::` sites correctly, so they
// degrade to INDETERMINATE (never a failure) — the same safe direction the
// walk already uses for missing targets.
//
// The scan is line-based (mirrors `find_item_names`): `include!(…)` on a
// single line, the standard form; multi-line `include!` calls are missed
// rather than mis-parsed. The corpus file set is matched (a name that names
// no scanned file is not an include!d file of this repo).

use std::collections::HashSet;
use std::path::Path;

/// The include!d-file set (issue #788): every file pulled into the corpus
/// via an `include!(…)` call. Each `include!("X")` in file F names the file
/// `X` relative to F's directory (Rust's `include!` semantics), and the
/// included file's logical module is F's module — NOT the directory the
/// probe's walk reads. The walk therefore cannot confirm or fail an
/// include!d file's `super::` sites correctly, so they degrade to
/// INDETERMINATE (never a failure) — the same safe direction the walk
/// already uses for missing targets. The scan is line-based (mirrors
/// `find_item_names`): `include!(…)` on a single line, the standard form;
/// multi-line `include!` calls are missed rather than mis-parsed. The
/// corpus file set is matched (a name that names no scanned file is not an
/// include!d file of this repo).
pub(crate) fn include_d_files(
    all_files: &[String],
    repo_root: &Path,
    known_paths: &HashSet<&str>,
) -> HashSet<String> {
    // The include! set is keyed by the INCLUDED file's physical path (the
    // form the probe's site list uses — `unit.file` is the physical path).
    // Build it from each including file's `include!("X")` calls: the
    // included path is `X` relative to the including file's directory,
    // with the `.rs` extension appended (Rust's `include!` semantics — the
    // name is the extensionless stem). Only include! names that name a
    // scanned corpus file are recorded.
    let mut include_d = HashSet::new();
    for file in all_files {
        if !file.ends_with(".rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root.join(file)) else {
            continue;
        };
        let dir = file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for line in content.lines() {
            let trimmed = line.trim();
            // Line-anchored scan (mirrors `find_item_names`): the
            // `include!` invocation must be at the start of the trimmed
            // line so a mid-line `include!` in an expression or a macro
            // name containing `include!` is not mis-parsed.
            let Some(after) = trimmed.strip_prefix("include!") else {
                continue;
            };
            let after = after.trim_start();
            let Some(open) = after.strip_prefix('(') else {
                continue;
            };
            let Some(close) = open.find(')') else {
                continue;
            };
            let quoted = open[..close].trim();
            let Some(inner) = quoted.strip_prefix('"').and_then(|q| q.strip_suffix('"')) else {
                continue;
            };
            let name = inner.trim();
            if name.is_empty() {
                continue;
            }
            // The included file's physical path: the including file's
            // directory + the include! name (which carries the `.rs`
            // extension in the corpus form) — use the name as-is if it
            // ends with `.rs`, otherwise append it.
            let included = if name.ends_with(".rs") {
                if dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{dir}/{name}")
                }
            } else {
                if dir.is_empty() {
                    format!("{name}.rs")
                } else {
                    format!("{dir}/{name}.rs")
                }
            };
            if known_paths.contains(included.as_str()) {
                include_d.insert(included);
            }
        }
    }
    include_d
}
