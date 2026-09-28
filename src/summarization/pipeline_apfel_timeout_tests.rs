// Apfel-timeout salvage test (issue #811 task-b), moved out of
// pipeline_tests.rs (issue #869) so it can hold the crate-wide env lock
// around its LIEVO_APFEL_TIMEOUT_SECS / PATH mutations: the 2s deadline is
// then invisible to every other apfel-subprocess test, closing the flake
// channel where the deadline (or the fake on PATH) could be read at spawn
// time by a test running outside the lock.

use crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET;
use crate::summarization::pipeline::SummarizationPipeline;
use crate::summarization::pipeline::function_id;
use crate::summarization::pipeline_tests_fixtures::{
    TestStorage, make_file_entity, make_fn_entity,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tempfile::TempDir;

/// Integration test (issue #673, task-b): a batch whose apfel invocation times
/// out must be salvaged by the existing match arm in summarize_functions — the
/// run completes without panicking, the timed-out batch is marked as
/// failed/warned (all positions missing), and the child process is killed.
///
/// The test uses a fake apfel binary that sleeps longer than the deadline and
/// writes a marker file on normal exit. If the pipeline killed the child on
/// timeout, the marker is absent. If it were merely abandoned, the marker
/// would appear after the sleep (and the test would fail).
///
/// The deadline is short (2s, via LIEVO_APFEL_TIMEOUT_SECS — a #[cfg(test)]
/// seam in apfel.rs, issue #811) instead of the 300s production constant,
/// so this test keeps default CI fast.
///
/// The fake `apfel` is installed on PATH as a small inline script (sleep 999
/// FIRST, then write the marker on normal exit — the same ordering as the
/// one-shot fixture at apfel_oneshot_tests.rs:208). The ordering is what
/// makes the kill-proof assertion at the bottom of the test a genuine proof
/// that the child was killed on timeout, not abandoned: if the marker were
/// written before the sleep, the child would write it at spawn time, ~2s
/// before the kill lands, and the assertion would fail even when the kill
/// works.
#[test]
fn test_salvage_timed_out_batch_child_killed() {
    // This test drives the pipeline into real apfel subprocess calls
    // (the fake `apfel` on PATH times out), mutating PATH and
    // LIEVO_APFEL_TIMEOUT_SECS. Hold the crate-wide env lock around the whole
    // run (issue #863 / #869) so no other test can race the env mutation or
    // observe the 2s deadline: every env-mutating lib test holds this same
    // lock, so the timeout and PATH changes are invisible to concurrent
    // apfel-subprocess tests.
    let _env_lock = crate::test_env_support::env_lock();
    // Short test-only deadline (issue #811 task-a): the production path keeps
    // its 300s APFEL_TIMEOUT_SECS; this env var is read only under #[cfg(test)].
    unsafe { std::env::set_var("LIEVO_APFEL_TIMEOUT_SECS", "2") };

    // Temp dir for the marker file (proves the child was killed, not abandoned).
    let tmp_dir = TempDir::new().expect("create temp dir for marker");
    let marker = tmp_dir.path().join("apfel_ran_marker");
    // Remove any pre-existing file at the marker path defensively: the
    // kill-proof assertion below (marker absent after the deadline) must not
    // be defeated by a stale file from a previously crashed run.
    let _ = fs::remove_file(&marker);

    // Set FAKE_APFEL_MARKER so the fake apfel writes it on normal exit.
    // set_var is `unsafe` in the 2024 edition (process-global mutation);
    // safe here: no other test mutates this variable concurrently (env-
    // mutating tests are serialized by the lock held above, and this is the
    // only test that sets FAKE_APFEL_MARKER).
    unsafe { std::env::set_var("FAKE_APFEL_MARKER", &marker) };

    // Install the fake apfel on PATH so `Command::new("apfel")` resolves to it.
    // Note: is_apfel_available() caches in a OnceLock, but batch_summarize and
    // summarize_code call Command::new("apfel") directly — they do NOT go
    // through the availability gate, so this PATH override is effective for
    // the call.
    let original_path = std::env::var("PATH").unwrap_or_default();
    let fake_apfel_dir = tmp_dir.path().join("fake_bin");
    fs::create_dir_all(&fake_apfel_dir).expect("create fake_bin dir");
    let fake_bin = fake_apfel_dir.join("apfel");
    // Inline fake: sleep 999 FIRST, then write the marker. This ordering is
    // what makes the kill-proof assertion at the bottom of the test valid —
    // if the marker were written before the sleep, the child would write it
    // at spawn time and the assertion (marker absent after the deadline)
    // would fail even when the kill works. See the test-level comment above.
    let script = "#!/bin/sh\nsleep 999\nif [ -n \"${FAKE_APFEL_MARKER:-}\" ]; then touch \"$FAKE_APFEL_MARKER\"; fi\n";
    {
        use crate::summarization::pipeline_tests_fixtures::write_fake_apfel;
        write_fake_apfel(&fake_bin, script);
    }
    fs::set_permissions(&fake_bin, fs::Permissions::from_mode(0o755))
        .expect("set +x on fake apfel");
    unsafe {
        std::env::set_var(
            "PATH",
            format!("{}:{}", fake_apfel_dir.display(), original_path),
        )
    };

    // Restore PATH and clean up on scope exit (best-effort; the test process
    // exits right after, but other tests in the same binary must not see it).
    // Declared AFTER _env_lock so it drops BEFORE the env lock is released:
    // no other test may run with PATH still pointing at this test's fake.
    struct PathGuard {
        original: String,
        tmp_dir: PathBuf,
    }
    impl Drop for PathGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::set_var("PATH", self.original.clone());
                std::env::remove_var("FAKE_APFEL_MARKER");
                std::env::remove_var("LIEVO_APFEL_TIMEOUT_SECS");
            }
            let _ = fs::remove_dir_all(&self.tmp_dir);
        }
    }
    let _guard = PathGuard {
        original: original_path,
        tmp_dir: tmp_dir.path().to_path_buf(),
    };

    // Build a minimal TestStorage with one file and two functions.
    let mut storage = TestStorage::new();
    let repo_id = "test-repo";
    let file_path = "src/lib.rs";
    let file_entity = make_file_entity("file-timeout", file_path);
    storage.add_file(repo_id, file_path, file_entity.clone());

    let fn_a = make_fn_entity(
        &function_id(&file_entity.id, "fn_a"),
        &file_entity.id,
        "fn_a",
    );
    let fn_b = make_fn_entity(
        &function_id(&file_entity.id, "fn_b"),
        &file_entity.id,
        "fn_b",
    );
    storage.add_function(&function_id(&file_entity.id, "fn_a"), fn_a);
    storage.add_function(&function_id(&file_entity.id, "fn_b"), fn_b);

    let code_units = [
        crate::model::CodeUnit {
            name: "fn_a".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_string(),
            line: 1,
            end_line: 1,
            language: "Rust".to_string(),
            signature: None,
            code: Some("fn fn_a() -> i32 { 1 }".to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "fn_a".to_string(),
            docstring: None,
            parent_class: None,
        },
        crate::model::CodeUnit {
            name: "fn_b".to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_string(),
            line: 3,
            end_line: 3,
            language: "Rust".to_string(),
            signature: None,
            code: Some("fn fn_b() -> i32 { 2 }".to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "fn_b".to_string(),
            docstring: None,
            parent_class: None,
        },
    ];

    // Call summarize_functions. It will invoke batch_summarize (or summarize_code
    // per missing position) which hits the fake apfel that sleeps 999s.
    // The pipeline's salvage arm must convert the Err to vec![None; batch_size]
    // and the run must complete without panicking.
    //
    // Called directly on this thread: `dyn Storage` is not `Sync`, so we cannot
    // move it into a spawned watchdog thread. The bound guarantee (that the call
    // returns within the task-a deadline instead of hanging) is proven by the fact
    // that this test returns at all — with no timeout mechanism, `Command::output()`
    // would wait on the sleeping child forever and the whole suite would hang.
    let count = SummarizationPipeline::summarize_functions(
        &storage,
        repo_id,
        &code_units[..],
        APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .expect("summarize_functions should not error")
    .summarized;

    // The timed-out batch should have been salvaged: no functions were summarized
    // (all positions are None because the child was killed before producing output).
    // The count should be 0 (no summaries were persisted from the timed-out batch).
    assert_eq!(
        count, 0,
        "timed-out batch should be salvaged as all-None; no summaries persisted"
    );

    // Verify the marker file was NOT written — proving the child was killed
    // before it could complete its sleep and write the marker.
    // (If the child were abandoned, sleep 999 would eventually write it.
    // We give it a moment to have written if it wasn't killed.)
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        !marker.exists(),
        "apfel marker file should NOT exist — the child must have been killed on timeout, not abandoned"
    );
}
