// Persistent apfel server backend (issue #772) — health validation and
// run-scoped process lifecycle for the `apfel --serve` HTTP transport.
//
// lievo's summarization path used to spawn a fresh one-shot `apfel` subprocess
// per invocation, paying a model cold start every time. This module drives a
// persistent server instead:
//
// - If a healthy apfel server is already listening on the configured endpoint
//   (e.g. started via `brew services`), it is REUSED — lievo neither starts
//   nor stops it.
// - Otherwise lievo spawns `apfel --serve` on the endpoint's port for the
//   duration of the run and shuts down exactly that process, via RAII, on all
//   exit paths (early return, error, panic unwind).
//
// The HTTP request path itself (chat completions, 429 retry) lives here; the
// one-shot `run_apfel` call sites in `apfel.rs` are swapped to it in the
// transport-swap workstream. This module owns only the lifecycle and the
// `/health` validation that keeps a non-apfel service on the same port
// (Ollama uses the same default port) from being mistaken for apfel.
//
// Everything here is synchronous: ureq is a blocking client, consistent with
// the pipeline staying free of the tokio runtime (AGENTS.md: async only for
// I/O, core analysis stays synchronous).

use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};
use ureq::http::StatusCode;

use crate::config::SummarizerBackend;
use crate::error::Result;

// Command/Stdio are used by both the test helpers (fake HTTP servers) and the
// spawn path (spawn_with_binary), so they stay unconditional.
use std::process::{Command, Stdio};

/// Per-request timeout for a single chat-completions call, in seconds.
///
/// Directly inherits the former one-shot `APFEL_TIMEOUT_SECS = 300` so the
/// effective per-invocation bound is unchanged by the transport swap.
pub const SERVER_REQUEST_TIMEOUT_SECS: u64 = 300;

/// Bounded retry policy for HTTP 429 (server at max concurrent capacity):
/// retryable backpressure, not failure (another client may hold all permits).
pub const HTTP_429_RETRY: u32 = 3;
pub const HTTP_429_BACKOFF_MS: u64 = 500;

/// HTTP status code returned by the server at max concurrent capacity.
pub const HTTP_429: StatusCode = StatusCode::TOO_MANY_REQUESTS;

/// Bounded /health poll window after spawning the server: apfel prewarms the
/// model BEFORE its listener accepts connections (measured ~6s; apfel #192
/// documents 80s class on macOS 27), so a naive connect-immediately fails.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(1);
const HEALTH_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// The `apfel` binary to spawn when no server is already running.
const APFEL_BINARY: &str = "apfel";

/// RAII guard over an apfel server process that lievo spawned itself.
///
/// Drop kills the process, so the server cannot leak on any exit path —
/// early return, error, or panic unwind (Drop still runs on unwind). A reused
/// already-running server holds no child and drops as a no-op.
pub struct ApfelServerHandle {
    child: Option<Child>,
}

impl ApfelServerHandle {
    /// Spawn `apfel --serve` bound to `port` and wait (bounded) until its
    /// `/health` endpoint validates.
    ///
    /// On success the caller OWNS the process and MUST keep the handle alive
    /// until the run is complete (drop shuts the server down). On failure
    /// (spawn error, or health never validates inside the poll window) the
    /// child is killed and `None` is returned, so callers degrade to the
    /// unavailable-summarizer path.
    pub fn spawn(base_url: &str, port: u16) -> Option<Self> {
        spawn_with_binary(APFEL_BINARY, base_url, port)
    }

    /// Test access to whether the handle owns a child (false for a reused
    /// already-running server).
    #[cfg(test)]
    pub fn owns_child(&self) -> bool {
        self.child.is_some()
    }

    /// Test-only constructor wrapping an existing child as if lievo had
    /// spawned it (drives the leak/shutdown tests without a real apfel server).
    #[cfg(test)]
    pub fn from_child_for_test(child: Child) -> Self {
        Self { child: Some(child) }
    }

    /// Explicitly shut the server down early (idempotent).
    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ApfelServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Spawn `binary --serve --port <port>` and wait (bounded) until its
/// `/health` endpoint validates. Used by `spawn` with the real `apfel`
/// binary and directly by tests with a missing binary to prove the
/// spawn-failure → `None` degradation path without waiting out the full
/// health poll window.
pub fn spawn_with_binary(binary: &str, base_url: &str, port: u16) -> Option<ApfelServerHandle> {
    let mut child = match Command::new(binary)
        .args(["--serve", "--port"])
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return None,
    };

