// Full-path tests for the generic summarizer backend (issue #780).
//
// The module-level tests in `apfel_server_tests.rs` drive the HTTP transport
// directly (via the thread-local override) and never go through
// `SummarizationPipeline::run` → `acquire_for_run` → thread-local → batch
// wiring. These tests close that gap:
//
// - A real `RepoConfig` naming a `generic` backend against a recording fake
//   HTTP server drives `SummarizationPipeline::run`; the test asserts a POST
//   actually arrived at the endpoint (including the Authorization header
//   when a token is set) AND that no apfel subprocess was spawned.
// - Unit tests for the health-probe order (generic tries `/v1/models` first,
//   falls through to `/health`), the Authorization header (present with a
//   token, absent without, on both probes and chat), and the token-leak
//   guards (token never appears in error strings or degradation messages).
//
use std::net::SocketAddr;
use std::time::Duration;

use crate::config::{RepoConfig, SummarizerBackend};
use crate::summarization::pipeline::server_dispatch::Degradation;
use crate::summarization::summarizer_backend::{health_validates_for, send_chat_completions_for};

// ── Test-only serial guard for env-var-mutating tests ────────────────────

/// `LIEVO_SUMMARIZER_TOKEN` is process-wide; `summarizer_token()` reads it
/// at call time. Tests that set and clear it must not run concurrently with
/// each other, or one test's token will leak into another test's request
/// headers. This mutex serializes all env-var-mutating tests.
///
/// The crate-wide env-var lock (issue #863): one mutex for every
/// env-mutating test in the crate: `std::env::set_var` is process-global,
/// so a guard that only serialized this module could still race
/// `LIEVO_MCP_TOOLS` / `PATH` mutations in another module.
//
// `pub(crate)` so `pipeline_rollup_tests::install_n_echo` — which also
// mutates PATH — can serialize against the env-var tests below rather than
// racing them on the fake `apfel` binary path (ETXTBSY on exec).
pub(crate) fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::test_env_support::env_lock()
}

fn set_token(token: &str) {
    unsafe { std::env::set_var("LIEVO_SUMMARIZER_TOKEN", token) };
}
fn clear_token() {
    unsafe { std::env::remove_var("LIEVO_SUMMARIZER_TOKEN") };
}

// ── HTTP helpers ───────────────────────────────────────────────────────────

fn base_url(addr: SocketAddr) -> String {
    format!("http://127.0.0.1:{}", addr.port())
}

