// Backend profiles (issue #776) — per-backend behavior for the named
// summarizer backend (`apfel` | `llama-server` | `generic`).
//
// #772 delivered a generic HTTP transport but left health validation and
// spawn apfel-shaped: `/health` required apfel's four-field body and the
// spawn path hardcoded the `apfel` binary, so a llama-server or generic
// OpenAI-compatible server on the configured port was rejected as
// "unavailable" and the chat request hardcoded `"model":
// "apple-foundationmodel"`, which non-apfel servers reject with a 400 that
// short-circuits the 429 retry and the demux entirely.
//
// This module owns what differs per backend:
//
// - The `/health` body validator (each backend's real introspection shape;
//   a mismatch degrades to "unavailable", never a hard error — the
//   Ollama-on-11434 collision guard).
// - Whether lievo may spawn the backend (only `apfel` is spawnable — the
//   others need a model path and flags lievo must not guess).
// - The `model` field of the chat-completions request body: apfel keeps
//   `apple-foundationmodel`; llama-server sends the configured name or a
//   neutral default; generic sends the configured name or OMITS the field
//   rather than guessing.
// - The user-facing name used in messages and error text (never "apfel"
//   when a different backend is configured).
//
// Everything here is pure and synchronous; the HTTP transport itself lives
// in `summarizer_backend.rs`.

#[cfg(test)]
#[path = "backend_profile_tests.rs"]
mod tests;

use crate::config::SummarizerBackend;

/// The endpoint paths lievo probes for liveness, in order (issue #780):
/// apfel and llama-server expose `/health`; a generic OpenAI-compatible
/// server may expose only `/v1/models`. A not-found first probe (a 404/
/// 405 response with a non-JSON body) is not fatal — the next path is tried.
pub fn health_probe_paths(backend: SummarizerBackend) -> &'static [&'static str] {
    match backend {
        SummarizerBackend::Generic => &["/v1/models", "/health"],
        SummarizerBackend::Apfel | SummarizerBackend::LlamaServer => &["/health"],
    }
}

/// The model name apfel's server expects in the chat-completions request.
pub const APFEL_DEFAULT_MODEL: &str = "apple-foundationmodel";

/// llama.cpp's server accepts any model string; this neutral default is sent
/// when no model name is configured, so a server that validates the field
/// does not reject the request.
pub const LLAMA_DEFAULT_MODEL: &str = "default";

/// The `model` field of the chat-completions request body for the backend,
/// or `None` to omit the field entirely (generic backend with no configured
/// model: the server serves whatever model it was started with, so lievo
/// must not guess a name).
pub fn request_model(backend: SummarizerBackend, configured: Option<&str>) -> Option<String> {
    let configured = configured.map(str::trim).filter(|s| !s.is_empty());
    match backend {
        // Apfel's server is built on Apple FoundationModels and expects the
        // platform model name regardless of user configuration.
        SummarizerBackend::Apfel => Some(APFEL_DEFAULT_MODEL.to_string()),
        SummarizerBackend::LlamaServer => configured
            .map(|s| s.to_string())
            .or_else(|| Some(LLAMA_DEFAULT_MODEL.to_string())),
        // Omit the field entirely when unset rather than sending a guessed
        // name a vLLM/MLX server would reject.
        SummarizerBackend::Generic => configured.map(|s| s.to_string()),
    }
}

/// Validate a health-probe response body for the backend.
///
/// Per-backend real introspection shapes:
/// - `apfel` — the four-field shape (#772): `prewarmed` (bool),
///   `active_requests` (number), `context_window` (number),
///   `model_available` (bool).
/// - `llama-server` — llama.cpp's documented shape: HTTP 2xx AND
///   `{"status":"ok"}`. Model-loading is 503, so the body check is what
///   rejects an Ollama collision answering 200 with a different body.
/// - `generic` — accepted probe forms (issue #780):
///   1. the standard OpenAI-compatible `/v1/models` list shape: `object` ==
///      `"list"` with a `data` array. `data` MAY be empty (a server that
///      reports no loaded models is still a live, correctly-shaped endpoint —
///      the decision issue #780 asked the implementer to lock with a named
///      test): an empty `data` array validates, so a freshly started hosted
///      endpoint is not rejected merely for not having loaded a model yet.
///      A non-empty `data` entry must be a JSON object; an entry that is not
///      an object is not the standard list shape and degrades to unavailable
///      (same as any wrong-shape body).
///   2. a `/health`-style liveness body: a top-level `status` field of
///      string or boolean type — retained as an additional accepted form for
///      generic servers that DO offer `/health` (it is no longer the only
///      form, per the issue's operator decision).
///
/// Both are pure liveness checks: a wrong-shaped 200 maps to "unavailable"
/// (degrade), never a protocol error.
pub fn health_body_is_valid(backend: SummarizerBackend, body: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    match backend {
        SummarizerBackend::Apfel => summarizer_backend_apfel_shape(object),
        SummarizerBackend::LlamaServer => object
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == "ok"),
        SummarizerBackend::Generic => {
            openai_models_list_shape(object)
                || matches!(
                    object.get("status"),
                    Some(serde_json::Value::String(_)) | Some(serde_json::Value::Bool(_))
                )
        }
    }
}

