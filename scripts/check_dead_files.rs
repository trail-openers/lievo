//! Dead-file resolver + gate for the `src/` module tree (issue #672).
//!
//! The three existing CI gates (fmt, clippy, test) compile only files
//! reachable from the crate roots via `mod` / `#[path]` declarations, so a
//! `.rs` file that is never declared is invisible to them. This checker
//! enumerates every `.rs` file under `src/` and fails (exit 1) if any of
//! them is unreachable from the crate roots declared in Cargo.toml
//! (`src/lib.rs` and `src/bin/lievo.rs`), printing the offending files
//! and the fix.
//!
//! Resolver semantics (verified against this repo's 54 `#[path]` usages):
//! - Only semicolon-terminated `mod name;` creates a file edge; inline
//!   `mod tests { ... }` blocks are not file references and are ignored.
//! - A `#[path = "..."]` attribute on the line(s) preceding a `mod name;`
//!   overrides default resolution and resolves relative to the DECLARING
//!   file's directory. The target may point at `dir/mod.rs` (a directory
//!   module); a plain target keeps its own extension.
//! - An `include!("file.rs")` statement inside a `mod name { ... }` block
//!   (e.g. `mod x { include!("x.rs"); }`) creates a file edge to that include
//!   target, resolved relative to the DECLARING file's directory.
//! - Without `#[path]`, `mod name;` resolves to `<dir>/name.rs` or
//!   `<dir>/name/mod.rs` (both are accepted).
//! - `#[cfg(test)]`-gated mods count as reachable: `cargo test` compiles
//!   them. `#[cfg]` evaluation is deliberately not attempted — the test
//!   profile is treated as always-on.
//! - Both crate roots seed the walk. A root with zero `mod` edges
//!   (`src/bin/lievo.rs`) is fine; it never re-declares lib modules.
//! - The traversal is recursive: `#[path]` targets themselves may declare
//!   further mods (`sqlite.rs -> sqlite_ops.rs -> sqlite_rel_ops.rs`).
//!
//! Standard library only. Run from the crate root:
//!   rustc --edition 2021 scripts/check_dead_files.rs -o target/check_dead_files && \
//!     ./target/check_dead_files
//!
//! The same binary self-tests with `--test`:
//!   rustc --edition 2021 --test scripts/check_dead_files.rs && ./scripts/check_dead_files

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::ExitCode;

#[cfg(not(test))]
fn main() -> ExitCode {
    let root = crate_root();
    match check(&root) {
        Ok(report) => {
            let _ = report.reachable.len();
            println!("{}", format_report(&report));
            if report.orphans.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("check_dead_files: {err}");
            ExitCode::from(2)
        }
    }
}

/// Crate root: walk up from the current directory to the nearest Cargo.toml
/// so the gate works from the repo root or any subdirectory.
fn crate_root() -> PathBuf {
    let mut dir = env::current_dir().expect("current dir");
    loop {
        if dir.join("Cargo.toml").is_file() {
            return dir;
        }
        if !dir.pop() {
            panic!("check_dead_files: run from within the lievo crate (no Cargo.toml found above)");
        }
    }
}

struct Report {
    all: Vec<String>,
    reachable: HashSet<String>,
    orphans: Vec<String>,
}

