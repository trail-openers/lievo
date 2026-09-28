// Unit tests for the `#[path]` module map scanner (issue #732).

use super::*;

fn write_tree(dir: &std::path::Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }
}

#[test]
fn scan_bin_directory_style_root_module_at_natural_location_is_not_an_alias() {
    // A bin-root module in a directory-style root (`src/bin/lievo/commands.rs`
    // for `mod commands;` declared in `src/bin/lievo/main.rs`) sits at its
    // natural bin-crate location `src/bin/<name>/{logical}.rs` — it must NOT
    // be recorded as an alias (issue #758: the old code recorded every bin
    // file as a spurious alias because the bin literal didn't account for
    // the bin directory segment).
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            ("src/bin/lievo/main.rs", "mod commands;\n"),
            ("src/bin/lievo/commands.rs", "pub fn cmd_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(
        !map.logical_to_physical.contains_key("commands"),
        "a bin-root module at its natural src/bin/lievo/commands.rs location must not be an alias. map: {:?}",
        map.logical_to_physical
    );
}

#[test]
fn scan_bin_directory_style_root_mod_rs_at_natural_location_is_not_an_alias() {
    // A bin-root module in a directory-style root whose file is
    // `src/bin/lievo/commands/mod.rs` (directory module form) also sits at
    // its natural bin-crate location — it must NOT be an alias.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            ("src/bin/lievo/main.rs", "mod commands;\n"),
            ("src/bin/lievo/commands/mod.rs", "pub fn cmd_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(
        !map.logical_to_physical.contains_key("commands"),
        "a bin-root module at its natural src/bin/lievo/commands/mod.rs location must not be an alias. map: {:?}",
        map.logical_to_physical
    );
}

#[test]
fn scan_bin_flat_root_module_at_literal_location_is_not_an_alias() {
    // A bin-root module at its natural flat location (`src/bin/tool/mods.rs`
    // for `mod mods;` declared in `src/bin/tool.rs`) — the bin-root literal
    // check must not record a dead alias for a module at its natural location.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            ("src/bin/tool.rs", "mod mods;\n"),
            ("src/bin/tool/mods.rs", "pub fn mods_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(
        !map.logical_to_physical.contains_key("mods"),
        "a bin-root module at its natural src/bin/tool/mods.rs location must not be an alias. map: {:?}",
        map.logical_to_physical
    );
}

#[test]
fn scan_bin_nested_module_at_natural_location_is_not_an_alias() {
    // A nested bin module (`commands::project::query_ops` at
    // `src/bin/lievo/commands/project/query_ops.rs`) must not be recorded
    // as a spurious alias — the bin prefix accounts for the full nested
    // path (issue #758 root-cause fix).
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            ("src/bin/lievo/main.rs", "mod commands;\n"),
            ("src/bin/lievo/commands/mod.rs", "pub mod project;\n"),
            ("src/bin/lievo/commands/project/mod.rs", "mod query_ops;\n"),
            (
                "src/bin/lievo/commands/project/query_ops.rs",
                "pub fn qop() {}\n",
            ),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(
        !map.logical_to_physical
            .contains_key("commands::project::query_ops"),
        "a nested bin module at its natural location must not be an alias. map: {:?}",
        map.logical_to_physical
    );
}

#[test]
fn scan_bin_path_attribute_divergent_module_is_still_an_alias() {
    // A genuine `#[path]`-diverged module in a bin crate MUST still be
    // recorded as an alias — the fix must not suppress real divergences.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            (
                "src/bin/lievo/main.rs",
                "mod commands;\n#[path = \"impls/cmd.rs\"]\nmod cmd;\n",
            ),
            ("src/bin/lievo/impls/cmd.rs", "pub fn cmd_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("cmd"),
        Some(&"src/bin/lievo/impls/cmd.rs".to_string())
    );
}

