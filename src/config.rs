// Configuration parser for .lievo/config.yaml
//
// Loads optional per-repository configuration that overrides grouping heuristics.

use crate::error::{LievoError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

fn default_true() -> bool {
    true
}

/// Summary backend name, parsed case-insensitively (issue #776).
///
/// An explicit, named choice — the backend is never inferred by probing, so a
/// misconfiguration is legible in the config file itself. `apfel` keeps the
/// incumbent four-field `/health` contract; `llama-server` and `generic`
/// (any OpenAI-compatible server, e.g. MLX-based) use their own validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummarizerBackend {
    Apfel,
    LlamaServer,
    Generic,
}

impl SummarizerBackend {
    /// All supported names, in canonical form (for error messages).
    pub const fn supported_names() -> &'static [&'static str] {
        &["apfel", "llama-server", "generic"]
    }

    /// Case-insensitive parse; `None` when the name is not a supported backend.
    pub fn parse(name: &str) -> Option<Self> {
        let lowered = name.trim().to_ascii_lowercase();
        let parsed = match lowered.as_str() {
            "apfel" => SummarizerBackend::Apfel,
            "llama-server" | "llama_server" => SummarizerBackend::LlamaServer,
            "generic" => SummarizerBackend::Generic,
            _ => return None,
        };
        Some(parsed)
    }

    /// Canonical (lowercase, kebab-case) name for logs and error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            SummarizerBackend::Apfel => "apfel",
            SummarizerBackend::LlamaServer => "llama-server",
            SummarizerBackend::Generic => "generic",
        }
    }
}

/// Per-repository configuration loaded from `.lievo/config.yaml`.
///
/// All fields are optional and fall back to sensible defaults when absent.
/// Unknown fields are silently ignored for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Explicit subsystem definitions that override auto-detection.
    #[serde(default)]
    pub subsystems: Vec<SubsystemOverride>,

    /// Glob patterns for paths to exclude from analysis.
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Directory levels below the subsystem root used as module boundaries;
    /// `None` infers from the detected project type (e.g. `2` for Rails).
    #[serde(default)]
    pub module_depth: Option<usize>,

    /// Language filters — only analyse these languages when set.
    #[serde(default)]
    pub languages: Vec<String>,

    /// Explicit dependency mapping: entity id → dependency entity id.
    #[serde(default)]
    pub dependency_map: HashMap<String, String>,

    /// Preserve Function entities from code units (default `true`).
    #[serde(default = "default_true")]
    pub preserve_function_entities: bool,

    /// Enable on-device summarization via apfel.
    /// `None` auto-enables if apfel is available; `Some(true)` requires it;
    /// `Some(false)` disables summarization.
    #[serde(default)]
    pub summarize: Option<bool>,

    /// Base URL of the summary backend server (e.g. `http://127.0.0.1:11434`);
    /// `LIEVO_APFEL_ENDPOINT` overrides this. `None`-defaulted: apfel's
    /// default port 11434 collides with Ollama's, so lievo must not assume
    /// anything on that port (issue #772); when unset, one-shot CLI is used.
    #[serde(default)]
    pub apfel_endpoint: Option<String>,

    /// Which summary backend serves `apfel_endpoint`: `apfel` (default),
    /// `llama-server`, or `generic`. Explicit named choice, never inferred
    /// (issue #776); an unknown name is a hard config error.
    #[serde(default)]
    pub summarizer_backend: Option<String>,

    /// The `model` name sent in the chat-completions request body for the
    /// configured backend (issue #776). Ignored by `apfel` (which always
    /// sends `apple-foundationmodel`); for `llama-server` a neutral default
    /// is used when unset, and for `generic` the field is omitted entirely
    /// when unset. `None`-defaulted: lievo never bundles, downloads or pins
    /// a model — the model is whatever the configured backend serves.
    #[serde(default)]
    pub summarizer_model: Option<String>,

    /// Optional override of the per-call summarizer input budget, in
    /// characters (issue #792). Applies ONLY to `llama-server` and
    /// `generic`; `apfel` ignores it (its 8,000 reflects a real model
    /// constraint — raising it only produces context-overflow failures).
    /// The resolver clamps the value into the sane range (see
    /// `summarization::backend_profile::input_char_budget`) rather than
    /// rejecting, so an absurd value cannot silently produce failing
    /// requests. `None` = the backend's own default budget.
    #[serde(default)]
    pub summarizer_input_char_budget: Option<usize>,

    /// Optional explicit identity key that overrides the one derived from
    /// the `origin` remote (issue #28, epic #24). Checked before any git
    /// call; `None` = derive the key from the origin remote, or `None` if
    /// there is no usable origin.
    #[serde(default)]
    pub identity: Option<String>,
}

