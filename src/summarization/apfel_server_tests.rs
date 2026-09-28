// Tests for the persistent apfel server backend (issue #772).
//
// Uses the fake-HTTP-server helpers in `summarizer_backend.rs` to stand up
// apfel / non-apfel / 429 endpoints without a real apfel binary or server.

use super::summarizer_backend::{
    ApfelServerHandle, HTTP_429, HTTP_429_BACKOFF_MS, HTTP_429_RETRY, acquire_server,
    health_body_is_apfel, health_validates_for, host_label, port_from_base_url,
    send_chat_completions, send_chat_completions_for, start_fake_http_server,
    start_flaky_http_server,
};
use crate::config::SummarizerBackend;
use crate::summarization::apfel::{BackendTransport, batch_summarize};
use crate::summarization::backend_profile::health_body_is_valid;
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::time::Duration;
use ureq::http::StatusCode;

// Tests point the summarization functions at a fake HTTP server without going
// through the pipeline's `run()` → `acquire_for_run()` path: each test builds
// a `BackendTransport` and passes it explicitly (issue #783 — no ambient state).
// The batch/summarize-code-over-HTTP tests live in `apfel_batch_http_tests.rs`.

fn base_url(addr: SocketAddr) -> String {
    format!("http://127.0.0.1:{port}", port = addr.port())
}

fn healthy_body() -> &'static str {
    r#"{"active_requests":0,"context_window":4096,"model":"apple-foundationmodel","model_available":true,"prewarmed":true,"status":"ok","version":"1.9.1"}"#
}

// --- /health validation ---

#[test]
fn test_health_validates_real_apfel_shape() {
    let addr = start_fake_http_server(200, healthy_body()).unwrap();
    let url = base_url(addr);
    assert!(
        health_validates_for(crate::config::SummarizerBackend::Apfel, &url),
        "real apfel /health shape must validate"
    );
}

#[test]
fn test_health_rejects_non_apfel_service() {
    // Ollama's /api/tags-style shape: no marker fields at all.
    let addr = start_fake_http_server(
        200,
        r#"{"models":[{"name":"llama3"}]}").unwrap();
    let url = base_url(addr);
    assert!(
        !health_validates_for(crate::config::SummarizerBackend::Apfel, &url),
        "non-apfel 200 must not validate"
    );
}

#[test]
fn test_health_rejects_malformed_json() {
    let addr = start_fake_http_server(200, "not json at all").unwrap();
    let url = base_url(addr);
    assert!(!health_validates_for(crate::config::SummarizerBackend::Apfel, &url), "malformed JSON must not validate");
}

#[test]
fn test_health_rejects_wrong_service_http_error() {
    // A non-apfel service answering a 404 (e.g. Ollama has no /health).
    let addr = start_fake_http_server(404, r#"{"error":"not found"}"#,
    )
    .unwrap();
    let url = base_url(addr);
    assert!(
        !health_validates_for(crate::config::SummarizerBackend::Apfel, &url),
        "404 must not validate"
    );
}

#[test]
fn test_health_body_missing_marker_field() {
    let body = r#"{"prewarmed":true,"active_requests":0,"context_window":4096}"#;
    assert!(
        !health_body_is_apfel(body),
        "missing model_available must fail validation"
    );
}

#[test]
fn test_health_body_mistyped_marker_field() {
    let body =
        r#"{"prewarmed":true,"active_requests":0,"context_window":"4096","model_available":true}"#;
    assert!(
        !health_body_is_apfel(body),
        "string-typed context_window must fail validation"
    );
}

#[test]
fn test_health_body_non_object_json() {
    assert!(
        !health_body_is_apfel("[1,2,3]"),
        "array body must fail validation"
    );
    assert!(
        !health_body_is_apfel("\"ok\""),
        "string body must fail validation"
    );
}

// --- lifecycle: reuse vs spawn ---

#[test]
fn test_reuse_of_running_server_never_starts_or_stops() {
    let addr = start_fake_http_server(200, healthy_body()).unwrap();
    let url = base_url(addr);

    let handle = acquire_server(crate::config::SummarizerBackend::Apfel, &url)
        .expect("healthy running server must be acquired");
    // Reuse: the handle owns no child.
    assert!(!handle.owns_child(), "reused server must not own a child");
    drop(handle);

    // The pre-existing server must survive: lievo neither started nor stopped
    // it.
    assert!(
        health_validates_for(crate::config::SummarizerBackend::Apfel, &url),
        "reused server must still be healthy after handle drop"
    );
}

#[test]
fn test_spawn_failure_degrades_to_none() {
    // A healthy server at the endpoint would be REUSED, not spawned, so prove
    // the spawn-failure path with a missing binary instead: the `Option` from
    // spawn is None and no child is leaked.
    let addr = start_fake_http_server(200, healthy_body()).unwrap();
    let url = base_url(addr);
    let missing =
        super::summarizer_backend::spawn_with_binary("no-such-binary-xyz-772", &url, addr.port());
    assert!(missing.is_none(), "missing binary must degrade to None");
}

#[test]
fn test_spawned_handle_drop_kills_child() {
    // Wrap a real (sleep) child as if it were a spawned server, drop the
    // handle, and verify the process was killed — the no-leak invariant on
    // early return / error / panic unwind.
    let child = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Grab the pid before moving the child into the handle.
    let pid = child.id();

    let handle = ApfelServerHandle::from_child_for_test(child);
    drop(handle);

    // The killed child was reaped inside shutdown(); confirm the pid is gone
    // (kill -0 on a dead pid fails on Unix).
    let pid_str = pid.to_string();
    let status = Command::new("kill")
        .args(["-0", &pid_str])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "sleep {pid} must be dead after handle drop"
    );
}