fn check(root: &Path) -> Result<Report, String> {
    let roots = crate_roots(root)?;
    let all_set = all_src_files(root)?;

    let mut reachable: HashSet<String> = HashSet::new();
    let mut queue: Vec<PathBuf> = Vec::new();
    for r in &roots {
        let rel = rel_src(r, root)?;
        reachable.insert(rel);
        queue.push(r.clone());
    }
    while let Some(file) = queue.pop() {
        let parent_rel = rel_src(&file, root)?;
        let src = read(&file)?;
        let decls = parse_mod_decls_source(&src);
        for (name, path_override) in decls {
            let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let child = match &path_override {
                Some(target) => dir.join(target),
                None => {
                    let rs = dir.join(format!("{name}.rs"));
                    if rs.is_file() {
                        rs
                    } else {
                        dir.join(format!("{name}/mod.rs"))
                    }
                }
            };
            if !child.is_file() {
                return Err(format!(
                    "{parent_rel}: declared module `{name}` resolves to missing file {}",
                    child.display()
                ));
            }
            let rel = rel_src(&child, root)?;
            if reachable.insert(rel) {
                queue.push(child);
            }
        }
        // `include!("file.rs")` inside a `mod` block: each named include target
        // is a file edge, resolved relative to the declaring file's directory.
        for target in parse_include_targets(&src) {
            let child = file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
                .join(&target);
            if !child.is_file() {
                return Err(format!(
                    "{parent_rel}: include!(\"{target}\") resolves to missing file {}",
                    child.display()
                ));
            }
            let rel = rel_src(&child, root)?;
            if reachable.insert(rel) {
                queue.push(child);
            }
        }
    }

    let orphans: Vec<String> = all_set
        .iter()
        .filter(|f| !reachable.contains(f.as_str()))
        .cloned()
        .collect();
    let mut all: Vec<String> = all_set.iter().cloned().collect();
    all.sort();
    let mut orphans = orphans;
    orphans.sort();
    Ok(Report {
        all,
        reachable,
        orphans,
    })
}

/// Crate root paths from Cargo.toml: the `[lib]` and `[[bin]]` `path` keys.
fn crate_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    // V3
    let manifest = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("reading Cargo.toml: {e}"))?;
    let mut roots = Vec::new();
    for header in ["[lib]", "[[bin]]"] {
        for section in manifest_lines_until_next_section(&manifest, header) {
            if let Some(path) = path_key(&section) {
                roots.push(root.join(path));
            }
        }
    }
    if roots.is_empty() {
        return Err("no [lib] / [[bin]] path entries found in Cargo.toml".into());
    }
    Ok(roots)
}

/// Trimmed lines of the section that starts with `header`, up to (excluding)
/// the next line beginning with `[`.
fn manifest_lines_until_next_section(manifest: &str, header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            break;
        }
        if in_section {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// `path = "src/lib.rs"` → `Some("src/lib.rs")`.
fn path_key(line: &str) -> Option<String> {
    let rest = line.strip_prefix("path")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let s = rest.trim();
    let inner = s.strip_prefix('"').or_else(|| s.strip_prefix('\''))?;
    let inner = inner
        .strip_suffix('"')
        .or_else(|| inner.strip_suffix('\''))?;
    Some(inner.to_string())
}

/// Every `.rs` file under `src/`, as `src/...`-relative POSIX paths.
fn all_src_files(root: &Path) -> Result<HashSet<String>, String> {
    let mut out = HashSet::new();
    walk(root.join("src"), root, &mut out)?;
    Ok(out)
}

fn walk(dir: PathBuf, root: &Path, out: &mut HashSet<String>) -> Result<(), String> {
    let entries = fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            walk(path, root, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| format!("{} is outside the crate root", path.display()))?
                .strip_prefix("src")
                .map_err(|_| format!("{} is not under src/", path.display()))?;
            let rel = rel
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()))?;
            let rel = rel.replace(std::path::MAIN_SEPARATOR, "/");
            out.insert(rel);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

/// (mod name, optional #[path] target string) pairs for every semicolon
/// `mod` declaration in `source`. The optional `#[path]` value is the last
/// one seen on the line(s) immediately before the declaration (attribute
/// lines may sit between the `#[cfg]` and the `mod`).
fn parse_mod_decls_source(source: &str) -> Vec<(String, Option<String>)> {
    let mut pending: Option<String> = None;
    let mut decls = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            // Attribute line: `#[path = "..."]` carries the target; any other
            // attribute (e.g. `#[cfg(test)]`) is an attribute, not a mod decl.
            if let Some(path) = parse_path_attr(trimmed) {
                pending = Some(path);
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if let Some(name) = parse_mod_name(trimmed) {
            decls.push((name, pending.take()));
        }
    }
    decls
}