/// An explicit subsystem definition with a name and the paths it covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemOverride {
    pub name: String,
    pub paths: Vec<String>,
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            subsystems: Vec::new(),
            exclude: Vec::new(),
            module_depth: None,
            languages: Vec::new(),
            dependency_map: HashMap::new(),
            preserve_function_entities: true,
            summarize: None,
            apfel_endpoint: None,
            summarizer_backend: None,
            summarizer_model: None,
            summarizer_input_char_budget: None,
            identity: None,
        }
    }
}

impl RepoConfig {
    /// Effective apfel server endpoint: `LIEVO_APFEL_ENDPOINT` when set to a
    /// non-empty value, otherwise the `apfel_endpoint` field from config.
    pub fn effective_apfel_endpoint(&self) -> Option<String> {
        match std::env::var("LIEVO_APFEL_ENDPOINT") {
            Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            _ => self.apfel_endpoint.clone(),
        }
    }

    /// The effective summary backend: the `summarizer_backend` field when
    /// set, otherwise `apfel` (the incumbent default).
    pub fn effective_summarizer_backend(&self) -> SummarizerBackend {
        self.summarizer_backend
            .as_deref()
            .and_then(SummarizerBackend::parse)
            .unwrap_or(SummarizerBackend::Apfel)
    }

