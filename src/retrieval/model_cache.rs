// Model caching and download utilities for semantic search.

use hf_hub::{Cache, Repo, RepoType, api::sync::Api, api::sync::ApiBuilder};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// HuggingFace hub id for the embedding model used by semantic search.
/// Shared with `usearch_searcher` so the on-disk index staleness marker and
/// the model actually loaded can never drift apart.
pub const MODEL_ID: &str = "minishlab/potion-code-16M-v2";
/// Timeout for model download (in seconds). Allows ~16MB+ downloads on slow links.
const MODEL_DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// Ensures the potion-code-16M-v2 model artefacts are downloaded (cached under
/// the HuggingFace hub cache) and returns the directory containing them.
///
/// Downloads three required artefacts: tokenizer.json, model.safetensors, config.json.
/// All artefacts are cached in the same snapshot directory by hf-hub.
/// The download is bounded by MODEL_DOWNLOAD_TIMEOUT_SECS to prevent indefinite hangs.
pub fn ensure_potion_code_model() -> crate::Result<PathBuf> {
    let (tx, rx) = mpsc::channel();

    // Spawn download on a separate thread with bounded timeout.
    // An orphaned thread (if timeout fires) will harmlessly finish downloading
    // and write to the HF cache; no cleanup needed.
    thread::spawn(move || {
        let result = do_download();
        // Best-effort send: if receiver dropped, the send fails silently.
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(MODEL_DOWNLOAD_TIMEOUT_SECS)) {
        Ok(Ok(path)) => Ok(path),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(crate::LievoError::RetrievalError(format!(
            "model download timed out after {MODEL_DOWNLOAD_TIMEOUT_SECS} seconds"
        ))),
    }
}

/// Returns the env var's value, treating unset and empty as absent.
fn non_empty_env(var: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(var).filter(|v| !v.is_empty())
}

/// Resolves the HuggingFace hub cache directory the model lives in.
///
/// hf-hub 0.5.0's `Cache::from_env` only reads `HF_HOME`, so lievo resolves it
/// itself: `HF_HUB_CACHE` (used as the hub dir directly), then `HF_HOME/hub`,
/// then hf-hub's default `~/.cache/huggingface/hub`. Empty values are ignored.
/// Download and lookup both go through this so they always agree.
pub(crate) fn resolve_hf_cache_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("HF_HUB_CACHE") {
        return PathBuf::from(dir);
    }
    if let Some(home) = non_empty_env("HF_HOME") {
        return PathBuf::from(home).join("hub");
    }
    Cache::default().path().clone()
}

/// Builds a HuggingFace API client rooted at `cache_dir`, keeping `HF_ENDPOINT`
/// support from `ApiBuilder::from_env`.
///
/// Uses `from_cache`, not `from_env().with_cache_dir()`: the latter would read the
/// token from the default cache. A non-UTF-8 `HF_ENDPOINT` is intentionally ignored,
/// since endpoints are URLs and must be UTF-8 anyway.
fn api_for_cache(cache_dir: &Path) -> Result<Api, hf_hub::api::sync::ApiError> {
    let mut builder = ApiBuilder::from_cache(Cache::new(cache_dir.to_path_buf()));
    if let Some(endpoint) = non_empty_env("HF_ENDPOINT").and_then(|v| v.into_string().ok()) {
        builder = builder.with_endpoint(endpoint);
    }
    builder.build()
}

/// Internal function that performs the actual download.
fn do_download() -> crate::Result<PathBuf> {
    let cache_dir = resolve_hf_cache_dir();
    // Only the init failure gets the writable-directory advice: it is the step where
    // a bad cache dir is the plausible cause.
    let api = api_for_cache(&cache_dir).map_err(|e| {
        crate::LievoError::RetrievalError(format!(
            "failed to init HuggingFace API (HF cache dir {}; set HF_HOME or HF_HUB_CACHE to a writable directory to override): {e}",
            cache_dir.display()
        ))
    })?;

    let repo = api.repo(Repo::new(MODEL_ID.to_string(), RepoType::Model));
    // Every download failure names the cache dir and the override, since a read-only or
    // unwritable cache surfaces here as an io error from `repo.get`.
    let cache_context = format!(
        "HF cache dir {}; set HF_HOME or HF_HUB_CACHE to a writable directory to override",
        cache_dir.display()
    );
    let fetch = |artifact: &str| {
        repo.get(artifact).map_err(|e| {
            crate::LievoError::RetrievalError(format!(
                "failed to download {MODEL_ID} {artifact}: {e} ({cache_context})"
            ))
        })
    };

    // Download all three artefacts model2vec needs; they share one snapshot dir.
    let tokenizer = fetch("tokenizer.json")?;
    fetch("model.safetensors")?;
    fetch("config.json")?;

    let dir = tokenizer.parent().ok_or_else(|| {
        crate::LievoError::RetrievalError("model snapshot path has no parent directory".into())
    })?;

    Ok(dir.to_path_buf())
}

