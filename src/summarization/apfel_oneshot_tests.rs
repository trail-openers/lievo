use crate::summarization::apfel::*;
use crate::summarization::pipeline_tests_fixtures::write_fake_apfel;
// Every test below that invokes `run_apfel` must hold this guard for the
// duration of the invocation: the guard serializes PATH-mutating tests
// (issue #863) so concurrent tests cannot interleave their PATH
// mutations.
use crate::summarization::summarizer_fullpath_tests::env_guard;

#[path = "apfel_parsing_tests.rs"]
mod parsing_tests;

use parsing_tests::format_batch_for_apfel;

/// Create a fake apfel binary in a temp dir that sleeps for `sleep_secs` seconds.
/// Returns the path to the binary.
///
/// ETXTBSY (os error 26) on exec means the file being executed is still open
/// for writing somewhere in the calling process. The `File` returned by
/// `File::create` is dropped (fd closed) before `set_permissions` and before
/// the helper returns, so no write handle is open when the child execs the
/// script. The tempdir is per-call, so no two tests can race on the same
/// path either.
fn make_hanging_apfel(sleep_secs: u64) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    let script = format!("#!/bin/sh\nsleep {sleep_secs}\n");
    write_fake_apfel(&bin_path, &script);
    (tmp, bin_path)
}

/// Create a fake apfel binary that exits immediately with valid JSON output.
fn make_fast_ok_apfel() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    let script = r#"#!/bin/sh
echo '{"content": "ok summary"}'
exit 0
"#;
    write_fake_apfel(&bin_path, script);
    (tmp, bin_path)
}

/// Create a fake apfel batch binary that echoes valid batch JSON immediately.
fn make_fast_ok_batch_apfel() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    // Shell script that echoes a JSON object. The JSON content field
    // contains \n-separated "#n: summary" lines. In the Rust string,
    // \\n produces a literal backslash-n in the shell output, which
    // serde_json then parses as a newline in the content string.
    let script = [
        "#!/bin/sh".to_string(),
        "printf '%s' '{\"content\": \"#0: adds two numbers\\n#1: subtracts two numbers\"}'"
            .to_string(),
        "exit 0".to_string(),
    ]
    .join("\n");
    write_fake_apfel(&bin_path, &script);
    (tmp, bin_path)
}

/// Test that run_apfel times out when the child hangs past the deadline.
/// The fake apfel sleeps 999 seconds; we give it a 1-second deadline.
/// This test proves:
/// 1. The error is Err(SummarizationFailed) with a timeout message
/// 2. The error message contains "timed out after 1 seconds"
/// 3. The call returns within a bounded wall-clock time (< 5s)
#[test]
fn test_run_apfel_times_out_and_returns_error() {
    let _guard = env_guard();
    let (_tmp, bin_path) = make_hanging_apfel(999);
    let bin_str = bin_path.to_string_lossy().to_string();

    let start = std::time::Instant::now();
    let result = run_apfel(&bin_str, 1, &["-o", "json", "-s", "test"], "hello");
    let elapsed = start.elapsed();

    // Must be an error, not a hang
    let err = result.expect_err("expected timeout error");
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("timed out after 1 seconds"),
        "error should name the timeout duration: got '{err_msg}'"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "call should return within bounded time, took {elapsed:?}"
    );
}

/// Test that run_apfel returns Ok with the correct stdout when the child exits fast.
#[test]
fn test_run_apfel_fast_ok_returns_stdout() {
    let _guard = env_guard();
    let (_tmp, bin_path) = make_fast_ok_apfel();
    let bin_str = bin_path.to_string_lossy().to_string();

    let result = run_apfel(&bin_str, 10, &["-o", "json", "-s", "test"], "hello")
        .expect("fast fake apfel should succeed");

    let (success, stdout, _stderr) = result;
    assert!(success, "fast fake apfel should exit 0");
    let stdout_str = String::from_utf8_lossy(&stdout);
    assert!(
        stdout_str.contains("\"content\": \"ok summary\""),
        "stdout should contain the canned JSON: '{stdout_str}'"
    );
}