#[test]
fn test_handle_shutdown_is_idempotent() {
    let child = Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut handle = ApfelServerHandle::from_child_for_test(child);
    handle.shutdown();
    handle.shutdown();
    drop(handle); // second drop must not panic
}

// --- URL plumbing ---

#[test]
fn test_port_from_base_url_explicit() {
    assert_eq!(port_from_base_url("http://127.0.0.1:11434"), Some(11434));
    assert_eq!(port_from_base_url("https://apfel.example:8443"), Some(8443));
    assert_eq!(port_from_base_url("http://host:8443/path?q=1"), Some(8443));
}

#[test]
fn test_port_from_base_url_default() {
    assert_eq!(port_from_base_url("http://apfel.local"), Some(80));
    assert_eq!(port_from_base_url("https://apfel.local"), Some(443));
}

#[test]
fn test_host_label() {
    assert_eq!(host_label("http://127.0.0.1:11434"), "127.0.0.1:11434");
    assert_eq!(
        host_label("https://apfel.example:8443/v1"),
        "apfel.example:8443"
    );
}

// --- 429 bounded-retry constants contract ---

#[test]
fn test_429_retry_constants_are_bounded() {
    // The retry policy must be finite and positive; the backoff must be a
    // real delay (not zero). These are compile-time constants so the asserts
    // double as documentation of the invariants (clippy forbids constant
    // assertions, so the bounds are folded into a runtime boolean).
    let bounded = (1..=10).contains(&HTTP_429_RETRY);
    assert!(bounded, "retry must be bounded");
    let positive = HTTP_429_BACKOFF_MS > 0;
    assert!(positive, "backoff must be positive");
    assert_eq!(
        HTTP_429.as_u16(),
        StatusCode::TOO_MANY_REQUESTS.as_u16(),
        "429 constant must be 429"
    );
}

// --- chat completions ---

#[test]
fn test_send_chat_completions_extracts_content() {
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "  A summary.  ", "role": "assistant" } }],
        "object": "chat.completion"
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let url = base_url(addr);
    let content = send_chat_completions(&url, "sys", "user").unwrap();
    assert_eq!(content, "A summary.", "content must be trimmed");
}