/// Directory containing the downloaded model snapshot (the parent of the
/// tokenizer artefact in the HuggingFace hub cache), without checking that it
/// exists and WITHOUT downloading — used by `lievo doctor` for a read-only
/// "model present on disk" check. `None` when the model has not been
/// downloaded yet. `Err` when the HF API client cannot be initialised, so the
/// caller can report the cause instead of mistaking it for a missing model.
pub fn model_dir() -> crate::Result<Option<PathBuf>> {
    let Some(tokenizer) = repo_get(MODEL_ID, "tokenizer.json")? else {
        return Ok(None);
    };
    Ok(tokenizer.parent().map(Path::to_path_buf))
}

/// Non-downloading lookup of a model artefact in the HF hub cache.
fn repo_get(id: &str, artifact: &str) -> crate::Result<Option<PathBuf>> {
    let cache_dir = resolve_hf_cache_dir();
    let api = api_for_cache(&cache_dir).map_err(|e| {
        crate::LievoError::RetrievalError(format!(
            "failed to init HuggingFace API (HF cache dir {}): {e}",
            cache_dir.display()
        ))
    })?;
    let repo = api.repo(Repo::new(id.to_string(), RepoType::Model));
    Ok(repo.get(artifact).ok().map(|f| f.to_path_buf()))
}