    let deadline = Instant::now() + HEALTH_WAIT_TIMEOUT;
    loop {
        // A server that dies during prewarm (bad port, missing Apple
        // Intelligence, crash) must not be leaked or waited on forever.
        if child_try_wait_dead(&mut child) {
            let _ = child.wait();
            return None;
        }
        if health_validates_for(SummarizerBackend::Apfel, base_url) {
            return Some(ApfelServerHandle { child: Some(child) });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(HEALTH_POLL_INTERVAL);
    }
}

/// `try_wait` that reports the child has already exited.
fn child_try_wait_dead(child: &mut Child) -> bool {
    child.try_wait().is_ok_and(|status| status.is_some())
}

/// Probe `GET {base_url}/health` with a short timeout and validate the body
/// against the backend's real introspection shape. Returns true only if the
/// body validates for `backend` (the gap-gate contract: a wrong-service 200
/// maps to "no server of this backend available", not a protocol error).
pub fn health_validates_for(backend: SummarizerBackend, base_url: &str) -> bool {
    for path in crate::summarization::backend_profile::health_probe_paths(backend) {
        match fetch_json_body(&format!("{base_url}{path}")) {
            // A non-JSON body (a 404 page from a server without the
            // endpoint) is not proof the backend is absent: the next probe
            // path may still validate, and a validating response is what
            // the backend contract accepts.
            FetchOutcome::Body(body) => {
                if crate::summarization::backend_profile::health_body_is_valid(backend, &body) {
                    return true;
                }
            }
            FetchOutcome::NotAvailable => return false,
            FetchOutcome::NotJson => continue,
        }
    }
    false
}

/// The result of fetching a JSON body from a probe endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchOutcome {
    /// The request succeeded and the body parsed as JSON (the body text).
    Body(String),
    /// The request failed at the transport level (connection refused, DNS)
    /// or with a non-2xx status: the endpoint offers nothing usable.
    NotAvailable,
    /// The request succeeded but the body was not JSON (e.g. a 404 HTML
    /// page from a server without the endpoint): try the next probe path.
    NotJson,
}

/// Fetch `url` with the probe timeout and the optional configured bearer
/// token (issue #780), classifying the outcome for `health_validates_for`.
fn fetch_json_body(url: &str) -> FetchOutcome {
    let mut request = ureq::get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(5)))
        .build();
    if let Some(token) = summarizer_token() {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    let response = match request.call() {
        Ok(response) => response,
        // A non-2xx status is "this path offers nothing usable" — the
        // next probe path (if any) is still tried.
        Err(_) => return FetchOutcome::NotAvailable,
    };
    let body = match response.into_body().read_to_string() {
        Ok(b) => b,
        Err(_) => return FetchOutcome::NotAvailable,
    };
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(value) => FetchOutcome::Body(value.to_string()),
        Err(_) => FetchOutcome::NotJson,
    }
}

/// Validate a /health JSON body against apfel's marker shape.
///
/// Required fields: `prewarmed` (bool), `active_requests` (number),
/// `context_window` (number), `model_available` (bool). A missing or mistyped
/// field (e.g. Ollama's shape, or any other service that happens to answer on
/// the configured port) fails validation and is treated as "no apfel
/// available".
pub fn health_body_is_apfel(body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    object.get("prewarmed").and_then(|v| v.as_bool()).is_some()
        && object
            .get("active_requests")
            .and_then(|v| v.as_u64())
            .is_some()
        && object
            .get("context_window")
            .and_then(|v| v.as_u64())
            .is_some()
        && object
            .get("model_available")
            .and_then(|v| v.as_bool())
            .is_some()
}

/// Read the configured summarizer bearer token: `Some(token)` when the
/// token environment variable (`SUMMARIZER_TOKEN_ENV`, defined in
/// `server_dispatch.rs`) is set to a non-empty value, `None` otherwise.
pub fn summarizer_token() -> Option<String> {
    std::env::var(crate::summarization::pipeline::server_dispatch::SUMMARIZER_TOKEN_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Resolve the apfel server port from a base URL. The URL is validated to be
/// http(s) in `config.rs`, so the scheme is always present. A missing port
/// falls back to 80 (http) / 443 (https).
pub fn port_from_base_url(base_url: &str) -> Option<u16> {
    let without_scheme = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))?;
    let host = without_scheme.split(['/', '?']).next()?;
    match host.rsplit_once(':') {
        Some((_, port)) => port.parse::<u16>().ok(),
        None => {
            let is_https = base_url.starts_with("https://");
            Some(if is_https { 443 } else { 80 })
        }
    }
}

/// Extract the host:port of a base URL, for display in log lines.
pub fn host_label(base_url: &str) -> String {
    let without_scheme = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
        .unwrap_or(base_url);
    without_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(base_url)
        .to_string()
}

/// Check whether a healthy apfel server is already listening at `base_url`.
///
/// Unlike `acquire_apfel_server`, this does NOT spawn a server — it only probes
/// `/health` and returns a simple `bool`. Used by the pipeline to decide whether
/// to go through the HTTP transport or fall back to the one-shot CLI path.
/// No side effects: nothing is started or stopped.
pub fn apfel_server_available(base_url: &str) -> bool {
    health_validates_for(SummarizerBackend::Apfel, base_url)
}