    /// Load `.lievo/config.yaml` from `repo_path`. `Ok(None)` if absent;
    /// `Err(InvalidConfig)` if present but invalid.
    pub fn load(repo_path: &Path) -> Result<Option<Self>> {
        let config_path = repo_path.join(".lievo").join("config.yaml");

        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(LievoError::InvalidConfig {
                    path: config_path.display().to_string(),
                    reason: e.to_string(),
                });
            }
        };

        // Empty/whitespace/comment-only files are valid ("all defaults");
        // serde_yaml_ng rejects an empty document, so when no line has
        // effective (non-comment) content we return all defaults.
        let has_effective_content = content
            .lines()
            .any(|line| !line.trim().is_empty() && !line.trim().starts_with('#'));
        let config: RepoConfig = if has_effective_content {
            serde_yaml_ng::from_str(&content).map_err(|e| LievoError::InvalidConfig {
                path: config_path.display().to_string(),
                reason: e.to_string(),
            })?
        } else {
            Self::default()
        };

        config.validate()?;

        Ok(Some(config))
    }

    /// Load `.lievo/config.yaml` from `repo_path`, falling back to the all-defaults
    /// config when the file is present but fails to parse or validate (issue #788).
    ///
    /// `load`'s `Err(InvalidConfig)` cases (corrupt YAML, a bad endpoint, an unknown
    /// `summarizer_backend`) previously collapsed silently to defaults at four call
    /// sites, where an invalid backend name could be misread as "unset" and the run
    /// silently key on the apfel default. This helper names the file and the reason
    /// at a `warn!` before falling back, so the same bad config is loud at every
    /// entry point that does not hard-fail (the analyze pipeline keeps `load()?`).
    /// The returned config is never the all-defaults one *because of an error*: a
    /// corrupt file falls back to defaults so the run still proceeds, but the warning
    /// says why, so no backend selection is ever silently substituted.
    pub fn load_or_default(repo_path: &Path) -> Self {
        match Self::load(repo_path) {
            Ok(Some(config)) => config,
            Ok(None) => Self::default(),
            Err(crate::error::LievoError::InvalidConfig { path, reason }) => {
                tracing::warn!("config: falling back to defaults for {path}: {reason}");
                Self::default()
            }
            Err(e) => {
                tracing::warn!(
                    "config: failed to load config for {}: {e}",
                    repo_path.display()
                );
                Self::default()
            }
        }
    }

    /// Validate config values. Returns `Err` for hard violations.
    ///
    /// Soft warnings (e.g. empty language list) are emitted via `tracing::warn!`.
    pub fn validate(&self) -> Result<()> {
        if let Some(d) = self.module_depth
            && d < 1
        {
            return Err(LievoError::InvalidConfig {
                path: ".lievo/config.yaml".to_string(),
                reason: "module_depth must be >= 1".to_string(),
            });
        }

        for subsystem in &self.subsystems {
            if subsystem.name.trim().is_empty() {
                return Err(LievoError::InvalidConfig {
                    path: ".lievo/config.yaml".to_string(),
                    reason: "subsystem name cannot be empty".to_string(),
                });
            }
            if subsystem.paths.is_empty() {
                return Err(LievoError::InvalidConfig {
                    path: ".lievo/config.yaml".to_string(),
                    reason: format!("subsystem '{}' has no paths", subsystem.name),
                });
            }
        }
        if self.languages.is_empty() && !self.subsystems.is_empty() {
            tracing::warn!("config: no language filters set; all languages will be analysed");
        }

        if let Some(endpoint) = &self.apfel_endpoint {
            if endpoint.trim().is_empty() {
                return Err(LievoError::InvalidConfig {
                    path: ".lievo/config.yaml".to_string(),
                    reason: "apfel_endpoint must not be empty".to_string(),
                });
            }
            // Must be an absolute http(s) URL: a bare host:port would fail
            // health validation (the server handle does not invent a scheme).
            let has_scheme = endpoint.starts_with("http://") || endpoint.starts_with("https://");
            if !has_scheme {
                return Err(LievoError::InvalidConfig {
                    path: ".lievo/config.yaml".to_string(),
                    reason: format!(
                        "apfel_endpoint must be an absolute http(s) URL, got '{endpoint}'"
                    ),
                });
            }
        }

        if let Some(backend) = &self.summarizer_backend
            && SummarizerBackend::parse(backend).is_none()
        {
            return Err(LievoError::InvalidConfig {
                path: ".lievo/config.yaml".to_string(),
                reason: format!(
                    "summarizer_backend must be one of {} (case-insensitive), got '{}'",
                    SummarizerBackend::supported_names().join(", "),
                    backend
                ),
            });
        }

        if let Some(model) = &self.summarizer_model
            && model.trim().is_empty()
        {
            return Err(LievoError::InvalidConfig {
                path: ".lievo/config.yaml".to_string(),
                reason: "summarizer_model must not be empty".to_string(),
            });
        }

        // 0 is not a usable budget: the resolver would clamp it to the
        // minimum, but rejecting it here keeps the config file honest
        // (issue #792: an absurd value must be rejected or clamped, never
        // silently produce failing requests). The non-zero range is clamped
        // by `summarization::backend_profile::input_char_budget` itself.
        if let Some(budget) = self.summarizer_input_char_budget
            && budget == 0
        {
            return Err(LievoError::InvalidConfig {
                path: ".lievo/config.yaml".to_string(),
                reason: "summarizer_input_char_budget must be >= 1".to_string(),
            });
        }

        // An explicit identity override must name a key (host/owner/repo
        // shape) — an empty or whitespace-only value would silently produce
        // a wrong key, so it is a hard error like the other Option<String>
        // fields (issue #28, epic #24). Dot/empty segments (`..`, `.`,
        // `a//b`) are path-traversal vectors and are rejected too.
        if let Some(identity) = &self.identity {
            let trimmed = identity.trim();
            if trimmed.is_empty()
                || trimmed
                    .split('/')
                    .any(|seg| seg.is_empty() || seg == "." || seg == "..")
            {
                return Err(LievoError::InvalidConfig {
                    path: ".lievo/config.yaml".to_string(),
                    reason: format!(
                        "identity '{identity}' must not be empty and must not contain empty, '.' or '..' path segments"
                    ),
                });
            }
        }

        Ok(())
    }
}