/// Index-staleness detection for the on-disk vector index.
///
/// Reads ONLY the `model_id` marker in the `.usearch.meta.json` artefact beside
/// an index — it never touches the model, the network, or the usearch index
/// itself. Returns `true` when the marker matches the current `MODEL_ID`;
/// `false` in every other case (no meta artefact, unparseable meta, missing
/// marker, or mismatched marker).
///
/// Use this in the refresh pipeline as a cheap "should I rebuild the vector
/// index?" check that runs on every refresh without the cost of loading the
/// model.
///
/// Returns `false` (treated as stale) in every case that means "the on-disk
/// index cannot be trusted": no meta artefact, unparseable meta, missing marker
/// (pre-swap legacy), or a marker that doesn't match the current model.
pub fn index_staleness_marker_matches(index_path: &Path) -> bool {
    use crate::retrieval::usearch_searcher::IndexMeta;
    let meta_path = index_path.with_extension("usearch.meta.json");
    let meta_json = match std::fs::read_to_string(&meta_path) {
        Ok(json) => json,
        Err(_) => return false,
    };
    let meta: IndexMeta = match serde_json::from_str(&meta_json) {
        Ok(meta) => meta,
        Err(_) => return false,
    };
    meta.model_id.as_deref() == Some(MODEL_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sets/removes HF env vars for one test, holding the crate env lock, and
    /// restores the originals on drop so nothing leaks into parallel tests.
    struct HfEnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl HfEnvGuard {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::test_env_support::env_lock();
            let mut saved = Vec::new();
            for (name, value) in vars {
                saved.push((*name, std::env::var_os(name)));
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { saved, _lock: lock }
        }
    }

    impl Drop for HfEnvGuard {
        fn drop(&mut self) {
            for (name, original) in self.saved.drain(..).rev() {
                unsafe {
                    match original {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    #[test]
    fn resolve_hf_cache_dir_defaults_to_hf_hub_default_when_nothing_set() {
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", None),
            ("HF_HUB_CACHE", None),
            ("HF_ENDPOINT", None),
        ]);
        assert_eq!(resolve_hf_cache_dir(), Cache::default().path().clone());
    }

    #[test]
    fn resolve_hf_cache_dir_honours_hf_home() {
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", Some("/nonexistent-lievo-hf-home")),
            ("HF_HUB_CACHE", None),
            ("HF_ENDPOINT", None),
        ]);
        assert_eq!(
            resolve_hf_cache_dir(),
            PathBuf::from("/nonexistent-lievo-hf-home/hub")
        );
        // Parity pin: hf-hub's own HF_HOME resolution must agree with ours, so an
        // hf-hub upgrade that changes it fails here instead of splitting download and lookup.
        assert_eq!(resolve_hf_cache_dir(), Cache::from_env().path().clone());
    }

    #[test]
    fn resolve_hf_cache_dir_honours_hf_hub_cache_verbatim() {
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", None),
            ("HF_HUB_CACHE", Some("/nonexistent-lievo-hub-cache")),
            ("HF_ENDPOINT", None),
        ]);
        assert_eq!(
            resolve_hf_cache_dir(),
            PathBuf::from("/nonexistent-lievo-hub-cache")
        );
    }

    #[test]
    fn resolve_hf_cache_dir_hf_hub_cache_wins_over_hf_home() {
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", Some("/nonexistent-lievo-hf-home")),
            ("HF_HUB_CACHE", Some("/nonexistent-lievo-hub-cache")),
            ("HF_ENDPOINT", None),
        ]);
        assert_eq!(
            resolve_hf_cache_dir(),
            PathBuf::from("/nonexistent-lievo-hub-cache")
        );
    }

    #[test]
    fn resolve_hf_cache_dir_ignores_empty_values() {
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", Some("")),
            ("HF_HUB_CACHE", Some("")),
            ("HF_ENDPOINT", None),
        ]);
        assert_eq!(resolve_hf_cache_dir(), Cache::default().path().clone());
    }

    #[test]
    fn do_download_error_names_cache_path() {
        let dir = std::env::temp_dir().join(format!("lievo-dl-err-{}", std::process::id()));
        let _guard = HfEnvGuard::set(&[
            ("HF_HOME", None),
            ("HF_HUB_CACHE", Some(dir.to_str().expect("utf-8 temp dir"))),
            // Nothing listens on port 1, so the request fails fast without real network access.
            ("HF_ENDPOINT", Some("http://127.0.0.1:1")),
        ]);
        let msg = do_download()
            .expect_err("download from a dead endpoint must fail")
            .to_string();
        assert!(msg.contains("failed to download"), "{msg}");
        assert!(msg.contains(&dir.display().to_string()), "{msg}");
        assert!(msg.contains("HF_HUB_CACHE"), "{msg}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_ensure_potion_code_model_network() {
        // This test requires network access to download the model (potion-code-16M-v2,
        // ~16MB from HuggingFace, cached after first run) so it is not run in CI;
        // it runs locally as an ordinary, un-ignored test when the full
        // suite is executed: cargo test test_ensure_potion_code_model_network
        match ensure_potion_code_model() {
            Ok(dir) => {
                assert!(dir.exists());
                assert!(dir.join("tokenizer.json").exists());
                assert!(dir.join("model.safetensors").exists());
                assert!(dir.join("config.json").exists());
            }
            Err(e) => {
                // Expected if network is unavailable
                eprintln!("Network test skipped (no network): {}", e);
            }
        }
    }

    // ----------------------------------------------------------------------
    // Staleness marker tests (issue #716)
    // ----------------------------------------------------------------------

    fn staleness_temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lievo-staleness-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_marker_meta(index_path: &std::path::Path, model_id: Option<String>) {
        let meta = crate::retrieval::usearch_searcher::IndexMeta {
            model_id,
            vectors: std::collections::HashMap::new(),
        };
        let meta_path = index_path.with_extension("usearch.meta.json");
        std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).expect("write meta");
    }

    #[test]
    fn staleness_no_index_artifact_is_not_stale() {
        // (a) No meta artefact present (no index yet) => not stale.
        let dir = staleness_temp_dir("none");
        let index_path = dir.join("vectors.usearch");
        assert!(!index_staleness_marker_matches(&index_path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staleness_matching_marker_is_fresh() {
        // (b) Marker equals current MODEL_ID => fresh (not stale).
        let dir = staleness_temp_dir("fresh");
        let index_path = dir.join("vectors.usearch");
        std::fs::write(&index_path, b"not a real usearch index").expect("write stub");
        write_marker_meta(&index_path, Some(MODEL_ID.to_string()));
        assert!(index_staleness_marker_matches(&index_path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staleness_mismatched_marker_is_stale() {
        // (c) Marker differs from current MODEL_ID (old model) => stale.
        let dir = staleness_temp_dir("stale-mismatch");
        let index_path = dir.join("vectors.usearch");
        std::fs::write(&index_path, b"not a real usearch index").expect("write stub");
        write_marker_meta(&index_path, Some("minishlab/potion-code-16M".to_string()));
        assert!(!index_staleness_marker_matches(&index_path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staleness_missing_marker_legacy_is_stale() {
        // (d) Meta exists but has no `model_id` field (pre-swap legacy) => stale.
        let dir = staleness_temp_dir("stale-legacy");
        let index_path = dir.join("vectors.usearch");
        std::fs::write(&index_path, b"not a real usearch index").expect("write stub");
        write_marker_meta(&index_path, None);
        assert!(!index_staleness_marker_matches(&index_path));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn staleness_corrupt_meta_is_stale() {
        // Defensive: unparseable meta JSON is treated as stale rather than
        // silently trusted — a rebuild is the safe recovery.
        let dir = staleness_temp_dir("stale-corrupt");
        let index_path = dir.join("vectors.usearch");
        std::fs::write(&index_path, b"not a real usearch index").expect("write stub");
        let meta_path = index_path.with_extension("usearch.meta.json");
        std::fs::write(&meta_path, b"not valid json").expect("write meta");
        assert!(!index_staleness_marker_matches(&index_path));
        std::fs::remove_dir_all(&dir).ok();
    }
}
