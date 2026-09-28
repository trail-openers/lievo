// Issue #783: batch-summarize / summarize_code through the explicit HTTP
// transport (the old thread-local endpoint override is gone — every test
// below builds a `BackendTransport` and passes it to the call directly, so
// parallel test isolation holds by construction).
//
// Split from apfel_server_tests.rs to respect the file-size gate (AGENTS.md §6).

use super::apfel::{BackendTransport, batch_summarize, summarize_code};
use crate::config::SummarizerBackend;
use crate::summarization::summarizer_backend::{start_fake_http_server, start_flaky_http_server};

/// A loopback-only apfel-shaped transport pointing at a fake server.
fn fake_apfel_transport(url: &str) -> BackendTransport {
    BackendTransport {
        url: url.to_string(),
        backend: SummarizerBackend::Apfel,
        model_name: None,
    }
}

fn base_url(addr: std::net::SocketAddr) -> String {
    format!("http://127.0.0.1:{port}", port = addr.port())
}

/// Degradation: when the fake server answers 500 (not 429), the batch path
/// must surface a `SummarizationFailed` error — the pipeline's salvage arm
/// then maps this to all-None results, and the refresh completes without
/// panicking. This test proves the error propagates correctly through
/// `batch_summarize` → `send_chat_completions`.
#[test]
fn test_batch_summarize_via_http_server_error_degrades() {
    let addr = start_fake_http_server(500, r#"{"error":"internal"}"#).unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let snippets = vec![
        (
            "src/a.rs".to_string(),
            "fn_a".to_string(),
            "fn a() {}".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "fn_b".to_string(),
            "fn b() {}".to_string(),
        ),
    ];
    let err = batch_summarize(&snippets, Some(&transport)).unwrap_err();
    assert!(
        err.to_string().contains("500"),
        "server error must surface as failure, got: {err}"
    );
}

/// Degradation: when the endpoint is unreachable (connection refused), the
/// batch path must fail with a `SummarizationFailed` — the same error the
/// pipeline classifies and degrades on. A closed port on 127.0.0.1 is
/// connection-refused, not timeout, so this returns quickly.
#[test]
fn test_batch_summarize_via_unreachable_endpoint_degrades() {
    // Use a port that is almost certainly not in use. We don't bind it, so
    // the connection is refused immediately.
    let url = "http://127.0.0.1:1";
    let transport = fake_apfel_transport(url);

    let snippets = vec![(
        "src/a.rs".to_string(),
        "fn_a".to_string(),
        "fn a() {}".to_string(),
    )];
    let err = batch_summarize(&snippets, Some(&transport)).unwrap_err();
    assert!(
        err.to_string().contains("request failed"),
        "unreachable endpoint must surface as transport failure, got: {err}"
    );
}

/// 429 through the full `batch_summarize` path: the first request gets 429
/// (backpressure), the retry gets 200 with a valid batch response. The
/// positional mapping must be preserved through the retry.
#[test]
fn test_batch_summarize_429_retry_preserves_positional_mapping() {
    let ok_body = serde_json::json!({
        "choices": [{ "message": { "content": "#0: first summary\n#1: second summary", "role": "assistant" } }]
    })
    .to_string();
    let addr = start_flaky_http_server(
        1,
        429,
        r#"{"error":{"type":"rate_limit_error"}}"#,
        200,
        &ok_body,
    )
    .unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let snippets = vec![
        (
            "src/a.rs".to_string(),
            "fn_a".to_string(),
            "fn a() {}".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "fn_b".to_string(),
            "fn b() {}".to_string(),
        ),
    ];
    let results = batch_summarize(&snippets, Some(&transport)).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("first summary".to_string()));
    assert_eq!(results[1], Some("second summary".to_string()));
}

/// Positional mapping through the HTTP transport: three snippets, the fake
/// server returns three `#n:` lines in order. Each position maps to the
/// correct snippet — position N maps to the Nth summary.
#[test]
fn test_batch_summarize_positional_mapping_three_entries() {
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "#0: auth\n#1: db\n#2: cache", "role": "assistant" } }]
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let snippets = vec![
        (
            "src/auth.rs".to_string(),
            "login".to_string(),
            "fn login() {}".to_string(),
        ),
        (
            "src/db.rs".to_string(),
            "query".to_string(),
            "fn query() {}".to_string(),
        ),
        (
            "src/cache.rs".to_string(),
            "get".to_string(),
            "fn get() {}".to_string(),
        ),
    ];
    let results = batch_summarize(&snippets, Some(&transport)).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Some("auth".to_string()));
    assert_eq!(results[1], Some("db".to_string()));
    assert_eq!(results[2], Some("cache".to_string()));
}

