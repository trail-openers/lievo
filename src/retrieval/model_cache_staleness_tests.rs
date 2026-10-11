// Staleness-marker tests for `model_cache::index_staleness_marker_matches` (issue #716).

use super::*;

fn staleness_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lievo-staleness-{tag}-{}", std::process::id()));
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
