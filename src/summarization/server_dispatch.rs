// Server acquisition dispatch (issue #776) — resolves the named backend and
// endpoint from the repo config, acquires the server handle, returns the
// HTTP transport as a value for the caller to thread down the call path
// (issue #783), and emits the backend-named unavailable message when
// acquisition degrades.
//
// Extracted from `pipeline.rs` to keep that file within the 500-line limit
// (AGENTS.md §6); this file is `include!`d into the pipeline module (see
// `mod server_dispatch` in `pipeline.rs`), so `crate::` paths resolve
// normally and the super-probe is not affected.
//
// # No silent cross-backend fallback (issue #780)
//
// A configured `generic` / `llama-server` backend that cannot be reached
// NEVER degrades to the apfel CLI (the pre-#780 behaviour silently
// substituted apfel for any non-apfel backend, which on non-Apple platforms
// produced nothing). Instead, the run degrades to the no-summarizer path
// (transport not set; `apfel` remains the default only when no backend is
// configured) and a `Degradation` message naming the backend, the endpoint
// label and the reason is returned so the caller emits it — every degradation
// is visible, for every backend.
//
// # Authorization (issue #780)
//
// When `LIEVO_SUMMARIZER_TOKEN` is set (and non-empty), the resolved token is
// passed to `BackendTransport`; the HTTP path sends `Authorization:
// Bearer <token>` on both the health probe and the chat request. The token
// never appears in any message, warning, or error string constructed here —
// only the endpoint's `host_label` does.

use crate::config::{RepoConfig, SummarizerBackend};
use crate::summarization::apfel::BackendTransport;
use crate::summarization::summarizer_backend::{ApfelServerHandle, acquire_server, host_label};

/// Environment variable name for the summarizer bearer token (issue #780).
///
/// The token is deliberately NOT a config-file field: config is checked in,
/// while the token must stay out of version control. An empty value is
/// treated as unset (no header is sent). The single definition: the
/// `summarizer_backend::summarizer_token` reader (health probes and chat
/// POSTs) resolves it through this path.
pub const SUMMARIZER_TOKEN_ENV: &str = "LIEVO_SUMMARIZER_TOKEN";

/// A degradation from a configured backend to the no-summarizer path.
///
/// Constructed only by [`acquire_for_run`]; the caller is the single print
/// site (via `Degradation::message` + `eprintln!`), which keeps the warning
/// text testable without capturing stderr (issue #780 residual-gap contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Degradation {
    /// The backend the user configured (e.g. `Generic`, `LlamaServer`).
    pub backend: SummarizerBackend,
    /// The endpoint's `host:port` label (scheme stripped, never the token).
    pub endpoint: String,
    /// The human-readable reason acquisition failed (never the token).
    pub reason: String,
}

impl Degradation {
    /// Build the warning text. The token is never part of any field, so the
    /// message is safe to log, echo, or serialize.
    pub(crate) fn message(&self) -> String {
        format!(
            "warning: {} backend at {} is unavailable ({}); \
             degrading to the no-summarizer path — no summaries will be produced",
            crate::summarization::backend_profile::display_name(self.backend),
            self.endpoint,
            self.reason,
        )
    }
}

/// Resolve the endpoint + backend, acquire the server handle, and build the
/// HTTP transport value for the run.
///
/// Returns the server handle (if any) — the RAII guard the caller keeps
/// alive for the run and drops at the end (shutting down a spawned server) —
/// the transport value for the call path (issue #783), and the degradation
/// message (if the configured backend was not reached), which the caller
/// emits via `eprintln!`.
///
/// # Degradation (never a hard error, never silent)
///
/// When the endpoint is configured but the server could not be acquired
/// (wrong-service health mismatch, non-spawnable backend not running, spawn
/// failure), the transport is NOT set — the run degrades to the no-summarizer
/// path — and a `Degradation` naming the CONFIGURED backend, the endpoint
/// label and the reason is returned. The apfel CLI is never substituted for a
/// different configured backend (issue #780); `apfel` remains the default
/// only when no backend is configured at all.
/// Build the remote-endpoint notice for a successful acquisition: `Some` of
/// the warning string when a usable endpoint is configured on a host that is
/// NOT loopback, `None` otherwise (no endpoint, or loopback — the normal local
/// setup must stay silent).
///
/// Pure and testable without capturing stderr (same pattern as
/// [`Degradation::message`]). The host label goes through
/// `summarizer_backend::host_label` (strips the scheme) and never touches the
/// token; because `host_label` does not strip userinfo, any `user:pass@`
/// portion is removed before printing so a configured credential can never be
/// echoed.
pub(crate) fn remote_endpoint_warning(repo_config: &RepoConfig) -> Option<String> {
    let endpoint = repo_config.effective_apfel_endpoint()?;
    // `host_label` strips the scheme but not userinfo, so strip `user:pass@`
    // before the host is printed — credentials must never be echoed.
    let label = host_label(&endpoint);
    let host = label.rsplit_once('@').map(|(_, h)| h).unwrap_or(&label);
    if host.is_empty() || is_loopback_host(host) {
        return None;
    }
    Some(format!(
        "warning: summarizing via remote endpoint {host}; source code will be sent to this host"
    ))
}

fn is_loopback_host(host_port: &str) -> bool {
    // Strip `:port` (bracket-aware for IPv6) so `127.0.0.1:8080` and
    // `[::1]:8080` are both recognised as loopback.
    let host = if let Some(bracket_end) = host_port.find(']') {
        host_port
            .get(1..bracket_end)
            .map(str::to_string)
            .unwrap_or_default()
    } else {
        let last_colon = host_port.rfind(':').unwrap_or(0);
        let port_part = &host_port[last_colon..];
        if port_part.starts_with(':')
            && port_part[1..].chars().all(|c| c.is_ascii_digit())
            && !port_part[1..].is_empty()
        {
            host_port[..last_colon].to_string()
        } else {
            host_port.to_string()
        }
    };
    host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "0.0.0.0"
}

/// Returns the server handle (if any), the HTTP transport for the run as a
/// value (issue #783: no ambient state — the caller threads it down the call
/// path), and the degradation (if the configured backend was not reached).
pub(crate) fn acquire_for_run(
    repo_config: &RepoConfig,
) -> (Option<ApfelServerHandle>, Option<BackendTransport>, Option<Degradation>) {
    let endpoint = repo_config.effective_apfel_endpoint();
    let backend = repo_config.effective_summarizer_backend();

    let handle = endpoint
        .as_deref()
        .and_then(|ep| acquire_server(backend, ep));

    // The HTTP transport: endpoint plus the backend-aware model field
    // (issue #776 decision 1: a non-apfel server rejecting an unknown model
    // with a 400 would short-circuit the 429 retry and the demux entirely).
    // The bearer token is not carried on the transport — the HTTP path
    // resolves it at request time via `summarizer_token()` (issue #780).
    let transport = endpoint.as_ref().zip(handle.as_ref()).map(|(ep, _)| BackendTransport {
        url: ep.clone(),
        backend,
        model_name: repo_config.summarizer_model.clone(),
    });

    let degradation = if endpoint.is_some() && handle.is_none() {
        let ep = endpoint.as_deref().expect("endpoint set");
        let label = host_label(ep);
        let reason = if crate::summarization::backend_profile::can_spawn(backend) {
            "spawn or health validation failed"
        } else {
            "server not running; it must be started separately"
        };
        Some(Degradation {
            backend,
            endpoint: label,
            reason: reason.to_string(),
        })
    } else {
        None
    };

    (handle, transport, degradation)
}
