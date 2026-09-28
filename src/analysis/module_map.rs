// `#[path]` module map for `lievo admin selfcheck` (issue #732).
//
// `#[path = "X.rs"] mod Y;` declares module `Y` at the physical location
// `X.rs` RELATIVE TO THE DECLARING FILE'S DIRECTORY, so the logical module
// path `a::b::Y` does not map to `src/a/b/Y.rs` on disk. The selfcheck's
// independent verifier (`independent_resolve`) only tests literal
// `src/{path}.rs` / `src/{path}/mod.rs` candidates, so `crate::`/`{repo}::`
// specifiers naming a `#[path]`-diverged module had no evidence at all —
// which both dropped healthy import edges from the evidence set
// (false-zero-callers) and mis-flagged edges the production resolver
// recorded against the physical file (wrong-edge rate).
//
// The walk below mirrors `scripts/check_dead_files.rs` (the orphan gate's
// resolver, issue #672): from the crate roots, follow every semicolon
// `mod` declaration — `#[path]` targets resolve relative to the declaring
// file (a directory target means `<dir>/mod.rs`), plain `mod name;` to
// `name.rs` or `name/mod.rs` — tracking each file's full logical path.
// Every file reached via a `#[path]` attribute whose physical path differs
// from the literal logical mapping is emitted as an alias.
//
// Deliberate divergence from the orphan gate (documented, not accidental):
// - The gate derives its crate roots from Cargo.toml `[lib]`/`[[bin]]` path
//   keys; this walk assumes the literal roots `src/lib.rs` / `src/bin/lievo.rs`
//   (a Cargo.toml-renamed root is missed here, under-reporting aliases).
// - The gate hard-fails on a missing `#[path]` target; this walk skips it
//   (a selfcheck degrades to less evidence, it never blocks).
//
// `#[cfg(test)]`-gated alias exclusion (module-wide invariant): a declaration
// preceded by `#[cfg(test)]` (whether before or after its `#[path]` line) is
// test-gated and NOT recorded — the physical file is absent from the
// non-test profile — and the walk does not descend through a cfg-gated
// intermediate. See `parse_mod_decls` for the attribute-ordering rules.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The `#[path]` module map for one repo: divergent logical→physical pairs
/// plus the same pairs reversed (physical→logical) for the false-zero
/// section, plus `skipped`, the count of unreadable files / missing or
/// unresolvable `mod` targets skipped during the walk — see module doc for
/// the cfg-gating and best-effort rules.
pub struct PathModuleMap {
    pub logical_to_physical: HashMap<String, String>,
    pub physical_to_logical: HashMap<String, String>,
    pub skipped: usize,
}

impl PathModuleMap {
    fn new() -> Self {
        Self {
            logical_to_physical: HashMap::new(),
            physical_to_logical: HashMap::new(),
            skipped: 0,
        }
    }
}

/// Build the `#[path]` module map for the repo at `repo_root`.
///
/// Best-effort by construction: unreadable files and missing/unresolvable
/// `mod` targets are skipped, but every skip is counted in `skipped` so a
/// degraded map (e.g. `src/lib.rs` transiently unreadable) is distinguishable
/// from a repo that genuinely has no aliases.
///
/// Alias rule (issue #732 round-6): a module is recorded as an alias when
/// its physical file sits anywhere but the literal module mapping — for a
/// lib-root (or nested lib) module the literals are `src/{logical}.rs` /
/// `src/{logical}/mod.rs`, for a bin-root module the literal is
/// `src/bin/{name}.rs`. This covers both `#[path]`-diverged files and
/// plain `mod name;` declarations whose target lands outside the literal
/// mapping (e.g. a bin-root module in a directory-style root).
pub fn scan_rust_module_map(repo_root: &Path) -> PathModuleMap {
    let mut visited: HashSet<std::path::PathBuf> = HashSet::new();
    let src = repo_root.join("src");
    // Crate roots, mirroring the Cargo.toml defaults: the lib root is
    // `src/lib.rs`; the binary root is `src/bin/<name>.rs` for each `[[bin]]`
    // (lievo's own binary is `src/bin/lievo/main.rs`, a directory-style root).
    let lib_root = src.join("lib.rs");
    let lib_roots: Vec<PathBuf> = if lib_root.is_file() {
        vec![lib_root]
    } else {
        Vec::new()
    };
    let bin_roots = bin_roots(&src).collect::<Vec<_>>();
    // (file, parent_logical, bin_prefix) — the bin prefix is the bin crate's
    // root directory (`src/bin/<name>/`) carried through the walk so
    // bin-crate modules are alias-checked against the bin-root literal
    // `src/bin/<name>/{logical}.rs` (derived from the bin root that seeded
    // the walk — flat roots and directory-style roots both yield the same
    // prefix, which is exactly Cargo's module layout for a binary), never
    // the crate-root literal. `None` marks a lib-crate file.
    let mut queue: Vec<(PathBuf, String, Option<String>)> = Vec::new();
    for f in lib_roots {
        queue.push((f, String::new(), None));
    }
    for (f, prefix) in bin_roots {
        queue.push((f, String::new(), Some(prefix)));
    }

    let mut map = PathModuleMap::new();
    while let Some((file, parent_logical, bin_prefix)) = queue.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&file) else {
            map.skipped += 1;
            continue;
        };
        let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
        for (name, path_override) in parse_mod_decls(&content) {
            let child = match &path_override {
                Some(target) => {
                    let t = dir.join(target);
                    // A `#[path]` target may point at a directory module —
                    // the module file is then `dir/mod.rs` (mirrors
                    // check_dead_files.rs semantics).
                    if t.is_dir() { t.join("mod.rs") } else { t }
                }
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
                map.skipped += 1;
                continue;
            }
            let Some(rel) = rel_posix(repo_root, &child) else {
                map.skipped += 1;
                continue;
            };
            let logical = if parent_logical.is_empty() {
                name.clone()
            } else {
                format!("{parent_logical}::{name}")
            };
            // Bin-crate modules resolve through the bin-root literal
            // (`{bin_prefix}{logical}.rs`), never the crate-root literal —
            // see `record_module_file`. The prefix is inherited from the
            // crate root the walk was seeded from.
            queue.push((child, logical.clone(), bin_prefix.clone()));
            record_module_file(&mut map, &logical, &rel, bin_prefix.as_deref());
        }
    }

    map
}

