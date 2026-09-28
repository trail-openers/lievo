// Integration tests for the CLI query subcommands.
//
// Each test gets its own isolated database file (a unique path in
// `std::env::temp_dir()`, overridden via the `LIEVO_DB` env var), so tests
// are hermetic and order-independent regardless of how many run in parallel.
//
// Since issue #631 the query command handlers are binary-local
// (`src/bin/lievo/commands/`), so these tests drive the `lievo` binary
// end-to-end rather than importing handler functions from the library.
//
// Assertions:
// - exit code != 0 for inputs that must fail (unknown project, unknown entity)
// - error envelope (JSON) is present on stdout for the known-failure paths
// - success paths that are DB-content-independent (e.g. `modules <id>`
//   with no matching data → empty list) exit 0

use std::path::PathBuf;
use std::process::Command;

/// Locate the pre-built lievo binary (built by `cargo test` before the
/// integration tests execute) and run it directly. `cargo run` would
/// rebuild and swallow the binary's own stdout/stderr.
fn lievo_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    // <workspace>/target/debug/deps/<test>-<hash>
    //   -> parent: deps, parent: debug
    let debug = exe.parent().and_then(|p| p.parent()).expect("debug dir");
    debug.join("lievo")
}

/// Point the lievo binary at a fresh, isolated SQLite database for this
/// invocation.
///
/// Each call gets a unique path (counter + process id) under
/// `std::env::temp_dir()`, so no two test invocations ever share a file.
/// Any pre-existing file at that path is removed first so a stale
/// left-over from a crashed prior run can't be reused.
fn run(args: &[&str]) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("lievo-test-{pid}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let output = Command::new(lievo_bin())
        .args(args)
        .env("LIEVO_DB", &path)
        .output()
        .expect("failed to run lievo");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let status = output.status.code().unwrap_or(1);
    let _ = std::fs::remove_file(&path);
    (stdout, stderr, status)
}

// ---------------------------------------------------------------------------
// subsystems
// ---------------------------------------------------------------------------

#[test]
fn test_subsystems_command_unknown_project_fails() {
    let (stdout, stderr, status) = run(&[
        "query",
        "subsystems",
        "--project",
        "ghost-9999",
        "--format",
        "json",
    ]);
    assert_ne!(status, 0, "unknown project must exit non-zero");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("ProjectNotFound") || combined.contains("not found"),
        "error envelope must be present, got: {combined:?}"
    );
}

// ---------------------------------------------------------------------------
// modules
// ---------------------------------------------------------------------------

#[test]
fn test_modules_command_with_valid_subsystem_id() {
    let (_stdout, _stderr, status) = run(&["query", "modules", "subsys-2", "--format", "json"]);
    // A valid entity ID with no matching data must return an empty list
    // (exit code 0), not an error.
    assert_eq!(status, 0, "modules for a valid ID with no data must exit 0");
}

#[test]
fn test_modules_command_succeeds_with_valid_subsystem() {
    let (_stdout, _stderr, status) = run(&["query", "modules", "subsys-1", "--format", "json"]);
    assert_eq!(status, 0, "modules for a valid ID must exit 0");
}

// ---------------------------------------------------------------------------
// files
// ---------------------------------------------------------------------------

#[test]
fn test_files_command_succeeds() {
    let (_stdout, _stderr, status) = run(&["query", "files", "module-1", "--format", "json"]);
    assert_eq!(status, 0, "files for a valid module ID must exit 0");
}

#[test]
fn test_files_command_empty_module() {
    let (_stdout, _stderr, status) = run(&["query", "files", "no-such-module", "--format", "json"]);
    assert_eq!(
        status, 0,
        "files for a module with no children must exit 0 (empty list)"
    );
}

// ---------------------------------------------------------------------------
// deps
// ---------------------------------------------------------------------------

#[test]
fn test_deps_command_entity_not_found_fails() {
    let (stdout, stderr, status) = run(&["query", "deps", "ghost-9999", "--format", "json"]);
    assert_ne!(status, 0, "deps for an unknown entity must exit non-zero");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("EntityNotFound") || combined.contains("not found"),
        "error envelope must be present, got: {combined:?}"
    );
}

// ---------------------------------------------------------------------------
// dependents
// ---------------------------------------------------------------------------

#[test]
fn test_dependents_command_entity_not_found_fails() {
    let (stdout, stderr, status) = run(&["query", "dependents", "ghost-9999", "--format", "json"]);
    assert_ne!(
        status, 0,
        "dependents for an unknown entity must exit non-zero"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("EntityNotFound") || combined.contains("not found"),
        "error envelope must be present, got: {combined:?}"
    );
}

// ---------------------------------------------------------------------------
// impact
// ---------------------------------------------------------------------------

#[test]
fn test_impact_command_unknown_project_fails() {
    let (stdout, stderr, status) = run(&[
        "query",
        "impact",
        "--project",
        "ghost-9999",
        "nonexistent-file-9999.rs",
        "--format",
        "json",
    ]);
    assert_ne!(
        status, 0,
        "impact for an unknown project must exit non-zero"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("ProjectNotFound") || combined.contains("not found"),
        "error envelope must be present, got: {combined:?}"
    );
}

// ---------------------------------------------------------------------------
// hotspots
// ---------------------------------------------------------------------------

#[test]
fn test_hotspots_command_unknown_project_fails() {
    let (stdout, stderr, status) = run(&[
        "query",
        "hotspots",
        "--project",
        "ghost-9999",
        "--format",
        "json",
    ]);
    assert_ne!(
        status, 0,
        "hotspots for an unknown project must exit non-zero"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("ProjectNotFound") || combined.contains("not found"),
        "error envelope must be present, got: {combined:?}"
    );
}