/// The standard OpenAI-compatible model-list shape (`/v1/models`):
/// `{"object":"list","data":[<object>, ...]}`. `data` may be empty (decision
/// in issue #780: an empty list is a live endpoint, not a wrong shape);
/// non-empty entries must be JSON objects.
fn openai_models_list_shape(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.get("object").and_then(|v| v.as_str()) == Some("list")
        && object
            .get("data")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().all(|item| item.is_object()))
            .unwrap_or(false)
}

/// Apfel's four-field marker shape (moved verbatim from
/// `summarizer_backend::health_body_is_apfel`, whose public shape and
/// behavior are unchanged — it now delegates here).
fn summarizer_backend_apfel_shape(object: &serde_json::Map<String, serde_json::Value>) -> bool {
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

/// Whether lievo may spawn the backend itself.
///
/// Only `apfel` is spawnable (it needs no user-supplied arguments:
/// `apfel --serve --port <port>`). llama-server needs a model path and
/// flags, and a generic server needs its own launch configuration — lievo
/// must not guess them. Non-spawnable backends are used only when already
/// running, with a clear message when they are not.
pub fn can_spawn(backend: SummarizerBackend) -> bool {
    matches!(backend, SummarizerBackend::Apfel)
}

/// The user-facing name of the backend, for messages and error text
/// (issue #776: the unavailable message must name the configured backend,
/// not apfel).
pub fn display_name(backend: SummarizerBackend) -> &'static str {
    backend.as_str()
}

// ---------------------------------------------------------------------------
// Per-backend summarizer input budget (issue #792)
// ---------------------------------------------------------------------------
//
// The input budget is a property of the backend actually receiving the
// prompt, not a single global constant sized for apfel. apfel (Apple
// FoundationModels) is empirically safe at 8,000 chars; llama-server and
// generic (any OpenAI-compatible server, e.g. Qwen3-4B at a 32,768-token
// window per our own docs) can hold 24,000 chars (~6,000 tokens), which sits
// inside the 32k window we document and still works on an 8k-context server.
// 24,000 was deliberately chosen over 48,000 for small-context safety (48k
// would exceed any window below 16k, including Ollama's 4,096 default).

/// apfel's budget: unchanged at 8,000 chars (a real model constraint, not
/// overridable — raising it only produces context-overflow failures further
/// down the path).
pub const APFEL_INPUT_CHAR_BUDGET: usize = 8000;

/// llama-server and generic default budget: 24,000 chars (~6,000 tokens).
pub const HTTP_BACKEND_INPUT_CHAR_BUDGET: usize = 24000;

/// The lower bound the optional override is clamped to for non-apfel
/// backends. 2,000 chars still permits a useful batch while keeping the
/// value sane.
pub const INPUT_CHAR_BUDGET_MIN: usize = 2000;

/// The upper bound the optional override is clamped to for non-apfel
/// backends. 64,000 chars (~16,000 tokens) is large but does not invite a
/// guaranteed server-side rejection on a 32k-window server.
pub const INPUT_CHAR_BUDGET_MAX: usize = 64000;

/// Resolve the summarizer input budget (in chars) for the backend actually
/// in use, honouring an optional user override for non-apfel backends
/// (issue #792).
///
/// This is the SINGLE source of truth for the budget. Every consumer — the
/// three defensive guards in `summarize_code` / `batch_summarize` /
/// `rollup_batch_summarize`, the packer `pack_by_char_budget`, and the
/// per-function retry guard in `batch_ops.rs` — must call this rather than
/// hardcode a constant, so they never compute two different numbers.
///
/// # Keys off the transport in use, not the configured name
///
/// The budget is a property of the thing receiving the prompt. If
/// `summarizer_backend: llama-server` is configured but no endpoint is set,
/// the run falls back to the apfel CLI — and in that state the budget is
/// apfel's 8,000, because apfel is what is executing. Callers resolve the
/// backend from the transport (`Option<&BackendTransport>` passed down the
/// call path, issue #783); a `None`
/// transport means the CLI fallback, which is apfel, so they pass
/// `SummarizerBackend::Apfel` here.
///
/// # Override semantics
///
/// `configured_override` is the optional raw override (chars) from config.
/// It applies ONLY to `llama-server` and `generic`; apfel is exempt and
/// always returns 8,000. An out-of-range value is clamped into
/// [`INPUT_CHAR_BUDGET_MIN`]..=[`INPUT_CHAR_BUDGET_MAX`] rather than passed
/// through to the server, so an absurd config cannot silently produce
/// failing requests.
pub fn input_char_budget(backend: SummarizerBackend, configured_override: Option<usize>) -> usize {
    match backend {
        // apfel's 8,000 reflects a real model constraint — never overridable.
        SummarizerBackend::Apfel => APFEL_INPUT_CHAR_BUDGET,
        SummarizerBackend::LlamaServer | SummarizerBackend::Generic => configured_override
            .map(|v| v.clamp(INPUT_CHAR_BUDGET_MIN, INPUT_CHAR_BUDGET_MAX))
            .unwrap_or(HTTP_BACKEND_INPUT_CHAR_BUDGET),
    }
}
