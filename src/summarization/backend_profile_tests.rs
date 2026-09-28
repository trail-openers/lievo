// Unit tests for the per-backend `/health` body validator (issue #780) —
// the generic backend must accept the standard OpenAI-compatible `/v1/models`
// list shape in addition to a `/health` body carrying `status`.
//
// Sibling file included via `#[path]` from `backend_profile.rs` (the
// established pattern; keeps `apfel_server_tests.rs` under its grandfathered
// line ceiling). Uses `crate::` imports only (super-probe budget: 8).

use crate::config::SummarizerBackend;
use crate::summarization::backend_profile::{
    APFEL_INPUT_CHAR_BUDGET, HTTP_BACKEND_INPUT_CHAR_BUDGET, INPUT_CHAR_BUDGET_MAX,
    INPUT_CHAR_BUDGET_MIN, health_body_is_valid, input_char_budget,
};

/// Standard OpenAI-compatible `/v1/models` response with a non-empty `data`
/// list — the shape hosted servers actually expose (issue #780: the generic
/// backend's probe must accept it).
#[test]
fn test_health_body_generic_accepts_openai_models_list() {
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"list","data":[{"id":"qwen3","object":"model","owned_by":"vendor"}]}"#
    ));
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"list","data":[{"id":"m1"},{"id":"m2"}]}"#
    ));
}

/// Operator decision locked by a named test (issue #780 edge case): some
/// OpenAI-compatible servers answer 200 with an EMPTY `data` list. That is
/// treated as HEALTHY — a server that reports no loaded models is still a
/// live, correctly-shaped endpoint, and rejecting it would send lievo down
/// the degradation path for a correctly configured hosted endpoint.
#[test]
fn test_health_body_generic_empty_data_list_is_healthy() {
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"list","data":[]}"#
    ));
}

/// A `/health`-style body with a `status` field of string or bool remains an
/// additional accepted form for generic servers that offer it (retained from
/// #776, no longer the only form).
#[test]
fn test_health_body_generic_retains_status_liveness_form() {
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":"ok"}"#
    ));
    assert!(health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":true}"#
    ));
}

/// The accepted-form union must not widen: wrong-shaped 200s still degrade to
/// "unavailable" (the Ollama collision guard stays intact).
#[test]
fn test_health_body_generic_rejects_wrong_shapes() {
    // `object` not "list" → not the models-list shape; no `status` either.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"model","id":"m1"}"#
    ));
    // A `data` entry that is not a JSON object.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"list","data":["not-an-object"]}"#
    ));
    // `data` present but not an array.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"object":"list","data":{}}"#
    ));
    // A bare model-map body (no `object`, no `status`).
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"models":[{"name":"llama3"}]}"#
    ));
    // Wrong-typed `status`.
    assert!(!health_body_is_valid(
        SummarizerBackend::Generic,
        r#"{"status":42}"#
    ));
}

/// The models-list form is GENERIC-only: apfel (four-field marker shape) and
/// llama-server (`status:"ok"`) must not start accepting it.
#[test]
fn test_health_body_models_list_not_accepted_by_other_backends() {
    let models_list =
        r#"{"object":"list","data":[{"id":"qwen3","object":"model","owned_by":"vendor"}]}"#;
    assert!(!health_body_is_valid(SummarizerBackend::Apfel, models_list));
    assert!(!health_body_is_valid(
        SummarizerBackend::LlamaServer,
        models_list
    ));
}

// ---------------------------------------------------------------------------
// Per-backend summarizer input budget (issue #792)
// ---------------------------------------------------------------------------

/// apfel's effective budget is pinned at 8,000 chars — unchanged, and the
/// value that must never drift. Pinned by name so a silent regression fails
/// loudly.
#[test]
fn test_budget_apfel_pinned_at_8000() {
    assert_eq!(APFEL_INPUT_CHAR_BUDGET, 8000);
    assert_eq!(input_char_budget(SummarizerBackend::Apfel, None), 8000);
}

/// llama-server gets the larger 24,000-char budget by default. Pinned so the
/// choice cannot drift silently (issue #792: a bigger default than apfel's).
#[test]
fn test_budget_llama_server_pinned_at_24000() {
    assert_eq!(HTTP_BACKEND_INPUT_CHAR_BUDGET, 24000);
    assert_eq!(
        input_char_budget(SummarizerBackend::LlamaServer, None),
        24000
    );
}