/// Decide the ownership model for the server at `base_url` (issues #772/#776):
///
/// - If a healthy server of the configured backend is already listening, it is
///   REUSED and the returned handle is a no-op that never starts or stops
///   anything.
/// - For `apfel` (the only spawnable backend, issue #776), lievo otherwise
///   spawns its own server for the duration of the run; the returned
///   `Some(handle)` owns that process and shuts it down on drop. A spawn
///   failure returns `None`, which callers map to the unavailable-summarizer
///   path.
/// - For `llama-server` and `generic` (non-spawnable: they need a model path
///   and flags lievo must not guess), a not-running server returns `None` —
///   the caller emits the backend-named "not running" message.
pub fn acquire_server(backend: SummarizerBackend, base_url: &str) -> Option<ApfelServerHandle> {
    if health_validates_for(backend, base_url) {
        // Already-running healthy server: reuse, never start/stop.
        return Some(ApfelServerHandle { child: None });
    }
    if !crate::summarization::backend_profile::can_spawn(backend) {
        return None;
    }
    let port = port_from_base_url(base_url)?;
    ApfelServerHandle::spawn(base_url, port)
}

/// Send a single chat-completions request to the server at `base_url` and
/// return the assistant `choices[0].message.content` string.
///
/// The `model` field of the request body is backend-aware (issue #776):
/// apfel sends `apple-foundationmodel`, llama-server sends the configured
/// name or a neutral default, and generic sends the configured name or omits
/// the field entirely. A non-apfel server rejecting an unknown model with a
/// 400 would short-circuit the 429 retry and the demux entirely, so the
/// field must match the backend.
///
/// HTTP 429 (server at max concurrent capacity) is retryable backpressure, not
/// a hard failure: it is retried `HTTP_429_RETRY` times with a short backoff.
/// Other non-2xx responses and transport errors are surfaced as
/// `SummarizationFailed`, which the pipeline classifies and degrades on.
pub fn send_chat_completions(
    base_url: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    send_chat_completions_for(
        base_url,
        SummarizerBackend::Apfel,
        None,
        system_prompt,
        user_prompt,
    )
}

/// The host:port of a base URL, embedded in error messages so a failure is
/// attributable to the right service (and the backend-named message stays
/// precise when multiple backends share the default port).
fn failure_label(base_url: &str) -> String {
    host_label(base_url)
}

/// Backend-aware chat-completions request. `model_name` is the user-configured
/// model (if any); the effective `model` field is resolved per backend by
/// `backend_profile::request_model`.
pub fn send_chat_completions_for(
    base_url: &str,
    backend: SummarizerBackend,
    model_name: Option<&str>,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let name = crate::summarization::backend_profile::display_name(backend);
    let label = failure_label(base_url);
    let mut body = serde_json::json!({
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ]
    });
    if let Some(model) = crate::summarization::backend_profile::request_model(backend, model_name) {
        body["model"] = serde_json::Value::String(model);
    }

    let mut attempts = 0u32;
    loop {
        match post_once(base_url, name, &label, &body, summarizer_token().as_deref())? {
            ChatPostOutcome::Content(content) => return Ok(content),
            ChatPostOutcome::TooManyRequests => {
                attempts += 1;
                if attempts > HTTP_429_RETRY {
                    return Err(crate::error::LievoError::SummarizationFailed(format!(
                        "{name} server at {label} at max concurrent capacity after {HTTP_429_RETRY} retries"
                    )));
                }
                thread::sleep(Duration::from_millis(HTTP_429_BACKOFF_MS));
            }
        }
    }
}

enum ChatPostOutcome {
    Content(String),
    TooManyRequests,
}

fn post_once(
    base_url: &str,
    name: &str,
    label: &str,
    body: &serde_json::Value,
    token: Option<&str>,
) -> Result<ChatPostOutcome> {
    let mut request = ureq::post(format!("{base_url}/v1/chat/completions"))
        .config()
        .timeout_global(Some(Duration::from_secs(SERVER_REQUEST_TIMEOUT_SECS)))
        .build();
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    let response = match request.send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) if code == HTTP_429.as_u16() => {
            return Ok(ChatPostOutcome::TooManyRequests);
        }
        Err(ureq::Error::StatusCode(code)) => {
            return Err(crate::error::LievoError::SummarizationFailed(format!(
                "{name} server at {label} failed with HTTP {code}"
            )));
        }
        Err(e) => {
            return Err(crate::error::LievoError::SummarizationFailed(format!(
                "{name} server at {label} request failed: {e}"
            )));
        }
    };

    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| crate::error::LievoError::SummarizationFailed(e.to_string()))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        crate::error::LievoError::SummarizationFailed(format!("failed to parse chat response: {e}"))
    })?;

    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            crate::error::LievoError::SummarizationFailed(
                "chat response missing choices[0].message.content".to_string(),
            )
        })?;

    Ok(ChatPostOutcome::Content(content.trim().to_string()))
}

#[cfg(test)]
#[path = "summarizer_backend_test_helpers.rs"]
mod test_helpers;
#[cfg(test)]
pub use test_helpers::*;