#[test]
fn scan_empty_tree_has_no_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_literal_mod_has_no_alias() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub mod a;\n"),
            ("src/a.rs", "pub fn a_fn() {}\n"),
        ],
    );
    // No #[path] → the literal mapping holds, so no alias is needed.
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_bin_directory_style_root_is_walked() {
    // A directory-style binary root (`src/bin/lievo/main.rs`) must be
    // walked: a `#[path]` divergence declared inside the bin tree is
    // recorded as an alias (issue #732 round-5 finding #1 — the walk
    // previously assumed the literal `src/bin/lievo.rs`, missing this
    // repo's actual `src/bin/lievo/main.rs` root).
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            (
                "src/bin/lievo/main.rs",
                "mod commands;\n#[path = \"impls/cmd.rs\"]\nmod cmd;\n",
            ),
            ("src/bin/lievo/commands.rs", "pub fn cmd_fn() {}\n"),
            ("src/bin/lievo/impls/cmd.rs", "pub fn cmd_fn2() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("cmd"),
        Some(&"src/bin/lievo/impls/cmd.rs".to_string())
    );
}

#[test]
fn scan_bin_flat_root_is_walked() {
    // A flat binary root (`src/bin/tool.rs`) must also be walked.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub fn lib_fn() {}\n"),
            ("src/bin/tool.rs", "#[path = \"x/impls/y.rs\"]\nmod y;\n"),
            ("src/bin/x/impls/y.rs", "pub fn y_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("y"),
        Some(&"src/bin/x/impls/y.rs".to_string())
    );
}

#[test]
fn scan_path_attribute_produces_divergent_alias() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "src/lib.rs",
                "#[path = \"steps/cleanup.rs\"]\nmod cleanup;\n",
            ),
            ("src/steps/cleanup.rs", "pub fn cleanup() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("cleanup"),
        Some(&"src/steps/cleanup.rs".to_string())
    );
}

#[test]
fn scan_path_targeting_literal_location_is_not_an_alias() {
    // `#[path = "a.rs"] mod a;` where the target sits at the module's
    // LITERAL location (`src/a.rs`): the independent resolver's literal
    // mapping already lands on the file, so no redirect is recorded.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "#[path = \"a.rs\"]\nmod a;\n"),
            ("src/a.rs", "pub fn a_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_nested_path_chain_is_transitive() {
    // lib → one (plain) → two (#[path], divergent) — the chain must keep
    // tracking logical paths through the plain-mod hop.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "pub mod one;\n"),
            ("src/one.rs", "#[path = \"two_impl.rs\"]\nmod two;\n"),
            ("src/two_impl.rs", "pub fn two_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("one::two"),
        Some(&"src/two_impl.rs".to_string())
    );
}

#[test]
fn scan_directory_path_target_maps_to_mod_rs() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "#[path = \"sub/\"]\nmod sub;\n"),
            ("src/sub/mod.rs", "pub fn sub_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_directory_path_target_divergent_records_alias() {
    // Directory target whose `mod.rs` sits at a DIVERGENT location: module
    // `deep` is physically at `src/impls/deep/mod.rs`, so the logical
    // `crate::deep` needs the redirect.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            ("src/lib.rs", "#[path = \"impls/deep/\"]\nmod deep;\n"),
            ("src/impls/deep/mod.rs", "pub fn deep_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("deep"),
        Some(&"src/impls/deep/mod.rs".to_string())
    );
}

#[test]
fn scan_missing_path_target_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "src/lib.rs",
                "#[path = \"gone.rs\"]\nmod gone;\npub mod b;\n",
            ),
            ("src/b.rs", "pub fn b_fn() {}\n"),
        ],
    );
    // The missing target must not abort the walk — the rest still resolves.
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_cfg_gated_path_decl_is_not_recorded() {
    // `#[cfg(test)]`-gated `#[path]` declarations must NOT register their
    // keys for the non-test profile (issue #732 edge case; round-2
    // finding #2): the physical test file is absent from the non-test
    // extraction, so the alias must not exist at all.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "src/lib.rs",
                "#[cfg(test)]\n#[path = \"t.rs\"]\nmod tests;\npub mod c;\n",
            ),
            ("src/t.rs", "pub fn t_fn() {}\n"),
            ("src/c.rs", "pub fn c_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(
        map.logical_to_physical.is_empty(),
        "cfg(test)-gated #[path] decl must not register an alias"
    );
}

