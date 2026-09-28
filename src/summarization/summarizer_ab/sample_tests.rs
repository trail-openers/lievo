// Unit tests for the fixed A/B entity sample (issue #773).
//
// Test surface (from the issue):
//   - the fixed entity sample is stable across runs

use super::sample::{SAMPLE, SampleEntity, SampleTier, validate_sample};

/// The sample must be stable: two successive reads of `SAMPLE` must produce
/// identical data. Since `SAMPLE` is a `&'static const`, this is trivially
/// true, but the test documents the invariant explicitly.
#[test]
fn test_sample_is_stable_across_reads() {
    // Read 1
    let first_ids: Vec<&str> = SAMPLE.iter().map(|e| e.id).collect();
    let first_names: Vec<&str> = SAMPLE.iter().map(|e| e.name).collect();
    let first_bodies: Vec<&str> = SAMPLE.iter().map(|e| e.body).collect();
    let first_tiers: Vec<&SampleTier> = SAMPLE.iter().map(|e| &e.tier).collect();

    // Read 2 (new iterator, same const)
    let second_ids: Vec<&str> = SAMPLE.iter().map(|e| e.id).collect();
    let second_names: Vec<&str> = SAMPLE.iter().map(|e| e.name).collect();
    let second_bodies: Vec<&str> = SAMPLE.iter().map(|e| e.body).collect();
    let second_tiers: Vec<&SampleTier> = SAMPLE.iter().map(|e| &e.tier).collect();

    assert_eq!(first_ids, second_ids, "ids must be stable");
    assert_eq!(first_names, second_names, "names must be stable");
    assert_eq!(first_bodies, second_bodies, "bodies must be stable");
    assert_eq!(first_tiers, second_tiers, "tiers must be stable");
}

/// The sample must be non-empty.
#[test]
fn test_sample_is_non_empty() {
    assert!(
        !SAMPLE.is_empty(),
        "SAMPLE must contain at least one entity"
    );
}

/// The sample must contain at least one entity of each tier.
#[test]
fn test_sample_spans_both_tiers() {
    assert!(
        SAMPLE.iter().any(|e| e.tier == SampleTier::Function),
        "SAMPLE must contain at least one function-tier entity"
    );
    assert!(
        SAMPLE.iter().any(|e| e.tier == SampleTier::Rollup),
        "SAMPLE must contain at least one rollup-tier entity"
    );
}

/// The sample must contain both short and long function-tier bodies.
#[test]
fn test_sample_spans_short_and_long_bodies() {
    let fn_entities: Vec<&SampleEntity> = SAMPLE
        .iter()
        .filter(|e| e.tier == SampleTier::Function)
        .collect();

    assert!(
        fn_entities.iter().any(|e| e.body.len() < 100),
        "SAMPLE must contain at least one short function-tier entity (<100 chars)"
    );
    assert!(
        fn_entities.iter().any(|e| e.body.len() > 500),
        "SAMPLE must contain at least one long function-tier entity (>500 chars)"
    );
}

/// All entity IDs must be unique.
#[test]
fn test_sample_ids_are_unique() {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    for entity in SAMPLE {
        assert!(seen.insert(entity.id), "duplicate entity id: {}", entity.id);
    }
}

/// All entity names and bodies must be non-empty.
#[test]
fn test_sample_names_and_bodies_non_empty() {
    for entity in SAMPLE {
        assert!(
            !entity.name.is_empty(),
            "entity {} has empty name",
            entity.id
        );
        assert!(
            !entity.body.is_empty(),
            "entity {} has empty body",
            entity.id
        );
    }
}

/// `validate_sample()` must return an empty Vec for the current sample.
#[test]
fn test_validate_sample_passes() {
    let problems = validate_sample();
    assert!(
        problems.is_empty(),
        "validate_sample() found problems: {:?}",
        problems
    );
}

/// The sample size must be at least 4 so that batch sizes 2 and 4 can be
/// tested with real entities.
#[test]
fn test_sample_size_sufficient_for_batch_sweep() {
    assert!(
        SAMPLE.len() >= 4,
        "SAMPLE must have at least 4 entities for batch size 4; has {}",
        SAMPLE.len()
    );
}

/// Each function-tier entity must have a body that looks like Rust code
/// (contains at least one `{` and `}`).
#[test]
fn test_function_tier_bodies_look_like_rust() {
    for entity in SAMPLE.iter().filter(|e| e.tier == SampleTier::Function) {
        assert!(
            entity.body.contains('{') && entity.body.contains('}'),
            "function-tier entity {} body does not look like Rust code",
            entity.id
        );
    }
}

/// Each rollup-tier entity must have a body that contains "Components:"
/// (the rollup prompt header) and at least one "- " line (child summary).
#[test]
fn test_rollup_tier_bodies_look_like_rollup_prompts() {
    for entity in SAMPLE.iter().filter(|e| e.tier == SampleTier::Rollup) {
        assert!(
            entity.body.contains("Components:"),
            "rollup-tier entity {} body missing 'Components:' header",
            entity.id
        );
        assert!(
            entity.body.contains("- "),
            "rollup-tier entity {} body missing child summary lines",
            entity.id
        );
    }
}