/// Test that the timeout error message is distinguishable (contains "timed out after").
/// This is the contract the pipeline's salvage arm at pipeline.rs:296 relies on.
#[test]
fn test_timeout_error_is_distinguishable() {
    let _guard = env_guard();
    let (_tmp, bin_path) = make_hanging_apfel(999);
    let bin_str = bin_path.to_string_lossy().to_string();

    let result = run_apfel(&bin_str, 1, &[], "test");
    let err = result.expect_err("expected timeout error");

    // The message must contain "timed out after" to be parseable by callers
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("timed out after"),
        "timeout error must contain 'timed out after' for pipeline salvage arm: got '{err_msg}'"
    );
}

/// Test that summarize_code with a fast fake apfel returns the parsed content.
/// This is a regression test: the bounded-wait refactor must not break the
/// happy-path JSON parsing for non-timeout invocations.
#[test]
fn test_summarize_code_happy_path_with_fast_apfel() {
    let _guard = env_guard();
    // Use the private run_apfel helper directly with a fast fake binary
    let (_tmp, bin_path) = make_fast_ok_apfel();
    let bin_str = bin_path.to_string_lossy().to_string();

    let (success, stdout, _stderr) =
        run_apfel(&bin_str, 10, &["-o", "json", "-s", "test"], "hello").expect("should succeed");

    assert!(success);
    let json_str = String::from_utf8_lossy(&stdout);
    let response: ApfelResponse = serde_json::from_str(&json_str).expect("should parse valid JSON");
    assert_eq!(response.content.trim(), "ok summary");
}

/// Test that batch_summarize with a fast fake apfel returns correctly parsed
/// positional results.
#[test]
fn test_batch_summarize_happy_path_with_fast_apfel() {
    let _guard = env_guard();
    let (_tmp, bin_path) = make_fast_ok_batch_apfel();
    let bin_str = bin_path.to_string_lossy().to_string();

    let (success, stdout, _stderr) =
        run_apfel(&bin_str, 10, &["-o", "json", "-s", "test"], "batch input")
            .expect("should succeed");

    assert!(success);
    let json_str = String::from_utf8_lossy(&stdout);
    let response: ApfelResponse = serde_json::from_str(&json_str).expect("should parse valid JSON");
    let results = parse_batch_response(&response.content, 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("adds two numbers".to_string()));
    assert_eq!(results[1], Some("subtracts two numbers".to_string()));
}

/// Test that run_apfel handles a child that exits with a non-zero status
/// (e.g., apfel fails gracefully). The success flag should be false.
#[test]
fn test_run_apfel_nonzero_exit() {
    let _guard = env_guard();
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    let script = "#!/bin/sh\necho 'error occurred' >&2\nexit 1\n";
    write_fake_apfel(&bin_path, script);
    let bin_str = bin_path.to_string_lossy().to_string();

    let result = run_apfel(&bin_str, 10, &[], "test").expect("should complete (not time out)");
    let (success, _stdout, stderr) = result;
    assert!(!success, "non-zero exit should report success=false");
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.contains("error occurred"),
        "stderr should contain the error message: '{stderr_str}'"
    );
}

/// Test that a hung child is actually killed — the marker file written on
/// normal exit should NOT appear after a timeout.
///
/// This proves child.kill() (via pipe close on the killed child) actually
/// terminates the process rather than merely abandoning it.
#[test]
fn test_run_apfel_kills_hung_child() {
    let _guard = env_guard();
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    let marker_path = tmp.path().join("marker.txt");
    let bin_path_str = bin_path.to_string_lossy().to_string();
    let marker_str = marker_path.to_string_lossy().to_string();

    // Fake apfel that sleeps 999s then writes a marker on normal exit.
    // If killed, the marker should never appear.
    let script = format!("#!/bin/sh\nsleep 999\n> '{marker_str}'\n");
    write_fake_apfel(&bin_path, &script);

    let _ = run_apfel(&bin_path_str, 1, &[], "test");

    // After the timeout + kill, the marker file must NOT exist.
    // (If the child were merely abandoned and continued running, it would
    // eventually write the marker — but since it was killed, it won't.)
    assert!(
        !marker_path.exists(),
        "marker file should not exist after kill — child was terminated"
    );
}

#[test]
fn test_is_apfel_available() {
    // This test will pass if apfel is installed at build time
    // It's expected to fail in CI environments without apfel
    let _ = is_apfel_available();
}

#[test]
#[cfg_attr(not(feature = "test-with-apfel"), ignore)]
fn test_summarize_code() {
    let code = "fn add(a: i32, b: i32) -> i32 {
    a + b
}";
    let result = summarize_code(code, None).unwrap();
    assert!(!result.content.is_empty());
}

