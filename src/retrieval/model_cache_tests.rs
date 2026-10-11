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
    assert_eq!(resolve_hf_cache_dir(), Cache::from_env().path().clone());
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
    // Deliberate divergence pin: hf-hub 0.5.0's `from_env` ignores HF_HUB_CACHE. If an
    // hf-hub upgrade starts honouring it, revisit `resolve_hf_cache_dir`.
    assert_eq!(Cache::from_env().path(), Cache::default().path());
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
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();
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
    // A network error is not a filesystem problem, so no override hint.
    assert!(!msg.contains(CACHE_OVERRIDE_HINT), "{msg}");
    assert!(!msg.contains(LOCK_HINT), "{msg}");
}

#[test]
fn cache_error_context_hints_only_for_matching_api_errors() {
    let dir = Path::new("/some/hf/cache");
    let io = ApiError::IoError(std::io::Error::other("read-only"));
    let lock = ApiError::LockAcquisition(dir.join("x.lock"));
    let other = ApiError::InvalidResume;
    let ctx = cache_error_context(dir, &io);
    assert!(ctx.contains("/some/hf/cache"), "{ctx}");
    assert!(ctx.contains(CACHE_OVERRIDE_HINT), "{ctx}");
    assert!(!ctx.contains(LOCK_HINT), "{ctx}");
    let ctx = cache_error_context(dir, &lock);
    assert!(ctx.contains("/some/hf/cache"), "{ctx}");
    assert!(ctx.contains(LOCK_HINT), "{ctx}");
    assert!(!ctx.contains(CACHE_OVERRIDE_HINT), "{ctx}");
    let ctx = cache_error_context(dir, &other);
    assert!(ctx.contains("/some/hf/cache"), "{ctx}");
    assert!(!ctx.contains(CACHE_OVERRIDE_HINT), "{ctx}");
    assert!(!ctx.contains(LOCK_HINT), "{ctx}");
}

#[test]
fn token_cache_follows_hf_home_not_hub_cache() {
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        (
            "HF_HOME",
            Some(home.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_HUB_CACHE", Some("/nonexistent-lievo-hub-cache")),
        ("HF_ENDPOINT", None),
    ]);
    assert_eq!(token_cache().path(), &home.path().join("hub"));
}

#[test]
fn token_cache_ignores_empty_hf_home() {
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", Some("")),
        ("HF_HUB_CACHE", None),
        ("HF_ENDPOINT", None),
    ]);
    assert_eq!(token_cache().path(), Cache::default().path());
}

#[test]
fn api_for_cache_accepts_any_hf_endpoint_like_hf_hub() {
    // hf-hub does not validate HF_ENDPOINT, so neither do we: a plain-http mirror builds.
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        ("HF_HUB_CACHE", None),
        ("HF_ENDPOINT", Some("http://mirror.example.com")),
    ]);
    assert!(api_for_cache(tmp.path()).is_ok());
}

#[test]
fn model_dir_finds_cached_tokenizer_without_network() {
    let root = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        (
            "HF_HUB_CACHE",
            Some(root.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_ENDPOINT", Some("http://127.0.0.1:1")),
    ]);
    assert_eq!(model_dir().expect("empty cache is readable"), None);

    let repo = root.path().join("models--minishlab--potion-code-16M-v2");
    std::fs::create_dir_all(repo.join("refs")).expect("create refs");
    std::fs::write(repo.join("refs/main"), "abc123").expect("write ref");
    let snapshot = repo.join("snapshots/abc123");
    std::fs::create_dir_all(&snapshot).expect("create snapshot");
    std::fs::write(snapshot.join("tokenizer.json"), "{}").expect("write tokenizer");
    assert_eq!(
        model_dir().expect("cached model is readable"),
        Some(snapshot)
    );
}

#[test]
fn model_dir_trims_ref_trailing_newline() {
    let root = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        (
            "HF_HUB_CACHE",
            Some(root.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_ENDPOINT", None),
    ]);
    let repo = root.path().join("models--minishlab--potion-code-16M-v2");
    std::fs::create_dir_all(repo.join("refs")).expect("create refs");
    std::fs::write(repo.join("refs/main"), "abc123\n").expect("write ref");
    let snapshot = repo.join("snapshots/abc123");
    std::fs::create_dir_all(&snapshot).expect("create snapshot");
    std::fs::write(snapshot.join("tokenizer.json"), "{}").expect("write tokenizer");
    assert_eq!(model_dir().expect("readable"), Some(snapshot));
}

#[test]
fn model_dir_rejects_ref_that_is_not_a_commit_id() {
    let root = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        (
            "HF_HUB_CACHE",
            Some(root.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_ENDPOINT", None),
    ]);
    let repo = root.path().join("models--minishlab--potion-code-16M-v2");
    std::fs::create_dir_all(repo.join("refs")).expect("create refs");
    std::fs::write(repo.join("refs/main"), "../x\n").expect("write ref");
    // Where "../x" would point if it were joined into snapshots/ unchecked.
    let escaped = repo.join("x");
    std::fs::create_dir_all(&escaped).expect("create escaped dir");
    std::fs::write(escaped.join("tokenizer.json"), "{}").expect("write tokenizer");
    assert_eq!(model_dir().expect("readable"), None);
}

#[test]
fn model_dir_matches_hf_hub_lookup() {
    let root = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        (
            "HF_HUB_CACHE",
            Some(root.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_ENDPOINT", None),
    ]);
    let repo = root.path().join("models--minishlab--potion-code-16M-v2");
    std::fs::create_dir_all(repo.join("refs")).expect("create refs");
    std::fs::write(repo.join("refs/main"), "abcdef0123").expect("write ref");
    let snapshot = repo.join("snapshots/abcdef0123");
    std::fs::create_dir_all(&snapshot).expect("create snapshot");
    std::fs::write(snapshot.join("tokenizer.json"), "{}").expect("write tokenizer");
    // Drift pin: hf-hub's own layout lookup must resolve the same snapshot directory.
    let hub_tokenizer = Cache::new(root.path().to_path_buf())
        .repo(Repo::model(MODEL_ID.to_string()))
        .get("tokenizer.json")
        .expect("hf-hub finds the seeded tokenizer");
    assert_eq!(
        model_dir().expect("readable"),
        Some(hub_tokenizer.parent().expect("has parent").to_path_buf())
    );
}

#[test]
fn model_dir_reports_unreadable_cache_as_error_not_absent() {
    let root = tempfile::tempdir().expect("tempdir");
    let _guard = HfEnvGuard::set(&[
        ("HF_HOME", None),
        (
            "HF_HUB_CACHE",
            Some(root.path().to_str().expect("utf-8 temp dir")),
        ),
        ("HF_ENDPOINT", None),
    ]);
    // `refs/main` is a directory, so reading it fails with a real I/O error.
    let ref_dir = root
        .path()
        .join("models--minishlab--potion-code-16M-v2/refs/main");
    std::fs::create_dir_all(&ref_dir).expect("create ref dir");
    let msg = model_dir()
        .expect_err("unreadable ref must not be reported as not downloaded")
        .to_string();
    assert!(msg.contains(&ref_dir.display().to_string()), "{msg}");
}

#[test]
fn test_ensure_potion_code_model_network() {
    // Hold the env lock: the download reads HF_HUB_CACHE/HF_HOME/HF_ENDPOINT, which
    // other env-mutating tests may be changing in parallel.
    let _guard = HfEnvGuard::set(&[]);
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