/// `#[path = "..."]` on its own line → the quoted target.
fn parse_path_attr(line: &str) -> Option<String> {
    if !line.starts_with("#[") || !line.ends_with(']') {
        return None;
    }
    let inner = line[2..line.len() - 1].trim();
    let eq = inner.strip_prefix("path"); // require the `path` identifier
    let inner = eq?;
    let inner = inner.trim_start();
    let inner = inner.strip_prefix('=')?;
    let inner = inner.trim();
    let inner = inner
        .strip_prefix('"')
        .or_else(|| inner.strip_prefix('\''))?;
    let inner = inner
        .strip_suffix('"')
        .or_else(|| inner.strip_suffix('\''))?;
    Some(inner.to_string())
}

/// `mod name;`, `pub mod name;`, `pub(crate) mod name;` — semicolon
/// terminated only. Inline `mod name {` is not a file reference.
fn parse_mod_name(line: &str) -> Option<String> {
    // The declaration is: [pub][(crate)] `mod` <space> <name> `;`.
    // Take the text after the last space before `;` — that is the module
    // name — and validate it is an identifier. This handles `mod n;`,
    // `pub mod n;`, and `pub(crate) mod n;` in one pattern and cannot be
    // fooled by a leading `pub` prefix (the name sits after the space).
    let body = line.strip_suffix(';').unwrap_or(line);
    let name = body.rsplit(' ').next().unwrap_or("");
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    // The line must actually contain `mod <space> <name>` as its declaration
    // part. Accept `mod n`, `pub mod n`, `pub(crate) mod n`.
    let expect = format!("mod {name}");
    let contains = body.ends_with(&expect)
        || body.ends_with(&format!("pub mod {name}"))
        || body.ends_with(&format!("pub(crate) mod {name}"));
    if !contains {
        return None;
    }
    Some(name.to_string())
}

/// `include!("path.rs")` targets in `source` (string-literal form only).
/// Parses the macro argument list by finding the first `(` and last `)` on
/// the line and extracting the quoted string between them.
fn parse_include_targets(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(body) = line.strip_prefix("include!") else {
            continue;
        };
        let open = match body.find('(') {
            Some(i) => i,
            None => continue,
        };
        let close = match body.rfind(')') {
            Some(i) => i,
            None => continue,
        };
        if close <= open {
            continue;
        }
        let inner = &body[open + 1..close];
        let inner = inner.trim();
        let Some(stripped) = inner.strip_prefix('"') else {
            continue;
        };
        let Some(target) = stripped.strip_suffix('"') else {
            continue;
        };
        if !target.is_empty() {
            out.push(target.to_string());
        }
    }
    out
}

