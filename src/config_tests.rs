// Tests for the `.lievo/config.yaml` parser (issue #776).
//
// Covers the `summarizer_backend` named-backend config option (apfel,
// llama-server, generic) alongside the pre-existing load/validate tests
// that moved here when `config.rs` hit the 500-line source budget.

use crate::config::{RepoConfig, SummarizerBackend};
use crate::error::LievoError;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_config(dir: &Path, yaml: &str) {
    let lievo_dir = dir.join(".lievo");
    fs::create_dir_all(&lievo_dir).unwrap();
    fs::write(lievo_dir.join("config.yaml"), yaml).unwrap();
}

// --- load() ---
// Re-verification under the serde_yaml_ng swap (issue #829): the
// empty/whitespace/comment-only workaround in `RepoConfig::load`
// (src/config.rs, `has_effective_content` branch) is kept verbatim —
// serde_yaml_ng, like the old parser, rejects an empty document, so
// those files must still parse as all-defaults and malformed YAML
// must still surface `InvalidConfig` with a non-empty reason.

#[test]
fn test_load_returns_none_when_config_absent() {
    let tmp = TempDir::new().unwrap();
    let result = RepoConfig::load(tmp.path()).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_load_valid_full_config() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        r#"
subsystems:
  - name: core
    paths: [src/core]
  - name: cli
    paths: [src/bin]
exclude:
  - "**/*.generated.rs"
  - vendor/
module_depth: 2
languages:
  - Rust
dependency_map:
  "a:b": "c:d"
"#,
    );

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.subsystems.len(), 2);
    assert_eq!(config.subsystems[0].name, "core");
    assert_eq!(config.subsystems[0].paths, vec!["src/core"]);
    assert_eq!(config.exclude, vec!["**/*.generated.rs", "vendor/"]);
    assert_eq!(config.module_depth, Some(2));
    assert_eq!(config.languages, vec!["Rust"]);
    assert_eq!(
        config.dependency_map.get("a:b").map(String::as_str),
        Some("c:d")
    );
}

#[test]
fn test_load_partial_config_applies_defaults() {
    let tmp = TempDir::new().unwrap();
    // Only module_depth is set; all other fields should use defaults.
    write_config(tmp.path(), "module_depth: 3\n");

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, Some(3));
    assert!(config.subsystems.is_empty());
    assert!(config.exclude.is_empty());
    assert!(config.languages.is_empty());
    assert!(config.dependency_map.is_empty());
}

#[test]
fn test_load_empty_file_applies_all_defaults() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "");

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, None);
    assert!(config.subsystems.is_empty());
    assert!(config.preserve_function_entities);
}

#[test]
fn test_load_comment_only_file_applies_all_defaults() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "# repository config\n# everything below is commented out\n",
    );

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, None);
    assert!(config.subsystems.is_empty());
    assert!(config.preserve_function_entities);
}

#[test]
fn test_load_comment_plus_real_key_parses_key() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "# set the module depth\nmodule_depth: 4\n");

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, Some(4));
}

#[test]
fn test_load_whitespace_only_file_applies_all_defaults() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "   \n\n  \t\n");

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, None);
    assert!(config.subsystems.is_empty());
    assert!(config.preserve_function_entities);
}

