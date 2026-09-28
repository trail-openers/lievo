// Issue #788: enabled-but-unconfigured and retry-bound tests.

use crate::config::RepoConfig;
use crate::summarization::pipeline::{SummarizeDecision, summarize_decision};
use crate::summarization::unconfigured::{is_enabled_but_unconfigured, unconfigured_message};

// The enabled-but-unconfigured state: `summarize: true`, no backend, no apfel
// binary. The gate enables (correct — explicit intent is honoured), but there
// is no endpoint to call and no apfel binary to fall back to.
#[test]
fn test_is_enabled_but_unconfigured_summarize_true_no_backend() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    assert!(
        is_enabled_but_unconfigured(false, &repo_config, false),
        "summarize: true + no backend + no apfel must be unconfigured"
    );
}

// Enabled with a backend and endpoint: not unconfigured.
#[test]
fn test_is_enabled_but_unconfigured_with_endpoint() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        apfel_endpoint: Some("http://127.0.0.1:8080".to_string()),
        ..Default::default()
    };
    assert!(
        !is_enabled_but_unconfigured(false, &repo_config, false),
        "enabled + endpoint must NOT be unconfigured"
    );
}

// Disabled by config: not unconfigured (the gate is off, not on).
#[test]
fn test_is_enabled_but_unconfigured_disabled_by_config() {
    let repo_config = RepoConfig {
        summarize: Some(false),
        ..Default::default()
    };
    assert!(
        !is_enabled_but_unconfigured(false, &repo_config, false),
        "summarize: false must NOT be unconfigured"
    );
}

// Disabled by CLI flag: not unconfigured.
#[test]
fn test_is_enabled_but_unconfigured_disabled_by_cli_flag() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    assert!(
        !is_enabled_but_unconfigured(true, &repo_config, false),
        "--no-summarize must NOT be unconfigured"
    );
}

// Unset summarize + apfel default + apfel available: enabled, not unconfigured
// (apfel binary IS available, so the CLI path works).
#[test]
fn test_is_enabled_but_unconfigured_apfel_available() {
    let repo_config = RepoConfig::default();
    assert!(
        !is_enabled_but_unconfigured(false, &repo_config, true),
        "unset + apfel available must NOT be unconfigured"
    );
}

// Unset summarize + apfel default + no apfel: disabled, not unconfigured.
#[test]
fn test_is_enabled_but_unconfigured_apfel_unavailable() {
    let repo_config = RepoConfig::default();
    assert!(
        !is_enabled_but_unconfigured(false, &repo_config, false),
        "unset + no apfel must NOT be unconfigured (gate is off)"
    );
}

// The unconfigured message names the backend and the real cause.
#[test]
fn test_unconfigured_message_names_backend() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    let msg = unconfigured_message(&repo_config);
    assert!(
        msg.contains("apfel"),
        "message must name the backend: {msg:?}"
    );
    assert!(
        msg.contains("no summaries will be produced"),
        "message must name the real cause: {msg:?}"
    );
}

// The #786 guard tests must stay green and unmodified. These two are
// re-asserted here to pin the contract:
#[test]
fn test_guard_summarize_true_enables_for_every_backend() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    assert!(
        summarize_decision(false, &repo_config, false) == SummarizeDecision::Enabled,
        "summarize: true must enable for every backend"
    );
}

#[test]
fn test_guard_non_apfel_backend_enabled_without_apfel() {
    let repo_config = RepoConfig {
        summarizer_backend: Some("llama-server".to_string()),
        apfel_endpoint: Some("http://localhost:8080".to_string()),
        ..Default::default()
    };
    assert!(
        summarize_decision(false, &repo_config, false) == SummarizeDecision::Enabled,
        "non-apfel backend + endpoint must enable even without apfel"
    );
}

// ── Issue #788 MEDIUM 2: the shared classifier is the one decision point ──

use crate::summarization::enabled_state::{EnabledState, classify_enabled_state};

// summarize: true, no endpoint, no apfel: the classifier says enabled-but-
// unconfigured (the state the marker tracks).
#[test]
fn test_classify_enabled_but_unconfigured() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    assert_eq!(
        classify_enabled_state(false, &repo_config, false),
        EnabledState::EnabledButUnconfigured,
    );
}

// summarize: true with a usable endpoint: Enabled.
#[test]
fn test_classify_enabled_with_endpoint() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        apfel_endpoint: Some("http://127.0.0.1:8080".to_string()),
        ..Default::default()
    };
    assert_eq!(
        classify_enabled_state(false, &repo_config, false),
        EnabledState::Enabled,
    );
}

// summarize: false: Disabled.
#[test]
fn test_classify_disabled_by_config() {
    let repo_config = RepoConfig {
        summarize: Some(false),
        ..Default::default()
    };
    assert_eq!(
        classify_enabled_state(false, &repo_config, false),
        EnabledState::Disabled,
    );
}

// --no-summarize: Disabled, even with summarize: true.
#[test]
fn test_classify_disabled_by_cli_flag() {
    let repo_config = RepoConfig {
        summarize: Some(true),
        ..Default::default()
    };
    assert_eq!(
        classify_enabled_state(true, &repo_config, false),
        EnabledState::Disabled,
    );
}

// Unset summarize, apfel default, apfel available: Enabled.
#[test]
fn test_classify_apfel_default_available() {
    let repo_config = RepoConfig::default();
    assert_eq!(
        classify_enabled_state(false, &repo_config, true),
        EnabledState::Enabled,
    );
}

// Unset summarize, apfel default, no apfel: Disabled (gate is off).
#[test]
fn test_classify_apfel_default_unavailable() {
    let repo_config = RepoConfig::default();
    assert_eq!(
        classify_enabled_state(false, &repo_config, false),
        EnabledState::Disabled,
    );
}
