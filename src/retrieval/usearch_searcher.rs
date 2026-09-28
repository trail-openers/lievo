use super::semantic_searcher::SemanticSearcher;
use super::{RetrievalSource, SearchResult};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct VectorMetadata {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) snippet: String,
}

/// On-disk metadata envelope: the embedding model id the index was built
/// with, plus the per-vector metadata. Stored beside the `.usearch` index
/// file so `load()` can detect vectors built with a different (stale) model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct IndexMeta {
    /// Embedding model id the index vectors were built with. Absent on
    /// indices written before this marker existed (pre-swap); such indices
    /// are treated as stale, not silently trusted.
    #[serde(default)]
    pub(crate) model_id: Option<String>,
    pub(crate) vectors: HashMap<u64, VectorMetadata>,
}

/// Semantic searcher backed by model2vec embeddings and usearch index.
pub struct UsearchSearcher {
    index: usearch::Index,
    metadata: HashMap<u64, VectorMetadata>,
    model: Arc<model2vec::Model2Vec>,
}

impl UsearchSearcher {
    /// Load the model and return (model, dimensionality).
    fn init_model() -> crate::Result<(Arc<model2vec::Model2Vec>, usize)> {
        let model_dir = super::model_cache::ensure_potion_code_model()?;
        let model = model2vec::Model2Vec::from_pretrained(&model_dir, None, None).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to load model2vec: {}", e))
        })?;
        let sample_embeddings = model.encode(["test"]).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to test encode: {}", e))
        })?;
        let dim = sample_embeddings.ncols();
        Ok((Arc::new(model), dim))
    }

    /// Build a new UsearchSearcher from code units and save to disk.
    pub fn build(code_units: &[crate::model::CodeUnit], index_path: &Path) -> crate::Result<Self> {
        let (model, dim) = Self::init_model()?;

        // Create usearch index with cosine similarity
        let options = usearch::IndexOptions {
            dimensions: dim,
            metric: usearch::MetricKind::Cos,
            quantization: usearch::ScalarKind::F32,
            connectivity: 0,     // auto
            expansion_add: 0,    // auto
            expansion_search: 0, // auto
            multi: false,
        };

        let index = usearch::new_index(&options).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to create index: {}", e))
        })?;

        // Prepare texts for encoding
        let texts: Vec<String> = code_units
            .iter()
            .map(|u| {
                format!(
                    "{} {}",
                    u.name,
                    u.code.as_deref().or(u.signature.as_deref()).unwrap_or("")
                )
            })
            .collect();

        // Encode all texts in batch
        let embeddings = model.encode(&texts).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to encode texts: {}", e))
        })?;

        // Reserve capacity
        index
            .reserve(code_units.len())
            .map_err(|e| crate::LievoError::RetrievalError(format!("Failed to reserve: {}", e)))?;

        // Add vectors to index and collect metadata
        let mut metadata = HashMap::new();
        for (idx, unit) in code_units.iter().enumerate() {
            let key = idx as u64;
            let embedding = embeddings.row(idx);

            // Convert ndarray view to Vec<f32>
            let vector: Vec<f32> = embedding.to_vec();

            index.add(key, &vector).map_err(|e| {
                crate::LievoError::RetrievalError(format!("Failed to add vector {}: {}", idx, e))
            })?;

            // Store metadata
            let snippet = unit
                .code
                .as_deref()
                .or(unit.signature.as_deref())
                .unwrap_or("")
                .chars()
                .take(200)
                .collect::<String>();

            metadata.insert(
                key,
                VectorMetadata {
                    path: unit.file.clone(),
                    name: unit.name.clone(),
                    snippet,
                },
            );
        }

        // Save index to disk
        let path_str = index_path
            .to_str()
            .ok_or_else(|| crate::LievoError::RetrievalError("Invalid path".into()))?;

        index.save(path_str).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to save index: {}", e))
        })?;

        // Save metadata to JSON, including the model-id marker so a future
        // load() can detect an index built with a different (stale) model.
        // The map is moved into the envelope (no clone); the meta file is
        // landed atomically (temp + rename) and a failed write removes the
        // just-saved index so on-disk state is "no index", not a marker-less index.
        let meta_path = index_path.with_extension("usearch.meta.json");
        let meta = IndexMeta {
            model_id: Some(super::model_cache::MODEL_ID.to_string()),
            vectors: std::mem::take(&mut metadata),
        };
        let meta_json = serde_json::to_string(&meta).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to serialize metadata: {}", e))
        })?;
        let tmp_meta_path = index_path.with_extension("usearch.meta.json.tmp");
        if let Err(e) = std::fs::write(&tmp_meta_path, meta_json)
            .and_then(|()| std::fs::rename(&tmp_meta_path, &meta_path))
        {
            let _ = std::fs::remove_file(index_path);
            return Err(crate::LievoError::RetrievalError(format!(
                "Failed to write metadata: {}",
                e
            )));
        }

        // Restore the searcher's in-memory metadata from the envelope we just
        // persisted (kept identical to what build() produced).
        let metadata = meta.vectors;

        Ok(Self {
            index,
            metadata,
            model,
        })
    }

    /// Load a UsearchSearcher from disk.
    pub fn load(index_path: &Path) -> crate::Result<Self> {
        // Verify both files exist upfront
        let path_str = index_path
            .to_str()
            .ok_or_else(|| crate::LievoError::RetrievalError("Invalid path".into()))?;

        let meta_path = index_path.with_extension("usearch.meta.json");

        if !index_path.exists() {
            return Err(crate::LievoError::RetrievalError(format!(
                "Vector index not found at {}",
                path_str
            )));
        }
        if !meta_path.exists() {
            return Err(crate::LievoError::RetrievalError(format!(
                "Vector index metadata not found at {}",
                meta_path.display()
            )));
        }

        // Load metadata from JSON and verify the model-id marker matches the
        // current model *before* touching the network or usearch: a missing
        // marker means the index predates staleness tracking (pre-swap) and
        // must count as stale, not silently trusted.
        let meta_json = std::fs::read_to_string(&meta_path).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to read metadata: {}", e))
        })?;
        let meta: IndexMeta = serde_json::from_str(&meta_json).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to parse metadata: {}", e))
        })?;

        match meta.model_id.as_deref() {
            Some(id) if id == super::model_cache::MODEL_ID => {}
            Some(id) => {
                return Err(crate::LievoError::RetrievalError(format!(
                    "Vector index at {} was built with model '{}' but the current model is '{}'; run `lievo refresh` to rebuild",
                    path_str,
                    id,
                    super::model_cache::MODEL_ID
                )));
            }
            None => {
                return Err(crate::LievoError::RetrievalError(format!(
                    "Vector index at {} has no model-id marker (built before staleness tracking); run `lievo refresh` to rebuild",
                    path_str
                )));
            }
        }

        // Load model and get dimensionality
        let (model, dim) = Self::init_model()?;

        // Create empty index and load from disk
        let options = usearch::IndexOptions {
            dimensions: dim,
            metric: usearch::MetricKind::Cos,
            quantization: usearch::ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };

        let index = usearch::new_index(&options).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to create index: {}", e))
        })?;

        index.load(path_str).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to load index: {}", e))
        })?;

        let metadata = meta.vectors;

        Ok(Self {
            index,
            metadata,
            model,
        })
    }
}