#[test]
fn test_load_invalid_yaml_returns_err_with_path() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "module_depth: [not_a_number");

    let err = RepoConfig::load(tmp.path()).unwrap_err();
    match err {
        LievoError::InvalidConfig { path, reason } => {
            assert!(
                path.contains(".lievo"),
                "path should mention .lievo: {path}"
            );
            assert!(!reason.is_empty(), "reason should not be empty");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_load_invalid_yaml_malformed_flow_sequence_reason_names_cause() {
    // Issue #829 re-verification: the malformed `module_depth:` flow
    // sequence above must yield a non-empty reason that names the cause
    // (not a generic/empty message) under serde_yaml_ng.
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "module_depth: [not_a_number");

    let err = RepoConfig::load(tmp.path()).unwrap_err();
    match err {
        LievoError::InvalidConfig { path, reason } => {
            assert!(
                path.contains(".lievo"),
                "path should mention .lievo: {path}"
            );
            assert!(
                reason.contains("module_depth") || reason.contains("not_a_number"),
                "reason should name the offending key or value: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_load_comment_with_indented_blank_lines_applies_all_defaults() {
    // Issue #829 re-verification: comments plus indented blank lines still
    // have no effective content and must parse as all-defaults.
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "# top\n\n  \n# bottom\n   \t  \n");

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, None);
    assert!(config.subsystems.is_empty());
    assert!(config.preserve_function_entities);
}

#[test]
fn test_load_unknown_fields_are_ignored() {
    let tmp = TempDir::new().unwrap();
    // Forward-compatibility: unknown keys must not cause an error.
    write_config(
        tmp.path(),
        "module_depth: 2\nunknown_future_key: some_value\n",
    );

    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.module_depth, Some(2));
}

// --- validate() ---

#[test]
fn test_validate_default_config_is_valid() {
    assert!(RepoConfig::default().validate().is_ok());
}

#[test]
fn test_validate_rejects_zero_module_depth() {
    let config = RepoConfig {
        module_depth: Some(0),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(reason.contains("module_depth"), "got: {reason}");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_rejects_subsystem_with_empty_paths() {
    let config = RepoConfig {
        subsystems: vec![SubsystemOverride {
            name: "empty".to_string(),
            paths: vec![],
        }],
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(reason.contains("empty"), "got: {reason}");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_accepts_valid_subsystems() {
    let config = RepoConfig {
        subsystems: vec![SubsystemOverride {
            name: "core".to_string(),
            paths: vec!["src/core".to_string()],
        }],
        module_depth: Some(2),
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}

#[test]
fn test_validate_rejects_subsystem_with_empty_name() {
    let config = RepoConfig {
        subsystems: vec![SubsystemOverride {
            name: "".to_string(),
            paths: vec!["src/".to_string()],
        }],
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(
                reason.contains("name"),
                "error should mention name: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_rejects_subsystem_with_whitespace_only_name() {
    let config = RepoConfig {
        subsystems: vec![SubsystemOverride {
            name: "   ".to_string(),
            paths: vec!["src/".to_string()],
        }],
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(
                reason.contains("name"),
                "error should mention name: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_default_preserve_function_entities_is_true() {
    assert!(RepoConfig::default().preserve_function_entities);
}

#[test]
fn test_load_explicit_false_preserve_function_entities() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "preserve_function_entities: false\n");
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert!(
        !config.preserve_function_entities,
        "explicit false must override default true"
    );
}

// --- apfel_endpoint ---

#[test]
fn test_apfel_endpoint_defaults_to_none() {
    assert!(RepoConfig::default().apfel_endpoint.is_none());
}

#[test]
fn test_load_apfel_endpoint_valid_url() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "apfel_endpoint: http://127.0.0.1:11434\n");
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(
        config.apfel_endpoint.as_deref(),
        Some("http://127.0.0.1:11434")
    );
}

#[test]
fn test_validate_rejects_empty_apfel_endpoint() {
    let config = RepoConfig {
        apfel_endpoint: Some(String::new()),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(reason.contains("apfel_endpoint"), "got: {reason}");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_rejects_apfel_endpoint_without_scheme() {
    let config = RepoConfig {
        apfel_endpoint: Some("127.0.0.1:11434".to_string()),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(reason.contains("http"), "got: {reason}");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_accepts_https_apfel_endpoint() {
    let config = RepoConfig {
        apfel_endpoint: Some("https://apfel.example:8443".to_string()),
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}

// --- SubsystemOverride import for tests above ---
use crate::config::SubsystemOverride;

// --- summarizer_backend ---

#[test]
fn test_summarizer_backend_defaults_to_none_apfel_effective() {
    assert!(RepoConfig::default().summarizer_backend.is_none());
    assert_eq!(
        RepoConfig::default().effective_summarizer_backend(),
        SummarizerBackend::Apfel
    );
}

#[test]
fn test_load_summarizer_backend_apfel() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "apfel_endpoint: http://127.0.0.1:11434\nsummarizer_backend: apfel\n",
    );
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(
        config.effective_summarizer_backend(),
        SummarizerBackend::Apfel
    );
}

#[test]
fn test_load_summarizer_backend_llama_server() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "apfel_endpoint: http://127.0.0.1:8080\nsummarizer_backend: llama-server\n",
    );
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(
        config.effective_summarizer_backend(),
        SummarizerBackend::LlamaServer
    );
}

#[test]
fn test_load_summarizer_backend_generic() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "apfel_endpoint: http://127.0.0.1:8005\nsummarizer_backend: generic\n",
    );
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(
        config.effective_summarizer_backend(),
        SummarizerBackend::Generic
    );
}

#[test]
fn test_load_summarizer_backend_omitted_falls_back_to_apfel() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "apfel_endpoint: http://127.0.0.1:11434\n");
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert!(config.summarizer_backend.is_none());
    assert_eq!(
        config.effective_summarizer_backend(),
        SummarizerBackend::Apfel
    );
}

#[test]
fn test_summarizer_backend_parse_apfel() {
    assert_eq!(
        SummarizerBackend::parse("apfel"),
        Some(SummarizerBackend::Apfel)
    );
}

#[test]
fn test_summarizer_backend_parse_llama_server() {
    assert_eq!(
        SummarizerBackend::parse("llama-server"),
        Some(SummarizerBackend::LlamaServer)
    );
}

#[test]
fn test_summarizer_backend_parse_generic() {
    assert_eq!(
        SummarizerBackend::parse("generic"),
        Some(SummarizerBackend::Generic)
    );
}

#[test]
fn test_summarizer_backend_parse_is_case_insensitive_and_trims() {
    assert_eq!(
        SummarizerBackend::parse("  Apfel "),
        Some(SummarizerBackend::Apfel)
    );
    assert_eq!(
        SummarizerBackend::parse("LLAMA-SERVER"),
        Some(SummarizerBackend::LlamaServer)
    );
    assert_eq!(
        SummarizerBackend::parse("Generic"),
        Some(SummarizerBackend::Generic)
    );
}

#[test]
fn test_summarizer_backend_parse_rejects_unknown_and_empty() {
    assert_eq!(SummarizerBackend::parse("vllm-mlx"), None);
    assert_eq!(SummarizerBackend::parse("ollama"), None);
    assert_eq!(SummarizerBackend::parse("llama"), None);
    assert_eq!(SummarizerBackend::parse(""), None);
    assert_eq!(SummarizerBackend::parse("  "), None);
}

#[test]
fn test_summarizer_backend_as_str_roundtrips_supported_names() {
    for name in SummarizerBackend::supported_names() {
        let backend = SummarizerBackend::parse(name).unwrap();
        assert_eq!(backend.as_str(), *name);
    }
}

#[test]
fn test_summarizer_backend_supported_names_are_the_three_backends() {
    let names: Vec<&str> = SummarizerBackend::supported_names().to_vec();
    assert_eq!(names, vec!["apfel", "llama-server", "generic"]);
}

#[test]
fn test_validate_rejects_unknown_summarizer_backend() {
    let config = RepoConfig {
        summarizer_backend: Some("vllm-mlx".to_string()),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(
                reason.contains("summarizer_backend"),
                "error should mention the field: {reason}"
            );
            assert!(
                reason.contains("vllm-mlx"),
                "error should show the offending value: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_rejects_empty_summarizer_backend() {
    let config = RepoConfig {
        summarizer_backend: Some("".to_string()),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(reason.contains("summarizer_backend"), "got: {reason}");
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

#[test]
fn test_validate_accepts_all_supported_summarizer_backends() {
    for name in SummarizerBackend::supported_names() {
        let config = RepoConfig {
            summarizer_backend: Some(name.to_string()),
            ..Default::default()
        };
        assert!(
            config.validate().is_ok(),
            "'{name}' must be a valid summarizer_backend"
        );
    }
}

#[test]
fn test_load_rejects_unknown_summarizer_backend_with_config_path() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "summarizer_backend: mlx-faster\n");
    let err = RepoConfig::load(tmp.path()).unwrap_err();
    match err {
        LievoError::InvalidConfig { path, reason } => {
            assert!(
                path.contains(".lievo"),
                "path should mention .lievo: {path}"
            );
            assert!(
                reason.contains("mlx-faster"),
                "reason should show the offending value: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

// --- summarizer_input_char_budget ---

#[test]
fn test_input_char_budget_defaults_to_none() {
    assert!(RepoConfig::default().summarizer_input_char_budget.is_none());
}

#[test]
fn test_load_input_char_budget_persists_value() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "apfel_endpoint: http://127.0.0.1:8080\nsummarizer_backend: llama-server\nsummarizer_input_char_budget: 40000\n",
    );
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.summarizer_input_char_budget, Some(40000));
}

#[test]
fn test_load_input_char_budget_omitted_falls_back_to_none() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "apfel_endpoint: http://127.0.0.1:8080\nsummarizer_backend: generic\n",
    );
    let config = RepoConfig::load(tmp.path()).unwrap().unwrap();
    assert_eq!(config.summarizer_input_char_budget, None);
}

#[test]
fn test_validate_rejects_zero_input_char_budget() {
    let config = RepoConfig {
        summarizer_input_char_budget: Some(0),
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    match err {
        LievoError::InvalidConfig { reason, .. } => {
            assert!(
                reason.contains("summarizer_input_char_budget"),
                "error should mention the field: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    };
}

#[test]
fn test_validate_accepts_in_range_input_char_budget() {
    // The resolver clamps [2000, 64000] itself; the config layer only
    // rejects 0. Values inside (and even outside) the clamp range parse.
    for value in [1, 2000, 24000, 64000, 1_000_000] {
        let config = RepoConfig {
            summarizer_input_char_budget: Some(value),
            ..Default::default()
        };
        assert!(
            config.validate().is_ok(),
            "'{value}' must be a valid summarizer_input_char_budget"
        );
    }
}

#[test]
fn test_load_rejects_zero_input_char_budget_with_config_path() {
    let tmp = TempDir::new().unwrap();
    write_config(tmp.path(), "summarizer_input_char_budget: 0\n");
    let err = RepoConfig::load(tmp.path()).unwrap_err();
    match err {
        LievoError::InvalidConfig { path, reason } => {
            assert!(
                path.contains(".lievo"),
                "path should mention .lievo: {path}"
            );
            assert!(
                reason.contains("summarizer_input_char_budget"),
                "reason should mention the field: {reason}"
            );
        }
        other => panic!("expected InvalidConfig, got {other:?}"),
    };
}

// --- load_or_default() (issue #788) ---

#[test]
fn test_load_or_default_returns_none_config_when_absent() {
    let tmp = TempDir::new().unwrap();
    let config = RepoConfig::load_or_default(tmp.path());
    assert!(config.summarize.is_none());
    assert!(config.apfel_endpoint.is_none());
}

#[test]
fn test_load_or_default_returns_valid_config() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "summarize: true\napfel_endpoint: http://127.0.0.1:8080\n",
    );
    let config = RepoConfig::load_or_default(tmp.path());
    assert_eq!(config.summarize, Some(true));
    assert_eq!(
        config.apfel_endpoint.as_deref(),
        Some("http://127.0.0.1:8080")
    );
}

#[test]
fn test_load_or_default_falls_back_on_invalid_backend() {
    let tmp = TempDir::new().unwrap();
    // Invalid backend name → validate() fails → load_or_default warns and
    // returns defaults (issue #788: no silent backend substitution).
    write_config(
        tmp.path(),
        "summarize: true\nsummarizer_backend: llamaserver\n",
    );
    let config = RepoConfig::load_or_default(tmp.path());
    // Defaults: summarize is None (not Some(true) from the bad file).
    assert!(config.summarize.is_none());
    assert!(config.summarizer_backend.is_none());
}

#[test]
fn test_load_or_default_falls_back_on_corrupt_yaml() {
    let tmp = TempDir::new().unwrap();
    write_config(
        tmp.path(),
        "subsystems:\n  - name: core\n    paths: [src/core]\n  - name: \n",
    );
    // Corrupt YAML → parse error → load_or_default warns and returns defaults.
    let config = RepoConfig::load_or_default(tmp.path());
    assert!(config.subsystems.is_empty());
}
