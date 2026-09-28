// Offline retrieval-evaluation metrics: recall@k and MRR.
//
// Pure functions over an ordered candidate list (rank order matters) and a
// set of relevant target paths. Used by the retrieval-evaluation harness
// (src/retrieval/retrieval_eval.rs) and the selfcheck probes to score a
// ranked result list. Keeping the metrics separate from the harness keeps
// them unit-testable without a model, a vector index, or any I/O.

/// Fraction of relevant targets found within the first `k` candidates.
///
/// `candidates` is the ranked result list (rank = position + 1); `targets`
/// is the set of relevant repo-relative paths (a single-element set for the
/// probe set used by this project). Returns 0.0 for an empty candidate list
/// or an empty target set — no candidates means no hits, which is 0 recall
/// rather than division by zero.
pub fn recall_at_k(
    candidates: &[String],
    targets: &std::collections::HashSet<String>,
    k: usize,
) -> f32 {
    if candidates.is_empty() || targets.is_empty() {
        return 0.0;
    }
    let hits = candidates
        .iter()
        .take(k)
        .filter(|c| targets.contains(*c))
        .count();
    hits as f32 / targets.len() as f32
}

/// Mean reciprocal rank: 1 / (rank of first relevant hit), or 0.0 when no
/// candidate is relevant. Rank is 1-indexed; an empty candidate list returns
/// 0.0 (no division by zero).
pub fn mrr(candidates: &[String], targets: &std::collections::HashSet<String>) -> f32 {
    if candidates.is_empty() || targets.is_empty() {
        return 0.0;
    }
    candidates
        .iter()
        .position(|c| targets.contains(c))
        .map_or(0.0, |pos| 1.0 / (pos + 1) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(target: &str) -> std::collections::HashSet<String> {
        std::iter::once(target.to_string()).collect()
    }

    #[test]
    fn recall_hit_inside_k() {
        let cands = vec!["a.rs".into(), "b.rs".into(), "target.rs".into()];
        assert!((recall_at_k(&cands, &one("target.rs"), 5) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn recall_hit_outside_k_is_zero() {
        let cands = vec!["a.rs".into(), "b.rs".into(), "target.rs".into()];
        assert_eq!(recall_at_k(&cands, &one("target.rs"), 2), 0.0);
    }

    #[test]
    fn recall_empty_candidates_is_zero() {
        assert_eq!(recall_at_k(&[], &one("target.rs"), 10), 0.0);
    }

    #[test]
    fn recall_empty_targets_is_zero() {
        let cands = vec!["a.rs".into()];
        assert_eq!(
            recall_at_k(&cands, &std::collections::HashSet::new(), 10),
            0.0
        );
    }

    #[test]
    fn recall_partial_with_multiple_targets() {
        // Two of three relevant targets in the top 2 → 2/3.
        let cands = vec!["t1.rs".into(), "t2.rs".into(), "other.rs".into()];
        let targets: std::collections::HashSet<String> = ["t1.rs", "t2.rs", "t3.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let expected = 2.0 / 3.0;
        assert!((recall_at_k(&cands, &targets, 10) - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn mrr_first_position_is_one() {
        let cands = vec!["target.rs".into(), "other.rs".into()];
        assert!((mrr(&cands, &one("target.rs")) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mrr_later_position_is_one_over_rank() {
        let cands = vec!["a.rs".into(), "b.rs".into(), "target.rs".into()];
        assert!((mrr(&cands, &one("target.rs")) - 1.0 / 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mrr_no_hit_is_zero() {
        let cands = vec!["a.rs".into(), "b.rs".into()];
        assert_eq!(mrr(&cands, &one("target.rs")), 0.0);
    }

    #[test]
    fn mrr_empty_candidates_is_zero() {
        assert_eq!(mrr(&[], &one("target.rs")), 0.0);
    }

    #[test]
    fn mrr_uses_first_relevant_hit_only() {
        // Two relevant targets at ranks 2 and 4 → MRR counts rank 2 only.
        let cands = vec!["a.rs".into(), "t1.rs".into(), "b.rs".into(), "t2.rs".into()];
        let targets: std::collections::HashSet<String> =
            ["t1.rs", "t2.rs"].iter().map(|s| s.to_string()).collect();
        assert!((mrr(&cands, &targets) - 1.0 / 2.0).abs() < f32::EPSILON);
    }
}