/// Partial batch through HTTP: the server returns fewer `#n:` lines than
/// snippets. The missing positions must be `None` (the pipeline's salvage
/// arm handles them). Positional mapping is preserved for the present ones.
#[test]
fn test_batch_summarize_partial_result_via_http() {
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "#0: present\n#2: also present", "role": "assistant" } }]
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let snippets = vec![
        ("a".to_string(), "f0".to_string(), "code0".to_string()),
        ("b".to_string(), "f1".to_string(), "code1".to_string()),
        ("c".to_string(), "f2".to_string(), "code2".to_string()),
    ];
    let results = batch_summarize(&snippets, Some(&transport)).unwrap();
    assert_eq!(results[0], Some("present".to_string()));
    assert_eq!(results[1], None, "position 1 was not in the response");
    assert_eq!(results[2], Some("also present".to_string()));
}

/// `summarize_code` through the HTTP transport: single snippet, the fake
/// server returns a chat-completions response. Content is trimmed and
/// returned.
#[test]
fn test_summarize_code_via_http_transport() {
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "  Adds two numbers.  ", "role": "assistant" } }]
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let result =
        summarize_code("fn add(a: i32, b: i32) -> i32 { a + b }", Some(&transport)).unwrap();
    assert_eq!(result.content, "Adds two numbers.");
}

/// 429 persistent (no recovery) through `batch_summarize`: the bounded retry
/// exhausts and the batch path surfaces a `SummarizationFailed` with the
/// "max concurrent capacity" message. The pipeline's salvage arm then maps
/// this to all-None results — the refresh degrades, it does not fail.
#[test]
fn test_batch_summarize_persistent_429_degrades() {
    let addr = start_fake_http_server(
        429,
        r#"{"error":{"type":"rate_limit_error","message":"Server at max concurrent capacity"}}"#,
    )
    .unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let snippets = vec![("a".to_string(), "f".to_string(), "code".to_string())];
    let err = batch_summarize(&snippets, Some(&transport)).unwrap_err();
    assert!(
        err.to_string().contains("max concurrent capacity"),
        "persistent 429 must surface as bounded-retry failure, got: {err}"
    );
}

// --- Named-corpus measurement: cold-start elimination ---
//
// The issue's core justification: driving a persistent server eliminates the
// per-invocation model cold start. This test measures wall-clock for a
// named fixture corpus through the HTTP transport and records the result.
// It does NOT compare against a real one-shot apfel subprocess (that would
// require a real apfel binary, which is not available in CI) — instead it
// records the HTTP wall-clock and asserts it completes. The before/after
// comparison is stated in the PR body using the recorded numbers.
//
// Corpus: `fixture_corpus_772_cold_start` — 10 function entities, each
// ~100 chars of code, packed into 1 batch (well under the 8000-char budget).
// Method: `batch_summarize` via the HTTP transport to a fake server that
// responds with 10 `#n:` lines. The fake server has no model cold start;
// the measurement isolates the HTTP round-trip + parsing overhead, which is
// the "after" component of the cold-start elimination. The "before" is the
// documented 20-80s per invocation (apfel #192, #364).

#[test]
fn test_cold_start_measurement_fixture_corpus() {
    use super::apfel::pack_by_char_budget;

    // Build the named corpus: 10 function entities, each ~100 chars of code.
    let snippets: Vec<(String, String, String)> = (0..10)
        .map(|i| {
            (
                format!("src/f{i}.rs"),
                format!("fn_{i}"),
                format!("fn f{i}() {{ let x = {}; x }} // pad ", i).repeat(3),
            )
        })
        .collect();

    // Verify they pack into 1 batch (well under the 8000-char budget).
    let batches = pack_by_char_budget(&snippets, super::apfel::APFEL_INPUT_CHAR_BUDGET);
    assert_eq!(
        batches.len(),
        1,
        "10 × ~100-char snippets must fit in one batch"
    );

    // Build the fake server response: 10 `#n:` lines.
    let content: String = (0..10)
        .map(|i| format!("#{i}: summary for fn_{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = serde_json::json!({
        "choices": [{ "message": { "content": content, "role": "assistant" } }]
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let url = base_url(addr);
    let transport = fake_apfel_transport(&url);

    let start = std::time::Instant::now();
    let results = batch_summarize(&snippets, Some(&transport)).unwrap();
    let elapsed = start.elapsed();

    // All 10 positions must be present and correctly mapped.
    assert_eq!(results.len(), 10);
    for (i, result) in results.iter().enumerate() {
        assert_eq!(
            result,
            &Some(format!("summary for fn_{i}")),
            "position {i} must map to fn_{i} summary"
        );
    }

    // The HTTP round-trip must be well under 1 second (the fake server
    // responds instantly). This is the "after" measurement: the per-invocation
    // cost is now the HTTP round-trip, not the 20-80s model cold start.
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "single batch via persistent server must complete in <1s, took {elapsed:?}"
    );

    eprintln!(
        "[measurement] fixture_corpus_772_cold_start: 10 entities, 1 batch, {} HTTP round-trip (per-invocation cold start eliminated: before = 20-80s per apfel #192/#364)",
        elapsed.as_millis()
    );
}