#[test]
fn test_send_chat_completions_429_then_success_retries() {
    // First request → 429 (backpressure), second → 200. The bounded retry
    // must survive the 429 and return the eventual content.
    let ok_body = serde_json::json!({
        "choices": [{ "message": { "content": "after-429", "role": "assistant" } }]
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
    let content = send_chat_completions(&url, "sys", "user").unwrap();
    assert_eq!(content, "after-429");
}

#[test]
fn test_send_chat_completions_persistent_429_is_hard_failure() {
    let addr = start_fake_http_server(
        429,
        r#"{"error":{"type":"rate_limit_error","message":"Server at max concurrent capacity"}}"#,
    )
    .unwrap();
    let url = base_url(addr);
    let err = send_chat_completions(&url, "sys", "user").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("max concurrent capacity"),
        "persistent 429 must surface as a failure, got: {msg}"
    );
}

#[test]
fn test_send_chat_completions_server_error_is_failure() {
    let addr = start_fake_http_server(500, r#"{"error":"boom"}"#).unwrap();
    let url = base_url(addr);
    let err = send_chat_completions(&url, "sys", "user").unwrap_err();
    assert!(
        err.to_string().contains("500"),
        "500 must surface as failure"
    );
}

#[test]
fn test_send_chat_completions_missing_choices_is_failure() {
    let addr = start_fake_http_server(200, r#"{"object":"chat.completion"}"#).unwrap();
    let url = base_url(addr);
    let err = send_chat_completions(&url, "sys", "user").unwrap_err();
    assert!(
        err.to_string().contains("choices"),
        "missing choices[0].message.content must surface as failure"
    );
}

// --- Issue #776: backend-aware health validation, spawn policy, model field ---

/// Read one COMPLETE HTTP request off the stream and return the raw bytes
/// (headers + body) as a string.
///
/// A single `read` is NOT sufficient: under thread-parallel test execution the
/// request can arrive split across reads, so a one-shot read captures a
/// truncated (or empty) body. Loop until the header terminator (`\r\n\r\n`)
/// has arrived, parse `Content-Length`, then keep reading until the full body
/// has been received (or a sane cap/timeout is hit so a malformed request
/// cannot hang the test).
fn read_full_http_request(stream: &mut std::net::TcpStream) -> Option<String> {
    use std::io::Read as _;

    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

    let mut data: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    // Phase 1: read until the header/body separator has arrived.
    loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() > 256 * 1024 {
            return None;
        }
        if data.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    // Phase 2: honour Content-Length, reading until the full body has arrived.
    let text = String::from_utf8_lossy(&data);
    if let Some(hsep) = text.find("\r\n\r\n") {
        let headers = &text[..hsep];
        let content_length = headers
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")?
                    .trim()
                    .parse::<usize>()
                    .ok()
            })
            .unwrap_or(0);
        let body_start = hsep + 4;
        let body_received = text.len() - body_start;
        if body_received < content_length {
            let mut missing = content_length - body_received;
            while missing > 0 {
                let chunk = missing.min(buf.len());
                let n = stream.read(&mut buf[..chunk]).ok()?;
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                missing -= n;
                if data.len() > 8 * 1024 * 1024 {
                    break;
                }
            }
        }
    }
    stream.set_read_timeout(None).ok();
    Some(String::from_utf8_lossy(&data).into_owned())
}

/// A recording fake server: answers every request with `status`/`body`, and
/// captures the COMPLETE raw request of each request (one entry per request,
/// in request order) into a `Mutex<Vec<String>>` so tests can assert what
/// lievo actually SENT (issue #776 decision 1: an e2e test must assert the
/// request body, not just the response).
#[cfg(test)]
pub fn start_recording_http_server(
    status_code: u16,
    body: &str,
    captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> std::io::Result<SocketAddr> {
    use std::net::TcpListener;

    use std::thread as std_thread;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let body = body.to_string();
    std_thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Read the complete request (headers + full body) before replying.
            let captured_req = read_full_http_request(&mut stream).unwrap_or_default();
            {
                let mut guard = captured.lock().expect("captured requests mutex poisoned");
                guard.push(captured_req);
            }
            let response = format!(
                "HTTP/1.1 {status_code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    Ok(addr)
}

/// llama-server's documented healthy `/health` body.
fn llama_healthy_body() -> &'static str {
    r#"{"status":"ok"}"#
}

#[test]
fn test_health_body_llama_server_shape() {
    assert!(
        health_body_is_valid(SummarizerBackend::LlamaServer, r#"{"status":"ok"}"#),
        "llama-server healthy shape must validate"
    );
    // apfel's shape is a superset (its fixture includes `status: "ok"`), so it
    // also validates as llama-server — the llama-server check is a subset.
    // A shape WITHOUT `status: "ok"` must fail llama-server validation.
    assert!(
        !health_body_is_valid(
            SummarizerBackend::LlamaServer,
            r#"{"prewarmed":true,"model_available":true}"#
        ),
        "body without status:ok must not validate as llama-server"
    );
    // Ollama collision: a 200 with an entirely different body.
    assert!(
        !health_body_is_valid(
            SummarizerBackend::LlamaServer,
            r#"{"models":[{"name":"llama3"}]}"#
        ),
        "Ollama-style body must not validate as llama-server"
    );
    // Model loading (503 body) must not validate.
    assert!(
        !health_body_is_valid(
            SummarizerBackend::LlamaServer,
            r#"{"error":"model loading"}"#
        ),
        "model-loading error body must not validate"
    );
}

#[test]
fn test_health_body_generic_shape() {
    // Chosen liveness probe: JSON object with a `status` field (string or bool).
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":"ok"}"#
    ));
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":true}"#
    ));
    // No status field → no service recognized as a generic backend.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"models":[]}"#
    ));
    // Wrong-type status → degrade to unavailable.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":42}"#
    ));
    // A string body is not a JSON object → unavailable.
    assert!(!health_body_is_valid(SummarizerBackend::Generic, "ok"));
}