#[test]
fn scan_path_before_cfg_test_still_gates() {
    // Attribute order: `#[path]` before `#[cfg(test)]` — the cfg gate still
    // applies (issue #732 edge case).
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "src/lib.rs",
                "#[path = \"t.rs\"]\n#[cfg(test)]\nmod tests;\npub mod c;\n",
            ),
            ("src/t.rs", "pub fn t_fn() {}\n"),
            ("src/c.rs", "pub fn c_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert!(map.logical_to_physical.is_empty());
}

#[test]
fn scan_non_test_cfg_gated_path_decl_is_recorded() {
    // `#[cfg(unix)]`-gated (non-test) `#[path]` declaration: the file is
    // compiled in the non-test profile on unix, so the alias is recorded.
    // The scan itself is platform-agnostic (it cannot evaluate cfgs), so
    // only `#[cfg(test)]` gates a declaration out.
    let dir = tempfile::tempdir().unwrap();
    write_tree(
        dir.path(),
        &[
            (
                "src/lib.rs",
                "#[cfg(unix)]\n#[path = \"steps/u.rs\"]\nmod u;\n",
            ),
            ("src/steps/u.rs", "pub fn u_fn() {}\n"),
        ],
    );
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("u"),
        Some(&"src/steps/u.rs".to_string())
    );
}

/// Cross-walk parity (issue #732 round-4, finding: duplicate #[path]
/// walkers): both walkers must agree that the SAME physical file hosts the
/// same `#[path]`-diverged module. Runs the orphan gate's `check_dead_files`
/// binary (the same `scripts/check_dead_files.rs` source, compiled by this
/// test) in a fixture repo; a gate failure on the literal twin is the
/// negative proof that the gate's own parser resolved the `#[path]` — i.e.
/// both walkers agree the module lives at the divergent location, while the
/// scanner's alias map records the exact same redirect.
#[test]
fn scan_and_dead_files_resolver_agree_on_physical_locations() {
    // Compile the gate (same source the CI gate compiles) and run it in the
    // fixture: crate_root() walks up to the fixture's Cargo.toml.
    let gate_src = std::env::current_dir()
        .expect("cwd")
        .join("scripts/check_dead_files.rs");
    let gate_bin = std::env::temp_dir().join(format!(
        "check_dead_files_parity_{}.bin",
        std::process::id()
    ));
    std::process::Command::new("rustc")
        .args(["--edition", "2021", "-o"])
        .arg(&gate_bin)
        .arg(&gate_src)
        .output()
        .expect("rustc on scripts/check_dead_files.rs");

    let dir = tempfile::tempdir().unwrap();
    // `#[path]` points at `impls/cleanup.rs`, so the LITERAL file
    // `src/two.rs` is never referenced — the gate MUST call it orphaned.
    // If the gate's parser did not resolve the `#[path]` (or resolved it
    // differently), the gate would exit 0 and this test fails.
    write_tree(
        dir.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"fixture\"\n[lib]\npath = \"src/lib.rs\"\n",
            ),
            (
                "src/lib.rs",
                "pub mod one;\n#[path = \"impls/cleanup.rs\"]\nmod two;\n",
            ),
            ("src/one.rs", "pub fn one_fn() {}\n"),
            ("src/impls/cleanup.rs", "pub fn two_fn() {}\n"),
            (
                "src/two.rs",
                "// literal twin of the #[path] module — orphaned by design\n",
            ),
        ],
    );

    let out = std::process::Command::new(&gate_bin)
        .current_dir(dir.path())
        .output()
        .expect("run check_dead_files binary");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_file(&gate_bin);
    assert!(
        out.status.code() == Some(1),
        "gate must flag the orphaned literal twin; exit {:?}, stdout: {stdout}",
        out.status.code()
    );
    assert!(
        stdout.contains("src/two.rs"),
        "gate must name the orphan; stdout: {stdout}"
    );
    assert!(
        !stdout.contains("src/impls/cleanup.rs"),
        "gate must resolve the #[path] target as reachable; stdout: {stdout}"
    );

    // The selfcheck scanner must record the same divergent physical file
    // for the same logical module.
    let map = scan_rust_module_map(dir.path());
    assert_eq!(
        map.logical_to_physical.get("two"),
        Some(&"src/impls/cleanup.rs".to_string())
    );
}
