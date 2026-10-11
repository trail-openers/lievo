// `UsearchSearcher::load` model-id marker tests. Hold the env lock: load() may reach
// the model loader, which reads HF_HOME / HF_HUB_CACHE / HF_ENDPOINT.

use super::*;

#[test]
fn test_load_rejects_missing_model_id_marker() {
    let _env_lock = crate::test_env_support::env_lock();
    // A pre-swap index has an index file and a meta.json with no
    // `model_id` field. load() must treat this as stale and error out
    // before touching the network or usearch.
    let dir = std::env::temp_dir().join(format!(
        "lievo-usearch-test-missing-marker-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let index_path = dir.join("vectors.usearch");
    let meta_path = index_path.with_extension("usearch.meta.json");

    std::fs::write(&index_path, b"not a real usearch index").expect("write index stub");
    std::fs::write(&meta_path, br#"{"vectors":{}}"#).expect("write legacy meta");

    let result = UsearchSearcher::load(&index_path);
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected load() to reject a missing model-id marker"),
    };
    assert!(
        msg.contains("no model-id marker"),
        "error should mention the missing marker, got: {msg}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_load_rejects_mismatched_model_id_marker() {
    let _env_lock = crate::test_env_support::env_lock();
    // An index built with a different (e.g. old) model id must be
    // rejected rather than silently mixed with the current model's
    // query embeddings.
    let dir = std::env::temp_dir().join(format!(
        "lievo-usearch-test-mismatched-marker-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let index_path = dir.join("vectors.usearch");
    let meta_path = index_path.with_extension("usearch.meta.json");

    std::fs::write(&index_path, b"not a real usearch index").expect("write index stub");
    let meta = IndexMeta {
        model_id: Some("minishlab/potion-code-16M".to_string()),
        vectors: HashMap::new(),
    };
    std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).expect("write meta");

    let result = UsearchSearcher::load(&index_path);
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected load() to reject a mismatched model-id marker"),
    };
    assert!(
        msg.contains("potion-code-16M"),
        "error should mention the stale model id, got: {msg}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