impl SemanticSearcher for UsearchSearcher {
    fn search(&self, query: &str, limit: usize) -> crate::Result<Vec<SearchResult>> {
        // Encode query
        let query_embeddings = self.model.encode([query]).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to encode query: {}", e))
        })?;

        // Get first row as a Vec
        let query_vec: Vec<f32> = query_embeddings.row(0).to_vec();

        // Search index
        let results = self.index.search(&query_vec, limit).map_err(|e| {
            crate::LievoError::RetrievalError(format!("Failed to search index: {}", e))
        })?;

        // Convert to SearchResult
        let search_results = results
            .keys
            .iter()
            .zip(results.distances.iter())
            .filter_map(|(key, distance)| {
                self.metadata.get(key).map(|meta| {
                    // Convert usearch Cos metric value to a similarity score in [0, 1].
                    // usearch returns the raw cosine value (in [-1, 1]) under the Cos metric
                    // where higher is more similar; normalise to [0, 1] by shifting by +1 and
                    // scaling by 1/2. Preserve relative ordering across the full range: two
                    // vectors with different cosine values always produce different scores, and
                    // the clamp guards against slight HNSW approximation drift outside [-1, 1].
                    let score = ((distance.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32)
                        .clamp(0.0_f32, 1.0_f32);

                    SearchResult {
                        entity_id: format!("usearch:{}", meta.path),
                        name: meta.name.clone(),
                        path: Some(meta.path.clone()),
                        snippet: meta.snippet.clone(),
                        score,
                        source: RetrievalSource::SemanticCode,
                        tier: "function".to_string(),
                    }
                })
            })
            .collect();

        Ok(search_results)
    }
}

