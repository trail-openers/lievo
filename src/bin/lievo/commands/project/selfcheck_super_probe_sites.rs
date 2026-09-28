// Site collection and resolution for the structural super:: probe (issue
// #744, #788). Extracted to keep selfcheck_super_probe.rs under the 500-line
// budget.

use std::collections::HashSet;
use std::path::Path;

use lievo::model::CodeUnit;

use super::selfcheck_include_d::include_d_files;

/// True when `file` is the crate root of its crate (the file that has no
/// parent module). `src/lib.rs` for lib crates; `src/bin/<name>.rs` (flat,
/// no sub-path) for bin crates. The crate root is exempt from the
/// `is_test_like_file` check (rule c flags `lib.rs` for the false-zero
/// section's 0-importer treatment, but the root is a valid probe target).
fn is_crate_root(file: &str) -> bool {
    if file == "src/lib.rs" {
        return true;
    }
    if let Some(rest) = file.strip_prefix("src/bin/") {
        // Flat `src/bin/<name>.rs` (no sub-path): the bin crate root.
        rest.ends_with(".rs") && !rest.contains('/')
    } else {
        false
    }
}

/// Collect the super:: sites from the code units, excluding test-like files
/// (defect 2) and include!d files (issue #788 — their sites degrade to
/// indeterminate, never failures). Returns (sites, include_d_site_count).
///
/// Each site is (importing_file, referenced name, hops). A glob member
/// (`*`) is a distinct site (indeterminate by the corrected rule — nothing
/// to confirm, never a failure). For grouped imports each member is its own
/// site and is classified independently.
pub(crate) fn collect_super_sites(
    code_units: &[CodeUnit],
    all_files: &[String],
    repo_root: &Path,
) -> (Vec<(String, String, usize)>, usize) {
    let known_files: HashSet<&str> = all_files.iter().map(String::as_str).collect();
    let include_d = include_d_files(all_files, repo_root, &known_files);

    let mut sites: Vec<(String, String, usize)> = Vec::new();
    let mut include_d_sites = 0usize;
    for unit in code_units {
        if unit.language != "Rust" {
            continue;
        }
        let is_root = is_crate_root(&unit.file);
        if !is_root && lievo::analysis::rust_mod_parse::is_test_like_file(&unit.file) {
            continue;
        }
        // Include!d file (issue #788): count its super:: sites as
        // indeterminate and skip them — the directory walk reads the wrong
        // module for them (the file's directory, not the including
        // parent's module).
        if include_d.contains(unit.file.as_str()) {
            for import in &unit.imports {
                let segments: Vec<&str> = import.split("::").collect();
                if segments.first() != Some(&"super") {
                    continue;
                }
                let leading = segments
                    .iter()
                    .take_while(|s| **s == "super" || **s == "self")
                    .count();
                if let Some(seg) = segments.get(leading) {
                    if seg.is_empty() {
                        include_d_sites += 1;
                    } else if let Some(inner) =
                        seg.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
                    {
                        for member in inner.split(',') {
                            if !member.trim().is_empty() {
                                include_d_sites += 1;
                            }
                        }
                    } else {
                        include_d_sites += 1;
                    }
                }
            }
            continue;
        }
        for import in &unit.imports {
            let segments: Vec<&str> = import.split("::").collect();
            if segments.first() != Some(&"super") {
                continue; // the probe is scoped to the super:: class only
            }
            let leading = segments
                .iter()
                .take_while(|s| **s == "super" || **s == "self")
                .count();
            let hops = segments
                .iter()
                .take(leading)
                .filter(|s| **s == "super")
                .count();
            // Only the FIRST segment after the leading super/self run is
            // the referenced name (for `use super::x::y` that is `x`, not
            // `y` — the issue #744 edge case). Subsequent segments are
            // sub-paths within the referenced module, not separate sites.
            if let Some(seg) = segments.get(leading) {
                if seg.is_empty() {
                    // `use super::` with no trailing module name.
                } else if let Some(inner) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
                {
                    // Grouped import: `use super::{a, b}` — split the
                    // braced members and classify each independently.
                    for member in inner.split(',') {
                        let member = member.trim();
                        if !member.is_empty() {
                            sites.push((unit.file.clone(), member.to_string(), hops));
                        }
                    }
                } else {
                    sites.push((unit.file.clone(), seg.to_string(), hops));
                }
            }
        }
    }
    (sites, include_d_sites)
}