/// Record `(logical, rel)` as an alias when the physical file sits anywhere
/// but the literal module mapping: for a lib-root (or nested lib) module,
/// the literals are `src/{logical}.rs` / `src/{logical}/mod.rs`; for any
/// module under a bin crate, the literal is `{bin_prefix}{logical}.rs`
/// (a bin crate is walked as a tree rooted at `src/bin/<name>` — flat root
/// `src/bin/<name>.rs` or directory-style root `src/bin/<name>/main.rs` —
/// so a nested bin module's natural location is `src/bin/<name>/…`, never
/// the crate-root literal; recording that as an alias would poison the map
/// with keys that no lib specifier can match). A `#[path]`-diverged bin
/// module at any other location is still an alias — the bin-crate
/// counterpart of the lib case.
fn record_module_file(map: &mut PathModuleMap, logical: &str, rel: &str, bin_prefix: Option<&str>) {
    let literal_rs = format!("src/{logical}.rs");
    let literal_mod = format!("src/{logical}/mod.rs");
    let divergent = match bin_prefix {
        Some(prefix) => {
            // Bin crates have no crate-root literal (`src/{name}.rs` does
            // not exist), so the bin-root literals are
            // `{prefix}{logical_path}.rs` and `{prefix}{logical_path}/mod.rs`
            // where `logical_path` is the logical path with `::` replaced
            // by `/`. For a top-level bin module `commands` this gives
            // `src/bin/<name>/commands.rs` or `src/bin/<name>/commands/mod.rs`;
            // for a nested bin module `commands::project::query_ops` this
            // gives `src/bin/<name>/commands/project/query_ops.rs`. A module
            // at either natural location is not a divergence, at any nesting
            // level. A `#[path]`-declared bin module at any other location
            // is still an alias.
            let logical_path = logical.replace("::", "/");
            let bin_rs = format!("{prefix}{logical_path}.rs");
            let bin_mod = format!("{prefix}{logical_path}/mod.rs");
            rel != bin_rs && rel != bin_mod
        }
        None => rel != literal_rs && rel != literal_mod,
    };
    if divergent {
        map.logical_to_physical
            .insert(logical.to_string(), rel.to_string());
        map.physical_to_logical
            .insert(rel.to_string(), logical.to_string());
    }
}

/// The `[[bin]]` root files: one per `src/bin/<name>` entry that is either
/// `<name>.rs` (flat root) or `<name>/main.rs` (directory-style root),
/// paired with the repo-relative `src/bin/<name>/` prefix its modules' natural
/// location is rooted under (flat root `src/bin/<name>.rs` and directory
/// style `src/bin/<name>/main.rs` both yield `src/bin/<name>/` — Cargo's
/// module layout for a binary is identical in both forms).
fn bin_roots(src: &Path) -> impl Iterator<Item = (PathBuf, String)> {
    let bin_dir = src.join("bin");
    std::fs::read_dir(bin_dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let dir_root = src.join("bin").join(&file_name);
            // Both root forms share one module prefix: a flat root
            // `src/bin/<name>.rs` and a directory-style root
            // `src/bin/<name>/main.rs` both have their modules under
            // `src/bin/<name>/…` (Cargo's module layout for a binary).
            let prefix = format!("src/bin/{file_name}/");
            if entry.path().is_file() {
                // `src/bin/<name>.rs` — a flat binary root file.
                let name = file_name.strip_suffix(".rs")?;
                if name.is_empty() || name.contains('/') {
                    return None;
                }
                Some((dir_root, prefix))
            } else {
                // `src/bin/<name>/` — a directory-style binary root.
                let main = dir_root.join("main.rs");
                main.is_file().then_some((main, prefix))
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
}

/// `repo_root`-relative POSIX path, or `None` if `p` is not under it.
/// Best-effort on non-UTF-8 path bytes (mirrors the existing
/// `to_string_lossy` pattern in `is_test_like_file`): such paths yield a
/// U+FFFD-substituted key that simply never matches a real on-disk path.
fn rel_posix(root: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(root).ok()?;
    let s = rel.to_string_lossy();
    Some(if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    })
}

/// (mod name, optional `#[path]` target string) pairs for every semicolon
/// `mod` declaration in `source`. Thin delegation to
/// `rust_mod_parse::parse_mod_decls` (issue #744: single shared copy — the
/// selfcheck's structural super:: probe needs the identical parser and must
/// not import this module's walk, so the parser lives in its own
/// dependency-free module both sides call).
pub fn parse_mod_decls(source: &str) -> Vec<(String, Option<String>)> {
    crate::analysis::rust_mod_parse::parse_mod_decls(source)
}

#[cfg(test)]
#[path = "module_map_tests.rs"]
mod module_map_tests;