/// generic gets the larger 24,000-char budget by default. Pinned for the
/// same drift reason as llama-server.
#[test]
fn test_budget_generic_pinned_at_24000() {
    assert_eq!(input_char_budget(SummarizerBackend::Generic, None), 24000);
}

/// The between-budgets behaviour the issue exists for: an input that exceeds
/// apfel's 8,000 but fits the 24,000 non-apfel budget resolves differently
/// per backend. This locks the core fix — the same input is in-budget for
/// llama-server/generic and out-of-budget for apfel.
#[test]
fn test_budget_between_apfel_and_http_backends() {
    let between = 12_000usize; // 8000 < 12000 < 24000
    assert!(between > input_char_budget(SummarizerBackend::Apfel, None));
    assert!(between <= input_char_budget(SummarizerBackend::LlamaServer, None));
    assert!(between <= input_char_budget(SummarizerBackend::Generic, None));
}

/// A valid in-range override is honoured for non-apfel backends.
#[test]
fn test_budget_override_honoured_in_range() {
    assert_eq!(
        input_char_budget(SummarizerBackend::LlamaServer, Some(4000)),
        4000
    );
    assert_eq!(
        input_char_budget(SummarizerBackend::Generic, Some(50000)),
        50000
    );
}

/// An absurdly low override is clamped UP to the lower bound so it cannot
/// silently produce a budget too small for a useful batch.
#[test]
fn test_budget_override_clamped_low() {
    assert_eq!(
        input_char_budget(SummarizerBackend::LlamaServer, Some(0)),
        INPUT_CHAR_BUDGET_MIN
    );
    assert_eq!(
        input_char_budget(SummarizerBackend::Generic, Some(1)),
        INPUT_CHAR_BUDGET_MIN
    );
    assert_eq!(INPUT_CHAR_BUDGET_MIN, 2000);
}

/// An absurdly high override is clamped DOWN to the upper bound so it cannot
/// pass a guaranteed-rejected number through to the server.
#[test]
fn test_budget_override_clamped_high() {
    assert_eq!(
        input_char_budget(SummarizerBackend::LlamaServer, Some(1_000_000)),
        INPUT_CHAR_BUDGET_MAX
    );
    assert_eq!(
        input_char_budget(SummarizerBackend::Generic, Some(10_000_000)),
        INPUT_CHAR_BUDGET_MAX
    );
    assert_eq!(INPUT_CHAR_BUDGET_MAX, 64000);
}

/// A boundary value equal to the clamp bound is returned exactly (clamp is
/// inclusive on both ends).
#[test]
fn test_budget_override_at_clamp_bounds() {
    assert_eq!(
        input_char_budget(SummarizerBackend::LlamaServer, Some(2000)),
        2000
    );
    assert_eq!(
        input_char_budget(SummarizerBackend::Generic, Some(64000)),
        64000
    );
}

/// apfel is EXEMPT from the override: any configured override is ignored and
/// the effective budget stays at 8,000 (its value reflects a real model
/// constraint, so a user raising it would only produce failures).
#[test]
fn test_budget_apfel_exempt_from_override() {
    assert_eq!(
        input_char_budget(SummarizerBackend::Apfel, Some(24000)),
        8000
    );
    assert_eq!(
        input_char_budget(SummarizerBackend::Apfel, Some(4000)),
        8000
    );
    assert_eq!(input_char_budget(SummarizerBackend::Apfel, Some(0)), 8000);
}

/// The transport-in-use rule (issue #792 operator decision #1): when no
/// endpoint is set the run falls back to the apfel CLI, so the effective
/// budget is apfel's 8,000 even though the *configured* backend name might be
/// llama-server. Callers resolve the backend from the transport; a `None`
/// transport means the CLI fallback = apfel. This pins that the resolver
/// returns 8,000 for apfel so the CLI-fallback state is never over-budget.
#[test]
fn test_budget_cli_fallback_is_apfel() {
    // No endpoint -> apfel CLI -> apfel budget.
    assert_eq!(input_char_budget(SummarizerBackend::Apfel, None), 8000);
}