/// Public factory function for building a UsearchSearcher.
pub fn build_usearch_searcher(
    code_units: &[crate::model::CodeUnit],
    index_path: &std::path::Path,
) -> crate::Result<UsearchSearcher> {
    UsearchSearcher::build(code_units, index_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_score_identical_vectors() {
        // Identical vectors: cosine = 1.0 -> score 1.0
        let score =
            ((1.0_f32.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_cosine_score_orthogonal_vectors() {
        // Orthogonal vectors: cosine = 0.0 -> score 0.5
        let score =
            ((0.0_f32.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32);
        assert_eq!(score, 0.5);
    }

    #[test]
    fn test_cosine_score_opposite_vectors() {
        // Opposite vectors: cosine = -1.0 -> score 0.0
        let score =
            ((-1.0_f32.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_cosine_score_preserves_relative_ordering() {
        // Across the full similarity range, higher cosine must yield higher score.
        let values = [-1.0_f32, -0.5_f32, 0.0_f32, 0.5_f32, 1.0_f32];
        let scores: Vec<f32> = values
            .iter()
            .map(|v| ((v.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32))
            .collect();
        for i in 0..values.len().saturating_sub(1) {
            assert!(
                scores[i + 1] > scores[i],
                "ordering broken between {} and {}: {:?} vs {:?}",
                values[i],
                values[i + 1],
                scores[i],
                scores[i + 1]
            );
        }
    }

    #[test]
    fn test_cosine_score_clamps_hnsw_drift() {
        // HNSW approximate search can return values slightly outside [-1, 1];
        // the mapping must clamp them to the valid range rather than panic or overflow.
        let high = ((1.5_f32.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32);
        assert_eq!(high, 1.0);
        let low = ((-1.5_f32.clamp(-1.0_f32, 1.0_f32) + 1.0_f32) / 2.0_f32).clamp(0.0_f32, 1.0_f32);
        assert_eq!(low, 0.0);
    }

    #[test]
    fn test_vector_metadata_serde() {
        let meta = VectorMetadata {
            path: "src/main.rs".to_string(),
            name: "main".to_string(),
            snippet: "fn main() {}".to_string(),
        };

        let json = serde_json::to_string(&meta).expect("Failed to serialize");
        let deserialized: VectorMetadata =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.path, meta.path);
        assert_eq!(deserialized.name, meta.name);
        assert_eq!(deserialized.snippet, meta.snippet);
    }

    #[test]
    fn test_metadata_hashmap_serde() {
        let mut metadata = HashMap::new();
        metadata.insert(
            0,
            VectorMetadata {
                path: "src/lib.rs".to_string(),
                name: "lib".to_string(),
                snippet: "pub fn init() {}".to_string(),
            },
        );

        let json = serde_json::to_string(&metadata).expect("Failed to serialize");
        let deserialized: HashMap<u64, VectorMetadata> =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.len(), 1);
        let meta = deserialized.get(&0).expect("Should have key 0");
        assert_eq!(meta.name, "lib");
    }

    #[test]
    fn test_index_meta_serde_round_trip_with_model_id() {
        let mut vectors = HashMap::new();
        vectors.insert(
            0,
            VectorMetadata {
                path: "src/lib.rs".to_string(),
                name: "lib".to_string(),
                snippet: "pub fn init() {}".to_string(),
            },
        );
        let meta = IndexMeta {
            model_id: Some("minishlab/potion-code-16M-v2".to_string()),
            vectors,
        };

        let json = serde_json::to_string(&meta).expect("Failed to serialize");
        let deserialized: IndexMeta = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(
            deserialized.model_id.as_deref(),
            Some("minishlab/potion-code-16M-v2")
        );
        assert_eq!(deserialized.vectors.len(), 1);
    }

    #[test]
    fn test_index_meta_missing_model_id_defaults_to_none() {
        // Simulates a pre-swap meta JSON that has no `model_id` field at all
        // (written before the staleness marker existed). `#[serde(default)]`
        // must decode this as `None`, which `load()` then treats as stale.
        let legacy_json = r#"{"vectors":{"0":{"path":"src/lib.rs","name":"lib","snippet":""}}}"#;
        let deserialized: IndexMeta =
            serde_json::from_str(legacy_json).expect("Failed to deserialize legacy meta");
        assert_eq!(deserialized.model_id, None);
        assert_eq!(deserialized.vectors.len(), 1);
    }

    #[test]
    fn test_load_rejects_missing_model_id_marker() {
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
}