/// Path relative to `src/`, POSIX-separated.
fn rel_src(path: &Path, root: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| format!("{} is outside the crate root", path.display()))?;
    let rel = rel
        .strip_prefix("src")
        .map_err(|_| format!("{} is not under src/", path.display()))?;
    Ok(rel
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

fn format_report(report: &Report) -> String {
    if report.orphans.is_empty() {
        return format!(
            "check_dead_files: OK — all {} .rs files under src/ are reachable from the crate roots (src/lib.rs, src/bin/lievo.rs).",
            report.all.len()
        );
    }
    let mut out = format!(
        "check_dead_files: FAILED — {} orphaned .rs file(s) under src/ are unreachable from \
         the crate roots following `mod` / `#[path]` declarations:\n",
        report.orphans.len()
    );
    for orphan in &report.orphans {
        let base = orphan.rsplit('/').next().unwrap_or(orphan);
        let name = base.trim_end_matches(".rs");
        out.push_str(&format!("  src/{orphan}\n"));
        out.push_str(&format!(
            "    fix: declare it (add `mod {name};` or `#[path = \"{base}\"]` in a live file), \
             or delete it if it is dead.\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn decls(text: &str) -> Vec<(String, Option<String>)> {
        parse_mod_decls_source(text)
    }

    fn names(decls: &[(String, Option<String>)]) -> Vec<String> {
        decls.iter().map(|(n, _)| n.clone()).collect()
    }

    fn write(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
    }

    // ── parser: mod declarations ────────────────────────────────────────

    #[test]
    fn plain_mod_decl() {
        let d = decls("mod project;");
        assert_eq!(names(&d), vec!["project".to_string()]);
    }

    #[test]
    fn pub_mod_and_pub_crate_mod() {
        // v2
        let d = decls("pub mod analysis;\n\npub(crate) mod test_support;");
        assert_eq!(
            names(&d),
            vec!["analysis".to_string(), "test_support".to_string()]
        );
    }

    #[test]
    fn inline_mod_block_is_not_a_file_reference() {
        let d = decls("mod tests {\n    fn x() {}\n}");
        assert!(
            d.is_empty(),
            "inline `mod tests` blocks must not create a file edge"
        );
    }

    #[test]
    fn mod_name_must_be_identifier() {
        assert!(decls("mod 1bad;").is_empty());
        assert!(decls("mod;").is_empty());
    }

    // ── parser: #[path] pairing ─────────────────────────────────────────

    #[test]
    fn path_attribute_pairs_with_following_mod() {
        let d = decls("#[cfg(test)]\n#[path = \"api_tests.rs\"]\nmod tests;");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, "tests");
        assert_eq!(d[0].1.as_deref(), Some("api_tests.rs"));
    }

    #[test]
    fn path_attribute_does_not_leak_into_next_mod() {
        let d = decls(
            "#[path = \"relationships_cross_repo.rs\"]\nmod relationships_cross_repo;\n\
             #[path = \"relationships_tests.rs\"]\n#[cfg(test)]\nmod relationships_tests;",
        );
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].1.as_deref(), Some("relationships_cross_repo.rs"));
        assert_eq!(d[1].1.as_deref(), Some("relationships_tests.rs"));
    }

    #[test]
    fn cfg_between_path_and_mod_is_ignored() {
        let d = decls("#[path = \"sqlite_tests.rs\"]\n#[cfg(test)]\nmod sqlite_tests;");
        assert_eq!(d[0].1.as_deref(), Some("sqlite_tests.rs"));
    }

    #[test]
    fn parse_path_attr_requires_quotes_and_shape() {
        assert_eq!(
            parse_path_attr("#[path = \"x/mod.rs\"]"),
            Some("x/mod.rs".to_string())
        );
        assert_eq!(parse_path_attr("#[cfg(test)]"), None);
        assert_eq!(parse_path_attr("#[path = unquoted]"), None);
        assert_eq!(parse_path_attr("not an attribute"), None);
    }

    // ── parser: include! targets ────────────────────────────────────────

    #[test]
    fn include_target_parsed() {
        let d = parse_include_targets("mod x {\n    include!(\"x.rs\");\n}");
        assert_eq!(d, vec!["x.rs".to_string()]);
    }

    #[test]
    fn include_target_with_semicolon() {
        let d = parse_include_targets("include!(\"foo.rs\");");
        assert_eq!(d, vec!["foo.rs".to_string()]);
    }

    // ── resolver: default name → file mapping ───────────────────────────

    /// Scratch crate root; caller removes it. Files live directly under it.
    fn temp_root(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = env::temp_dir().join(format!("check_dead_files_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, content) in files {
            let p = dir.join(path);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, content).unwrap();
        }
        dir
    }

    #[test]
    fn resolver_plain_mod_name_resolves_to_file_in_temp_tree() {
        let tmp = temp_root(
            "resolver",
            &[
                ("src/lib.rs", "mod a;\nmod b;\n"),
                ("src/a.rs", ""),
                ("src/b/mod.rs", "mod c;\n"),
                ("src/b/c.rs", ""),
                ("src/stray.rs", ""),
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        assert!(reachable.contains("a.rs"));
        assert!(reachable.contains("b/mod.rs"));
        assert!(reachable.contains("b/c.rs"));
        assert_eq!(report.orphans, vec!["stray.rs"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolver_path_attribute_resolves_relative_to_declarer() {
        let tmp = temp_root(
            "pathattr",
            &[
                ("src/lib.rs", "#[path = \"real/thing.rs\"]\nmod thing;\n"),
                ("src/real/thing.rs", ""),
                ("src/thing.rs", ""), // must NOT be the target
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        assert!(reachable.contains("real/thing.rs"));
        assert!(
            !reachable.contains("thing.rs"),
            "crate-root-relative resolution is wrong"
        );
        assert_eq!(report.orphans, vec!["thing.rs"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolver_include_target_resolves_relative_to_declarer() {
        let tmp = temp_root(
            "includefile",
            &[
                ("src/lib.rs", "mod a {\n    include!(\"a.rs\");\n}\n"),
                ("src/a.rs", ""),
                ("src/b.rs", ""),
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        assert!(
            reachable.contains("a.rs"),
            "include! target must be reachable"
        );
        assert_eq!(report.orphans, vec!["b.rs"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolver_directory_target_pointing_at_mod_rs() {
        let tmp = temp_root(
            "dirtarget",
            &[
                ("src/lib.rs", "#[path = \"sub/mod.rs\"]\nmod sub;\n"),
                ("src/sub/mod.rs", "mod inner;\n"),
                ("src/sub/inner.rs", ""),
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        assert!(reachable.contains("sub/mod.rs"));
        assert!(reachable.contains("sub/inner.rs"));
        assert!(report.orphans.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolver_multi_hop_chain_three_deep() {
        let tmp = temp_root(
            "multihop",
            &[
                ("src/lib.rs", "mod one;\n"),
                ("src/one.rs", "#[path = \"two.rs\"]\nmod two;\n"),
                ("src/two.rs", "#[path = \"three.rs\"]\nmod three;\n"),
                ("src/three.rs", ""),
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        for f in ["one.rs", "two.rs", "three.rs"] {
            assert!(reachable.contains(f), "{f} should be reachable");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolver_two_roots_bin_with_zero_mods_is_fine() {
        let tmp = temp_root(
            "two_roots",
            &[
                ("src/lib.rs", "mod lib_only;\n"),
                ("src/lib_only.rs", ""),
                ("src/bin/lievo.rs", "fn main_() {}\n"),
            ],
        );
        let root = tmp.clone();
        write(
            &root.join("Cargo.toml"),
            "[lib]\npath = \"src/lib.rs\"\n\n[[bin]]\npath = \"src/bin/lievo.rs\"\n",
        );
        let report = check(&root).expect("resolver should succeed");
        let reachable: HashSet<String> = report.reachable.iter().cloned().collect();
        assert!(reachable.contains("lib_only.rs"));
        assert!(reachable.contains("bin/lievo.rs"));
        assert!(
            report.orphans.is_empty(),
            "bin root with zero mods must not flag lib files"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn report_format_names_each_orphan_and_gives_fix() {
        let tmp = temp_root(
            "report",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", ""),
                ("src/orphan.rs", ""),
            ],
        );
        let root = tmp.clone();
        write(&root.join("Cargo.toml"), "[lib]\npath = \"src/lib.rs\"\n");
        let report = check(&root).expect("resolver should succeed");
        let text = format_report(&report);
        assert!(text.contains("src/orphan.rs"), "orphan path must be listed");
        assert!(
            text.contains("mod orphan;"),
            "fix hint must name the declaration: {text}"
        );
        assert!(
            text.contains("or delete it"),
            "fix hint must offer deletion"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Run the checker on the real repo (the test runs from the crate root,
    /// or a parent of it). Every file named in issue #672's acceptance
    /// criteria must classify reachable — this is the false-negative guard
    /// for the 54 `#[path]`-wired files and the cfg(test) mods.
    /// Run the checker on the real repo (the test runs from the crate root,
    /// or a parent of it). Every `.rs` file under `src/` must be reachable
    /// from the crate roots — the gate must pass clean on the current tree
    /// (exit 0). This is the false-negative guard for the 54 `#[path]`-wired
    /// files and the `#[cfg(test)]` mods: if the resolver misses a live file
    /// it is flagged as an orphan and this test fails.
    #[test]
    fn real_tree_is_clean_of_orphans_and_golden_files_are_reachable() {
        let root = crate_root();
        let report = check(&root).expect("resolver should succeed on the real tree");
        assert!(
            report.orphans.is_empty(),
            "unexpected orphans on the tree: {:?}\n    fix: declare each file (add a `mod` / \
             `#[path]` declaration in a live file) or delete it if it is dead.",
            report.orphans
        );
        let golden = [
            "api_tests.rs", // #[cfg(test)] #[path] mod tests (src/api.rs:287)
            "storage/sqlite.rs",
            "storage/sqlite_ops.rs",     // #[path] from sqlite.rs:12
            "storage/sqlite_rel_ops.rs", // 3-hop: sqlite.rs → sqlite_ops.rs → …
            "storage/sqlite_delete.rs",
            "storage/sqlite_delete_tests.rs",
            "storage/sqlite_ref_impl.rs",
            "retrieval/tools.rs",
            "retrieval/tools_impl.rs", // #[path] from tools.rs:6
            "retrieval/tools_search.rs",
            "retrieval/tools_search_validation_tests/mod.rs", // directory #[path] target
            "retrieval/tools_query_tests.rs",
            "bin/lievo/commands/query_entity.rs",
            "bin/lievo/commands/query_entity_tests.rs", // plain mod in bin/lievo/commands/mod.rs
            "analysis/relationships.rs",
            "analysis/relationships_cross_repo.rs", // non-test #[path] target
            "analysis/relationships_tests.rs",
            "analysis/relationships_regression_tests.rs",
            "analysis/convention_detector.rs",
            "analysis/convention_detector_tests/mod.rs",
            "analysis/flow_tracer/mod.rs",
            "analysis/flow_tracer/tests.rs", // #[path = "tests.rs"]
            "analysis/pipeline_steps/mod.rs",
            "analysis/pipeline_steps/pipeline_cleanup.rs",
            "analysis/insights_tests.rs", // #[cfg(test)] mod in analysis/mod.rs
            "analysis/test_helpers.rs",
            "extraction/function_preservation.rs",
            "extraction/function_preservation_tests_core.rs",
            "extraction/grouping/mod.rs",
            "extraction/grouping/tests.rs",
            "retrieval/hybrid.rs",
            "retrieval/hybrid_tests.rs",
            "retrieval/test_helpers.rs",
            "summarization/pipeline.rs",
            "summarization/pipeline_tests.rs",
            "summarization/pipeline_tests_fixtures.rs",
            "summarization/batch_ops.rs", // include! from pipeline.rs
            "mcp/tools.rs",
            "mcp/tools_tests.rs",
            "mcp/tools_error_tests_entity.rs",
            "mcp/tools_error_tests_insights.rs",
            "mcp/tools_error_tests_io.rs",
            "bin/lievo/main.rs",
        ];
        for f in golden {
            assert!(
                report.reachable.contains(f),
                "golden file {f} must be reachable; missing from {:?}-sized reachable set",
                report.reachable.len()
            );
        }
        // No false negatives: every file the walker reached must exist in src/.
        for f in &report.reachable {
            let p = root.join("src").join(f);
            assert!(p.is_file(), "reachable set contains missing file {f}");
        }
    }
}
