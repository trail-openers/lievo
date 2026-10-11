// Model caching and download utilities for semantic search.

use hf_hub::{
    Cache, Repo, RepoType,
    api::sync::{Api, ApiBuilder, ApiError},
};
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
/// Remediation shown wherever the HF cache directory is the likely cause of a failure.
const CACHE_OVERRIDE_HINT: &str = "set HF_HOME or HF_HUB_CACHE to a writable directory to override";
/// Remediation for a lock held by another download: the lock is usually a concurrent lievo run.
const LOCK_HINT: &str = "another lievo process may be downloading the model; retry, or set HF_HOME/HF_HUB_CACHE to a writable directory";

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

/// `HF_HOME/hub` when `HF_HOME` is set and non-empty; `None` otherwise.
fn hf_home_hub() -> Option<PathBuf> {
    non_empty_env("HF_HOME").map(|home| PathBuf::from(home).join("hub"))
}

/// Cache whose `token` entry holds the HF login: `HF_HOME/hub` when `HF_HOME` is set and
/// non-empty, else the default cache. Never derived from `HF_HUB_CACHE`, and never from an
/// empty `HF_HOME` (hf-hub would turn that into a relative `./token`).
fn token_cache() -> Cache {
    hf_home_hub().map_or_else(Cache::default, Cache::new)
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
    if let Some(hub) = hf_home_hub() {
        return hub;
    }
    Cache::default().path().clone()
}

/// Builds a HuggingFace API client rooted at `cache_dir`, honouring `HF_ENDPOINT` the
/// way `ApiBuilder::from_env` does (a non-empty value replaces the hub URL).
fn api_for_cache(cache_dir: &Path) -> crate::Result<Api> {
    // Token follows HF_HOME (or the default cache), not `cache_dir`, as `from_env` does.
    let mut builder = ApiBuilder::from_cache(Cache::new(cache_dir.to_path_buf()))
        .with_token(token_cache().token());
    if let Some(endpoint) = non_empty_env("HF_ENDPOINT") {
        let endpoint = endpoint.into_string().map_err(|_| {
            crate::LievoError::RetrievalError("HF_ENDPOINT is not valid UTF-8".into())
        })?;
        builder = builder.with_endpoint(endpoint);
    }
    builder.build().map_err(|e| {
        crate::LievoError::RetrievalError(format!(
            "failed to init HuggingFace API ({})",
            cache_error_context(cache_dir, &e)
        ))
    })
}

/// Context for a failed HF cache operation. Always names the cache dir; the override
/// hint is added only for filesystem I/O failures, not for network or HTTP errors.
fn cache_error_context(cache_dir: &Path, err: &ApiError) -> String {
    let dir = cache_dir.display();
    match err {
        ApiError::IoError(_) => format!("HF cache dir {dir}; {CACHE_OVERRIDE_HINT}"),
        ApiError::LockAcquisition(_) => format!("HF cache dir {dir}; {LOCK_HINT}"),
        _ => format!("HF cache dir {dir}"),
    }
}

/// Internal function that performs the actual download.
fn do_download() -> crate::Result<PathBuf> {
    let cache_dir = resolve_hf_cache_dir();
    let api = api_for_cache(&cache_dir)?;

    let repo = api.repo(Repo::new(MODEL_ID.to_string(), RepoType::Model));
    let fetch = |artifact: &str| {
        repo.get(artifact).map_err(|e| {
            crate::LievoError::RetrievalError(format!(
                "failed to download {MODEL_ID} {artifact}: {e} ({})",
                cache_error_context(&cache_dir, &e)
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

/// Directory containing the downloaded model snapshot, read from the on-disk hub
/// layout (`refs/<revision>` → `snapshots/<commit>/tokenizer.json`). Never downloads
/// and never touches the network, so `lievo doctor` can call it safely.
///
/// `Ok(None)` means the model is not cached (missing or empty ref).
/// `Err` also covers a non-empty ref that is not a plain commit id, since that is a
/// malformed cache rather than an absent model.
/// `Err` means the cache exists but cannot be read, which must not be reported as
/// "not downloaded". hf-hub's `CacheRepo::get` collapses every I/O error to `None`,
/// so this reads the files directly.
pub fn model_dir() -> crate::Result<Option<PathBuf>> {
    let cache_dir = resolve_hf_cache_dir();
    let repo = Repo::new(MODEL_ID.to_string(), RepoType::Model);
    let repo_dir = cache_dir.join(repo.folder_name());
    let read_err = |path: &Path, e: std::io::Error| {
        crate::LievoError::RetrievalError(format!(
            "cannot read HF model cache at {}: {e}",
            path.display()
        ))
    };

    let ref_path = repo_dir.join("refs").join(repo.revision());
    let commit = match std::fs::read_to_string(&ref_path) {
        Ok(commit) => commit.trim().to_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(read_err(&ref_path, e)),
    };
    if commit.is_empty() {
        return Ok(None);
    }
    // The ref is joined into a path, so only a plain hex commit id may be used.
    if !is_plain_commit_id(&commit) {
        return Err(crate::LievoError::RetrievalError(format!(
            "invalid ref in HF cache at {}",
            ref_path.display()
        )));
    }

    let snapshot = repo_dir.join("snapshots").join(commit);
    let tokenizer = snapshot.join("tokenizer.json");
    match std::fs::metadata(&tokenizer) {
        Ok(_) => Ok(Some(snapshot)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(read_err(&tokenizer, e)),
    }
}

/// A non-empty ASCII hex string: no separators or dots, so it cannot escape `snapshots/`.
fn is_plain_commit_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
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
#[path = "model_cache_staleness_tests.rs"]
mod staleness_tests;

#[cfg(test)]
#[path = "model_cache_tests.rs"]
mod tests;