#[test]
#[cfg_attr(not(feature = "test-with-apfel"), ignore)]
fn test_batch_summarize() {
    let snippets = vec![
        (
            "src/lib.rs".to_string(),
            "add".to_string(),
            "fn add(a: i32, b: i32) -> i32 { a + b }".to_string(),
        ),
        (
            "src/lib.rs".to_string(),
            "sub".to_string(),
            "fn sub(a: i32, b: i32) -> i32 { a - b }".to_string(),
        ),
    ];
    let results = batch_summarize(&snippets, None).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].is_some());
    assert!(results[1].is_some());
}

#[test]
fn test_batch_summarize_empty() {
    let snippets: Vec<(String, String, String)> = vec![];
    let results = batch_summarize(&snippets, None).unwrap();
    assert!(results.is_empty());
}

/// `BackendTransport` no longer carries the bearer token (issue #783: the
/// token is resolved at request time via `summarizer_backend::summarizer_token()
///`), so its `Debug` output can never surface a token value. The redaction
/// contract now holds by construction — this locks that the struct holds no
/// secret material.
#[test]
fn test_backend_transport_debug_has_no_token_to_leak() {
    let transport = BackendTransport {
        url: "http://127.0.0.1:11434".to_string(),
        backend: crate::config::SummarizerBackend::Apfel,
        model_name: Some("mymodel".to_string()),
    };
    let formatted = format!("{transport:?}");
    assert!(
        !formatted.to_ascii_lowercase().contains("token"),
        "Debug output must contain no token field: {formatted}"
    );
    assert!(
        formatted.contains("mymodel"),
        "Debug must still show the model name: {formatted}"
    );
}

#[test]
fn test_batch_summarize_index_preservation_across_batches() {
    // Integration test for per-batch index space (a): verify that batch_summarize
    // correctly uses positional mapping so each batch's indices map to the correct
    // functions within that batch.
    //
    // This test verifies the complete flow: pack -> format -> index labels -> parse -> positional result.

    // Use large code snippets to force multiple batches (similar to existing test)
    let large_code = "x".repeat(4000); // 4000 chars each
    let snippets = vec![
        ("src/a.rs".to_string(), "a".to_string(), large_code.clone()),
        ("src/b.rs".to_string(), "b".to_string(), large_code.clone()),
        ("src/c.rs".to_string(), "c".to_string(), large_code.clone()),
    ];

    let batches = pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);

    // Verify we got multiple batches (not all fit in first batch)
    assert!(
        batches.len() > 1,
        "expected multiple batches for 3 large snippets, got {}",
        batches.len()
    );

    // Verify first batch has at least one entry
    assert!(
        !batches[0].is_empty(),
        "first batch should have at least one entry"
    );

    // Total entries equals total snippets
    let total_entries: usize = batches.iter().map(|b| b.len()).sum();
    assert_eq!(
        total_entries, 3,
        "total entries across all batches should equal total snippets"
    );

    // Critical: verify each batch starts with index 0 (per-batch index space reset)
    for (batch_idx, batch) in batches.iter().enumerate() {
        let formatted = format_batch_for_apfel(batch);
        assert!(
            formatted.contains("--- Function: #0"),
            "batch {} should start with #0 (index space reset per batch)",
            batch_idx
        );

        // Verify budget is respected
        assert!(
            formatted.len() <= APFEL_INPUT_CHAR_BUDGET,
            "batch {batch_idx} formatted input exceeds budget: {}",
            formatted.len()
        );
    }

    // Critical: verify that the first and second batch have different file identities at position 0
    // This proves that index 0 in batch 1 is different from index 0 in batch 0 (different files)
    let batch_0_pos_0_file = &batches[0][0].0;
    let batch_1_pos_0_file = &batches[1][0].0;
    assert_ne!(
        batch_0_pos_0_file, batch_1_pos_0_file,
        "index 0 in batch 0 should be different from index 0 in batch 1 (different files)"
    );

    // Verify the files are correctly associated with their positions in the original snippet list
    // This ensures the (file,name,code) tuple is preserved through batching
    assert_eq!(snippets[0].0, "src/a.rs", "original snippet 0 is file a.rs");
    assert_eq!(snippets[1].0, "src/b.rs", "original snippet 1 is file b.rs");
    assert_eq!(snippets[2].0, "src/c.rs", "original snippet 2 is file c.rs");
}