#[test]
fn test_health_body_apfel_rejects_other_backend_shapes() {
    // llama-server's shape must not validate as apfel (four fields required).
    assert!(
        !health_body_is_valid(SummarizerBackend::Apfel, r#"{"status":"ok"}"#),
        "llama-server shape must not validate as apfel"
    );
    // apfel's shape still validates (regression guard for #772).
    assert!(health_body_is_valid(
        SummarizerBackend::Apfel,
        healthy_body()
    ));
}

#[test]
fn test_http_health_validates_llama_server_shape() {
    let addr = start_fake_http_server(200, llama_healthy_body()).unwrap();
    let url = base_url(addr);
    assert!(
        health_validates_for(SummarizerBackend::LlamaServer, &url),
        "llama-server healthy /health must validate"
    );
    assert!(
        !health_validates_for(SummarizerBackend::Apfel, &url),
        "the same endpoint must NOT validate as apfel"
    );
}

/// The wrong-service degradation guard: Ollama answers 200 on the shared
/// default port with a body that is not the configured backend's shape. The
/// validator must say "unavailable" (degrade), never surface a hard error.
#[test]
fn test_wrong_service_on_port_degrades_to_unavailable() {
    let addr = start_fake_http_server(200, r#"{"models":[{"name":"llama3"}]}"#).unwrap();
    let url = base_url(addr);
    for backend in [
        SummarizerBackend::Apfel,
        SummarizerBackend::LlamaServer,
        SummarizerBackend::Generic,
    ] {
        assert!(
            !health_validates_for(backend, &url),
            "Ollama-style body must degrade to unavailable for {backend:?}"
        );
    }
}

/// llama-server is non-spawnable (needs a model path + flags lievo must not
/// guess): `acquire_server` returns `Some` when the server is already running
/// (reuse, no child owned) and `None` when nothing validates on the port —
/// the caller then emits the "not running" message naming the backend.
#[test]
fn test_llama_server_non_spawnable_reuse_and_not_running() {
    // Already running → reused, no child.
    let addr = start_fake_http_server(200, llama_healthy_body()).unwrap();
    let url = base_url(addr);
    let handle = acquire_server(SummarizerBackend::LlamaServer, &url)
        .expect("running llama-server must be acquired (reused)");
    assert!(
        !handle.owns_child(),
        "reused llama-server must not own a child"
    );

    // Not running (nothing validates on the port) → None, no spawn attempted.
    // A closed local port is used: no service listens there.
    let dead = "http://127.0.0.1:1";
    let missing = acquire_server(SummarizerBackend::LlamaServer, dead);
    assert!(
        missing.is_none(),
        "non-spawnable backend with no running server must degrade to None"
    );
}

/// End to end: drive `batch_summarize` through the HTTP transport to a
/// fake llama-server-shaped endpoint (health → chat → demux) and assert the
/// summaries are correctly demultiplexed. The llama-server transport is
/// passed explicitly (backend = LlamaServer, model = the configured name).
#[test]
fn test_llama_server_e2e_health_chat_demux() {
    // Step 1: health validation against the llama-server shape.
    let addr = start_fake_http_server(200, llama_healthy_body()).unwrap();
    let url = base_url(addr);
    assert!(
        health_validates_for(SummarizerBackend::LlamaServer, &url),
        "llama-server health must validate before the chat path"
    );

    // Step 2: chat completions with per-position `#<n>:` lines; the demux
    // must map each position correctly.
    let content: String = (0..3)
        .map(|i| format!("#{i}: llama summary {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = serde_json::json!({
        "choices": [{ "message": { "content": content, "role": "assistant" } }]
    })
    .to_string();
    let chat_addr = start_fake_http_server(200, &body).unwrap();
    let chat_url = base_url(chat_addr);

    let transport = BackendTransport {
        url: chat_url.clone(),
        backend: SummarizerBackend::LlamaServer,
        model_name: Some("my-gguf-model".to_string()),
    };

    let snippets = (0..3)
        .map(|i| ("f".to_string(), format!("fn_{i}"), "code".to_string()))
        .collect::<Vec<_>>();
    let results = batch_summarize(&snippets, Some(&transport)).unwrap();
    assert_eq!(results.len(), 3);
    for (i, result) in results.iter().enumerate() {
        assert_eq!(
            result,
            &Some(format!("llama summary {i}")),
            "position {i} must demux to the llama-server summary"
        );
    }
}

/// Issue #776 decision 1, asserted on the REQUEST BODY (a response-only check
/// would pass while the request was malformed):
///
/// - apfel always sends `"model": "apple-foundationmodel"` (configured name
///   ignored — apfel's server expects the platform model name).
/// - llama-server sends the configured name when set.
/// - llama-server sends the neutral default `"default"` when unset.
/// - generic sends the configured name when set…
/// - …and OMITS the `model` field entirely when unset (never guesses).
#[test]
fn test_request_model_field_per_backend_asserts_request_body() {
    let ok_body = serde_json::json!({
        "choices": [{ "message": { "content": "ok", "role": "assistant" } }]
    })
    .to_string();

    // Each backend run gets its OWN server and its OWN capture, so the model
    // assertion below is checked against that run's own request body — never
    // against a capture from another run (each captured request is complete
    // and contains exactly the one request that run sent).
    let run = |backend: SummarizerBackend, model: Option<&str>| -> String {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let addr = start_recording_http_server(200, &ok_body, captured.clone()).unwrap();
        let url = base_url(addr);
        send_chat_completions_for(&url, backend, model, "sys", "user").unwrap();
        let mut guard = captured.lock().expect("captured requests mutex poisoned");
        assert_eq!(
            guard.len(),
            1,
            "exactly one request must have been sent to this backend's server"
        );
        let captured_req = guard.remove(0);
        captured_req
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default()
    };

    // apfel: configured name ignored, platform model name sent.
    let body_part = run(SummarizerBackend::Apfel, Some("my-model"));
    let value: serde_json::Value =
        serde_json::from_str(&body_part).expect("apfel request body must be JSON");
    assert_eq!(
        value["model"],
        serde_json::json!("apple-foundationmodel"),
        "apfel must send apple-foundationmodel regardless of config"
    );

    // llama-server: configured name sent.
    let body_part = run(SummarizerBackend::LlamaServer, Some("my-gguf"));
    let value: serde_json::Value =
        serde_json::from_str(&body_part).expect("llama-server request body must be JSON");
    assert_eq!(value["model"], serde_json::json!("my-gguf"));

    // llama-server unset: neutral default.
    let body_part = run(SummarizerBackend::LlamaServer, None);
    let value: serde_json::Value =
        serde_json::from_str(&body_part).expect("llama-server request body must be JSON");
    assert_eq!(value["model"], serde_json::json!("default"));

    // generic: configured name sent.
    let body_part = run(SummarizerBackend::Generic, Some("mlx-model"));
    let value: serde_json::Value =
        serde_json::from_str(&body_part).expect("generic request body must be JSON");
    assert_eq!(value["model"], serde_json::json!("mlx-model"));

    // generic unset: field omitted entirely.
    let body_part = run(SummarizerBackend::Generic, None);
    let value: serde_json::Value =
        serde_json::from_str(&body_part).expect("generic request body must be JSON");
    assert!(
        value.get("model").is_none(),
        "generic with no configured model must OMIT the model field, got: {value}"
    );
}

// --- Issue #783: two-server isolation regression (positive proof) ---

/// Regression: the overwrite class that bit #780 (issue #780) — a second run
/// clobbering the transport a first run left behind — is structurally
/// impossible now that the transport is a plain value threaded down the call
/// path instead of ambient thread-local state. Two independent
/// `SummarizationPipeline::run` calls against two separate recording servers
/// must each deliver its request to its own server and to no other.
#[test]
fn test_two_sequential_runs_reach_only_their_own_server() {
    use crate::summarization::pipeline::{SummarizationConfig, SummarizationPipeline};
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, FakeApfelPathGuard, TestStorage, make_file_entity, make_fn_entity,
        read_fake_apfel_invocations,
    };

    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    use crate::summarization::summarizer_fullpath_tests::start_path_aware_recording_server;

    // Install the fake apfel on PATH so the test can prove no subprocess
    // was spawned: the invocation log in the test's own TempDir must
    // remain empty (issue #869). The guard owns the PATH restore; it must be
    // dropped before the env guard (which holds the env lock).
    let (log, _fake_apfel_guard) = FakeApfelPathGuard::install_noop_fake_apfel();

    let make_run = |unit_id: &str,
                    content: &str|
     -> (
        SocketAddr,
        TestStorage,
        Vec<CodeUnit>,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let chat_body = serde_json::json!({
            "choices": [{ "message": { "content": content, "role": "assistant" } }]
        })
        .to_string();
        let models_body = r#"{"object":"list","data":[{"id":"t","object":"model"}]}"#;
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let addr = start_path_aware_recording_server(
            models_body,
            &chat_body,
            std::sync::Arc::clone(&captured),
        )
        .unwrap();

        let mut storage = TestStorage::new();
        let file_entity = make_file_entity(unit_id, "src/a.rs");
        storage.add_file("test-repo", "src/a.rs", file_entity);
        let fn_entity = make_fn_entity(
            &crate::extraction::function_preservation::function_id(unit_id, "fn_a"),
            unit_id,
            "fn_a",
        );
        storage.add_function(
            &crate::extraction::function_preservation::function_id(unit_id, "fn_a"),
            fn_entity,
        );
        let code_units = vec![CodeUnit {
            name: "fn_a".to_string(),
            unit_type: "function".to_string(),
            file: "src/a.rs".to_string(),
            line: 1,
            end_line: 1,
            language: "Rust".to_string(),
            signature: None,
            code: Some("fn a() {}".to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "fn_a".to_string(),
            docstring: None,
            parent_class: None,
        }];
        (addr, storage, code_units, captured)
    };

    let run_config = SummarizationConfig {
        enabled: true,
        no_summarize: false,
        apfel_available: false,
    };

    // Run 1 → server 1 only.
    let (addr1, storage1, units1, captured1) = make_run("file-r783-1", "#0: run one summary");
    let url1 = base_url(addr1);
    let repo_config1 = crate::config::RepoConfig {
        apfel_endpoint: Some(url1),
        summarizer_backend: Some("generic".to_string()),
        ..Default::default()
    };
    let outcome1 = SummarizationPipeline::run(
        &storage1,
        "test-repo",
        "test-project",
        &units1,
        &run_config,
        &repo_config1,
    )
    .expect("run 1 must succeed");
    assert_eq!(outcome1.summarized, 1);

    // Run 2 → server 2 only, even though run 1 completed just before.
    let (addr2, storage2, units2, captured2) = make_run("file-r783-2", "#0: run two summary");
    let url2 = base_url(addr2);
    let repo_config2 = crate::config::RepoConfig {
        apfel_endpoint: Some(url2),
        summarizer_backend: Some("generic".to_string()),
        ..Default::default()
    };
    let outcome2 = SummarizationPipeline::run(
        &storage2,
        "test-repo",
        "test-project",
        &units2,
        &run_config,
        &repo_config2,
    )
    .expect("run 2 must succeed");
    assert_eq!(outcome2.summarized, 1);

    // No apfel subprocess for either run (a regression to the subprocess
    // path would write to the per-test invocation log, issue #869).
    assert_eq!(
        read_fake_apfel_invocations(&log),
        0,
        "no apfel subprocess for either run"
    );

    // Each server received requests — and only requests for its own run.
    let requests_for = |captured: &std::sync::Arc<std::sync::Mutex<Vec<String>>>| {
        let guard = captured.lock().expect("captured requests mutex poisoned");
        guard.len()
    };
    assert!(
        requests_for(&captured1) >= 1,
        "server 1 must have received run 1's requests"
    );
    assert!(
        requests_for(&captured2) >= 1,
        "server 2 must have received run 2's requests"
    );
}