/// Read one complete HTTP request (headers + body) off the stream.
///
/// A single `read` is not sufficient under thread-parallel test execution:
/// the request can arrive split across reads. Loop until `\r\n\r\n` has
/// arrived, parse `Content-Length`, then read until the full body has been
/// received (or a sane cap is hit).
fn read_full_http_request(stream: &mut std::net::TcpStream) -> Option<String> {
    use std::io::Read;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut data: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
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

/// Build an HTTP response string with the given status, JSON body, and
/// correct Content-Length.
fn http_response(status: u16, body: &str) -> String {
    let crlf = "\r\n";
    format!(
        "HTTP/1.1 {status} X{crlf}Content-Type: application/json{crlf}Content-Length: {}{crlf}Connection: close{crlf}{crlf}{body}",
        body.len()
    )
}

/// A path-aware recording fake server: `/v1/chat/completions` gets
/// `chat_body`; everything else (including `/v1/models` and `/health`) gets
/// `other_body`. Captures every raw request into `captured`.
///
/// The reader is `Content-Length`-aware (see `read_full_http_request`) — a
/// single-read helper caused a flake in #776, so the recording harness
/// must read the complete request.
pub(crate) fn start_path_aware_recording_server(
    other_body: &str,
    chat_body: &str,
    captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> std::io::Result<SocketAddr> {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let other_body = other_body.to_string();
    let chat_body = chat_body.to_string();
    let captured_thread = std::sync::Arc::clone(&captured);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let captured_req = read_full_http_request(&mut stream).unwrap_or_default();
            let first_line = captured_req.lines().next().unwrap_or("").to_string();
            {
                let mut guard = captured_thread.lock().expect("captured mutex poisoned");
                guard.push(captured_req);
            }
            let body = if first_line.contains("/v1/chat/completions") {
                chat_body.clone()
            } else {
                other_body.clone()
            };
            let response = http_response(200, &body);
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    Ok(addr)
}

// ── Full-path tests ────────────────────────────────────────────────────────

/// The full-path test (issue #780, central requirement): drive
/// `SummarizationPipeline::run` end to end with a `RepoConfig` naming a
/// `generic` backend against a recording fake HTTP server — no thread-local
/// shortcut, no direct `batch_summarize` call, no static override in force.
///
/// Asserts:
/// 1. A POST actually arrived at `/v1/chat/completions` (exactly one, and
///    the health probe went to `/v1/models`).
/// 2. The POST is a real HTTP POST (method check on the first line).
/// 3. No apfel subprocess was spawned (the per-test fake apfel's invocation
///    log in its own TempDir is empty — issue #869).
/// 4. The summaries are correct: the upserted entity carries the demuxed
///    `#0:` summary and the outcome count matches.
#[test]
fn test_full_path_generic_backend_pipeline_run() {
    use crate::summarization::pipeline::{SummarizationConfig, SummarizationPipeline};
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, TestStorage, make_file_entity, make_fn_entity, read_fake_apfel_invocations,
    };

    // Hold the env-var guard: `install_n_echo` in `pipeline_rollup_tests`
    // injects a fake `apfel` onto PATH while this test is running. The
    // guard serializes PATH-mutating tests so no other test can rewrite
    // the `apfel` binary this run is about to exec (ETXTBSY on exec).
    let _env_guard = env_guard();

    let models_body = r#"{"object":"list","data":[{"id":"test-model","object":"model"}]}"#;
    let chat_content = "#0: first summary";
    let chat_body = serde_json::json!({
        "choices": [{ "message": { "content": chat_content, "role": "assistant" } }]
    })
    .to_string();

    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let addr = start_path_aware_recording_server(
        models_body,
        &chat_body,
        std::sync::Arc::clone(&captured),
    )
    .unwrap();
    let url = base_url(addr);

    let repo_config = RepoConfig {
        apfel_endpoint: Some(url.clone()),
        summarizer_backend: Some("generic".to_string()),
        ..Default::default()
    };
    let run_config = SummarizationConfig::new(false, &repo_config, true);

    let mut storage = TestStorage::new();
    let file_entity = make_file_entity("file-fp780", "src/a.rs");
    storage.add_file("test-repo", "src/a.rs", file_entity);
    let fn_entity = make_fn_entity(
        &crate::extraction::function_preservation::function_id("file-fp780", "fn_a"),
        "file-fp780",
        "fn_a",
    );
    storage.add_function(
        &crate::extraction::function_preservation::function_id("file-fp780", "fn_a"),
        fn_entity,
    );

    let code_units = vec![CodeUnit {
        name: "fn_a".to_string(),
        unit_type: "function".to_string(),
        file: "src/a.rs".to_string(),
        line: 1,
        end_line: 3,
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

    clear_token();

    // Install the fake apfel on PATH so the test can prove no subprocess
    // was spawned: the invocation log in the test's own TempDir must
    // remain empty (issue #869).
    let (log, _apfel_guard) =
        crate::summarization::pipeline_tests_fixtures::FakeApfelPathGuard::install_noop_fake_apfel(
        );

    let outcome = SummarizationPipeline::run(
        &storage,
        "test-repo",
        "test-project",
        &code_units,
        &run_config,
        &repo_config,
    )
    .expect("run must succeed");

    // 3. No apfel subprocess was spawned: the per-test invocation log is empty.
    assert_eq!(
        read_fake_apfel_invocations(&log),
        0,
        "no apfel subprocess must be spawned when a generic backend is configured"
    );

    let guard = captured.lock().expect("captured mutex poisoned");
    let chat_posts: Vec<&String> = guard
        .iter()
        .filter(|req| {
            req.lines()
                .next()
                .is_some_and(|l| l.contains("/v1/chat/completions"))
        })
        .collect();
    let model_probes: Vec<&String> = guard
        .iter()
        .filter(|req| req.lines().next().is_some_and(|l| l.contains("/v1/models")))
        .collect();

    // 1. Exactly two requests: the /v1/models health probe and the chat POST.
    assert_eq!(
        guard.len(),
        2,
        "run must make exactly 2 requests: the /v1/models health probe and the chat POST; got: {guard:#?}"
    );
    assert_eq!(
        chat_posts.len(),
        1,
        "exactly one POST to /v1/chat/completions must have arrived"
    );
    assert_eq!(
        model_probes.len(),
        1,
        "exactly one health probe to /v1/models must have arrived"
    );
    assert!(
        chat_posts[0]
            .lines()
            .next()
            .map(|l| l.to_ascii_uppercase().starts_with("POST "))
            .unwrap_or(false),
        "the chat request must be a POST, got: {:?}",
        chat_posts[0].lines().next().unwrap_or("")
    );

    // 4. The summary made it into storage and the outcome counter reflects it.
    let upserted = storage.upserted.borrow();
    let fn_upserts: Vec<&crate::model::Entity> = upserted
        .iter()
        .filter(|e| e.id.starts_with("fn-"))
        .collect();
    assert_eq!(
        fn_upserts.len(),
        1,
        "exactly one function entity must be upserted, got: {upserted:#?}"
    );
    assert_eq!(
        fn_upserts[0].summary.as_deref(),
        Some("first summary"),
        "the upserted summary must be the demuxed #0: entry"
    );
    assert_eq!(
        outcome.summarized, 1,
        "the run outcome must count the one summarized function"
    );

    clear_token();
}

/// Same wiring as the full-path test, with `LIEVO_SUMMARIZER_TOKEN` set:
/// the chat POST and the health probe must both carry
/// `Authorization: Bearer <token>`, and the raw token value must never
/// appear in the request body.
#[test]
fn test_full_path_generic_backend_pipeline_run_sends_authorization_header() {
    use crate::summarization::pipeline::{SummarizationConfig, SummarizationPipeline};
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, TestStorage, make_file_entity, make_fn_entity, read_fake_apfel_invocations,
    };

    let _env_guard = env_guard();

    let models_body = r#"{"object":"list","data":[{"id":"m1","object":"model"}]}"#;
    let chat_content = "#0: header summary";
    let chat_body = serde_json::json!({
        "choices": [{ "message": { "content": chat_content, "role": "assistant" } }]
    })
    .to_string();

    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let addr = start_path_aware_recording_server(
        models_body,
        &chat_body,
        std::sync::Arc::clone(&captured),
    )
    .unwrap();
    let url = base_url(addr);

    let repo_config = RepoConfig {
        apfel_endpoint: Some(url.clone()),
        summarizer_backend: Some("generic".to_string()),
        ..Default::default()
    };
    let run_config = SummarizationConfig::new(false, &repo_config, true);

    let mut storage = TestStorage::new();
    let file_entity = make_file_entity("file-fp780a", "src/h.rs");
    storage.add_file("test-repo", "src/h.rs", file_entity);
    let fn_entity = make_fn_entity(
        &crate::extraction::function_preservation::function_id("file-fp780a", "fn_h"),
        "file-fp780a",
        "fn_h",
    );
    storage.add_function(
        &crate::extraction::function_preservation::function_id("file-fp780a", "fn_h"),
        fn_entity,
    );

    let code_units = vec![CodeUnit {
        name: "fn_h".to_string(),
        unit_type: "function".to_string(),
        file: "src/h.rs".to_string(),
        line: 1,
        end_line: 2,
        language: "Rust".to_string(),
        signature: None,
        code: Some("fn h() {}".to_string()),
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: "fn_h".to_string(),
        docstring: None,
        parent_class: None,
    }];

    // Install the fake apfel on PATH so the test can prove no subprocess
    // was spawned: the invocation log in the test's own TempDir must
    // remain empty (issue #869).
    let (log, _apfel_guard) =
        crate::summarization::pipeline_tests_fixtures::FakeApfelPathGuard::install_noop_fake_apfel(
        );

    set_token("fullpath-token-780");
    let result = SummarizationPipeline::run(
        &storage,
        "test-repo",
        "test-project",
        &code_units,
        &run_config,
        &repo_config,
    );
    clear_token();

    assert!(result.is_ok(), "run must succeed: {result:?}");

    let guard = captured.lock().expect("captured mutex poisoned");
    let auth = "authorization: bearer fullpath-token-780";
    let headers_of = |req: &String| {
        req.split_once("\r\n\r\n")
            .map(|(h, _)| h)
            .unwrap_or(req)
            .to_ascii_lowercase()
    };

    // Positive evidence that the HTTP path — not a subprocess — served the
    // request: exactly one /v1/models health probe plus exactly one chat
    // POST, and nothing else. This is per-test state; it does not depend on
    // the global invocation counter and independently proves the transport
    // was the fake server.
    assert_eq!(
        guard.len(),
        2,
        "exactly two requests must have been captured: the /v1/models probe and the chat POST; got: {guard:#?}"
    );
    let chat_posts: Vec<&String> = guard
        .iter()
        .filter(|req| {
            req.lines()
                .next()
                .is_some_and(|l| l.contains("/v1/chat/completions"))
        })
        .collect();
    assert_eq!(chat_posts.len(), 1, "one chat POST expected");
    assert!(
        headers_of(chat_posts[0]).contains(auth),
        "the chat POST must carry the Authorization header; got: {:?}",
        chat_posts[0]
    );
    let model_probes: Vec<&String> = guard
        .iter()
        .filter(|req| req.lines().next().is_some_and(|l| l.contains("/v1/models")))
        .collect();
    assert_eq!(model_probes.len(), 1, "one /v1/models probe expected");
    assert!(
        headers_of(model_probes[0]).contains(auth),
        "the health probe must carry the Authorization header too; got: {:?}",
        model_probes[0]
    );
    // The token belongs in the header only, never in the request body.
    let (_, chat_body_text) = chat_posts[0].split_once("\r\n\r\n").expect("request body");
    assert!(
        !chat_body_text.contains("fullpath-token-780"),
        "the raw token must never appear in the request body"
    );
    drop(guard);

    // The per-test invocation log must be empty: no apfel subprocess was
    // spawned for the generic backend (issue #869).
    assert_eq!(
        read_fake_apfel_invocations(&log),
        0,
        "no apfel subprocess for the generic backend"
    );

    clear_token();
}

// ── Degradation message content (issue #780 decision 1) ───────────────────

/// `Degradation::message()` must name the backend, the endpoint and the
/// reason, and must NEVER contain the bearer token value (the token is not
/// a field of the struct — this locks the contract).
#[test]
fn test_degradation_message_names_backend_endpoint_and_reason_and_never_token() {
    let degradation = Degradation {
        backend: SummarizerBackend::Generic,
        endpoint: "127.0.0.1:1234".to_string(),
        reason: "server not running; it must be started separately".to_string(),
    };
    let msg = degradation.message();

    assert!(
        msg.contains("generic"),
        "message must name the backend; got: {msg}"
    );
    assert!(
        msg.contains("127.0.0.1:1234"),
        "message must name the endpoint label; got: {msg}"
    );
    assert!(
        msg.contains("server not running; it must be started separately"),
        "message must carry the reason; got: {msg}"
    );
    assert!(
        msg.starts_with("warning:"),
        "the message is a warning; got: {msg}"
    );
    assert!(
        !msg.contains("fullpath-token-780") && !msg.contains("LIEVO_SUMMARIZER_TOKEN"),
        "message must never contain the token value or the env var name; got: {msg}"
    );

    // A llama-server degradation names that backend, not the default apfel.
    let llama = Degradation {
        backend: SummarizerBackend::LlamaServer,
        endpoint: "127.0.0.1:8080".to_string(),
        reason: "spawn or health validation failed".to_string(),
    };
    let llama_msg = llama.message();
    assert!(
        llama_msg.contains("llama-server"),
        "the message must name the configured llama-server backend; got: {llama_msg}"
    );
    assert!(
        !llama_msg.contains("apfel backend"),
        "a llama-server degradation must not name apfel; got: {llama_msg}"
    );
}

// ── Health probe order + Authorization header (unit level) ────────────────

/// For the `generic` backend, the health probe must try `/v1/models` first
/// and fall through to `/health` when the first path is non-validating.
#[test]
fn test_generic_health_probe_order() {
    // Case 1: /v1/models with standard list shape → validates on first probe.
    let captured1 = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let addr1 = start_path_aware_recording_server(
        r#"{"object":"list","data":[{"id":"m1","object":"model"}]}"#,
        r#"{"status":"ok"}"#,
        std::sync::Arc::clone(&captured1),
    )
    .unwrap();
    let url1 = base_url(addr1);

    let valid1 = health_validates_for(SummarizerBackend::Generic, &url1);
    assert!(
        valid1,
        "generic health must validate against a /v1/models list response"
    );
    let guard1 = captured1.lock().expect("captured mutex poisoned");
    assert_eq!(
        guard1.len(),
        1,
        "only one probe request for a validating first path"
    );
    assert!(
        guard1[0]
            .lines()
            .next()
            .unwrap_or("")
            .contains("/v1/models"),
        "the probe must hit /v1/models first"
    );

    // Case 2: both probe paths return a non-validating body → the probe
    // must try /v1/models AND /health before giving up.
    let captured2 = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let addr2 = start_path_aware_recording_server(
        r#"{"error":"no such endpoint"}"#,
        r#"{"status":"ok"}"#,
        std::sync::Arc::clone(&captured2),
    )
    .unwrap();
    let url2 = base_url(addr2);
    let valid2 = health_validates_for(SummarizerBackend::Generic, &url2);
    assert!(
        !valid2,
        "non-validating body on both paths must NOT validate"
    );
    let guard2 = captured2.lock().expect("captured mutex poisoned");
    assert!(
        guard2.len() >= 2,
        "both probe paths must have been tried (got {} requests)",
        guard2.len()
    );
    assert!(
        guard2[0]
            .lines()
            .next()
            .unwrap_or("")
            .contains("/v1/models"),
        "first probe must be /v1/models"
    );
    assert!(
        guard2[1].lines().next().unwrap_or("").contains("/health"),
        "second probe must be /health"
    );
}

/// When `LIEVO_SUMMARIZER_TOKEN` is set, the Authorization header must be
/// present on chat-completions requests. When unset, it must be absent.
#[test]
fn test_authorization_header_present_with_token_absent_without() {
    let _env_guard = env_guard();

    let ok_body =
        serde_json::json!({ "choices": [{ "message": { "content": "ok", "role": "assistant" } }] })
            .to_string();

    // Case 1: no token → no Authorization header.
    clear_token();
    {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let addr =
            start_path_aware_recording_server(&ok_body, &ok_body, std::sync::Arc::clone(&captured))
                .unwrap();
        let url = base_url(addr);
        send_chat_completions_for(&url, SummarizerBackend::Generic, None, "sys", "user").unwrap();
        let guard = captured.lock().expect("captured mutex poisoned");
        assert_eq!(guard.len(), 1);
        let headers = guard[0]
            .split_once("\r\n\r\n")
            .map(|(h, _)| h)
            .unwrap_or(&guard[0])
            .to_ascii_lowercase();
        assert!(
            !headers.contains("authorization"),
            "no Authorization header when token is unset; got: {headers}"
        );
    }

    // Case 2: token set → Authorization: Bearer <token> present.
    set_token("secret-token-12345");
    {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let addr =
            start_path_aware_recording_server(&ok_body, &ok_body, std::sync::Arc::clone(&captured))
                .unwrap();
        let url = base_url(addr);
        send_chat_completions_for(&url, SummarizerBackend::Generic, None, "sys", "user").unwrap();
        let guard = captured.lock().expect("captured mutex poisoned");
        assert_eq!(guard.len(), 1);
        let headers = guard[0]
            .split_once("\r\n\r\n")
            .map(|(h, _)| h)
            .unwrap_or(&guard[0])
            .to_ascii_lowercase();
        assert!(
            headers.contains("authorization: bearer secret-token-12345"),
            "Authorization: Bearer header must be present when token is set; got: {headers}"
        );
    }
    clear_token();
}

/// The health probe must carry the Authorization header when a token is
/// configured (a hosted endpoint may require auth for /v1/models too).
#[test]
fn test_health_probe_carries_authorization_header() {
    let _env_guard = env_guard();

    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let addr = start_path_aware_recording_server(
        r#"{"object":"list","data":[{"id":"m1","object":"model"}]}"#,
        r#"{"status":"ok"}"#,
        std::sync::Arc::clone(&captured),
    )
    .unwrap();
    let url = base_url(addr);

    set_token("probe-token-abc");
    let valid = health_validates_for(SummarizerBackend::Generic, &url);
    clear_token();

    assert!(
        valid,
        "generic health must validate with a /v1/models list response"
    );

    let guard = captured.lock().expect("captured mutex poisoned");
    assert_eq!(guard.len(), 1, "exactly one probe request");
    let headers = guard[0]
        .split_once("\r\n\r\n")
        .map(|(h, _)| h)
        .unwrap_or(&guard[0])
        .to_ascii_lowercase();
    assert!(
        headers.contains("authorization: bearer probe-token-abc"),
        "health probe must carry the Authorization header; got: {headers}"
    );
}

/// When the endpoint is unreachable and a token is configured, the error
/// message must NOT contain the token value.
#[test]
fn test_token_never_appears_in_error() {
    let _env_guard = env_guard();

    set_token("leak-detection-token-xyz");
    let dead_url = "http://127.0.0.1:1";
    let result =
        send_chat_completions_for(dead_url, SummarizerBackend::Generic, None, "sys", "user");
    clear_token();

    let err = result.expect_err("unreachable endpoint must fail");
    let err_text = format!("{err:?}");
    assert!(
        !err_text.contains("leak-detection-token-xyz"),
        "token must never appear in an error message: {err_text}"
    );
}

/// A configured generic backend whose endpoint is unreachable must degrade
/// (no handle) and must NOT spawn an apfel subprocess.
#[test]
fn test_unreachable_generic_backend_degrades_without_apfel_spawn() {
    use crate::summarization::pipeline::server_dispatch::acquire_for_run;
    use crate::summarization::pipeline_tests_fixtures::read_fake_apfel_invocations;

    // Hold the env lock: this test sets PATH (issue #869 / #863).
    let _env_guard = env_guard();

    // Install the fake apfel on PATH so the test can prove no subprocess
    // was spawned: the invocation log in the test's own TempDir must
    // remain empty (issue #869).
    let (log, _apfel_guard) =
        crate::summarization::pipeline_tests_fixtures::FakeApfelPathGuard::install_noop_fake_apfel(
        );

    let repo_config = RepoConfig {
        apfel_endpoint: Some("http://127.0.0.1:1".to_string()),
        summarizer_backend: Some("generic".to_string()),
        ..Default::default()
    };

    let (handle, _transport, degradation) = acquire_for_run(&repo_config);
    assert!(
        handle.is_none(),
        "unreachable generic backend must return no handle (degrade, not spawn)"
    );
    assert!(
        degradation.is_some_and(|d| d.backend == SummarizerBackend::Generic),
        "the degradation must name the configured backend"
    );

    // The per-test invocation log must be empty: no apfel subprocess was
    // spawned for the unreachable generic backend (issue #869).
    assert_eq!(
        read_fake_apfel_invocations(&log),
        0,
        "no apfel subprocess for an unreachable generic backend"
    );
}
