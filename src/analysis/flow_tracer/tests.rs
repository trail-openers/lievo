// Flow tracer tests

use super::*;
use crate::model::EdgeProvenance;

fn make_entity(id: &str, name: &str) -> Entity {
    make_entity_with_tier(id, name, EntityTier::Function)
}

fn make_entity_with_tier(id: &str, name: &str, tier: EntityTier) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj-1".to_string(),
        repo_id: None,
        tier,
        parent_id: None,
        name: name.to_string(),
        path: None,
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

fn make_depends_on(source: &str, target: &str) -> Relationship {
    Relationship {
        source_id: source.to_string(),
        target_id: target.to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::default(),
    }
}

fn make_calls(source: &str, target: &str) -> Relationship {
    Relationship {
        source_id: source.to_string(),
        target_id: target.to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::default(),
    }
}

#[test]
fn test_trace_all_callees_in_diamond_pattern() {
    // A→[B,C], B→[D], C→[D] — all 4 nodes should be visited
    // Note: has_cycle=true because D is reached via two different paths (revisiting)
    let entities = [
        make_entity("a", "A"),
        make_entity("b", "B"),
        make_entity("c", "C"),
        make_entity("d", "D"),
    ];
    let relationships = [
        make_depends_on("a", "b"),
        make_depends_on("a", "c"),
        make_depends_on("b", "d"),
        make_depends_on("c", "d"),
    ];

    // A has no incoming edges, so it's the entry point
    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "a");
    // D is reached via two different paths (B→D and C→D), so revisiting sets has_cycle=true
    assert!(
        flow.has_cycle,
        "Reaching same node via multiple paths triggers cycle flag"
    );

    // Collect all visited entity IDs
    let visited_ids: Vec<&str> = flow.steps.iter().map(|s| s.entity_id.as_str()).collect();
    assert!(visited_ids.contains(&"a"), "A should be visited");
    assert!(visited_ids.contains(&"b"), "B should be visited");
    assert!(visited_ids.contains(&"c"), "C should be visited");
    assert!(visited_ids.contains(&"d"), "D should be visited");
    assert_eq!(
        visited_ids.len(),
        4,
        "All 4 nodes should be visited exactly once"
    );
}

#[test]
fn test_cycle_detection_true_cycle() {
    // X→A→B→A forms a cycle; X has no incoming edges, so it's the entry point
    let entities = [
        make_entity("x", "X"),
        make_entity("a", "A"),
        make_entity("b", "B"),
    ];
    let relationships = [
        make_depends_on("x", "a"),
        make_depends_on("a", "b"),
        make_depends_on("b", "a"),
    ];

    // X has no incoming edges, so it's the entry point
    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "x");
    assert!(flow.has_cycle, "True cycle A→B→A should be detected");

    // Should not hang or infinitely loop
    assert!(flow.steps.len() <= 5);
}

#[test]
fn test_single_chain_no_cycle() {
    // A→B→C with no cycles
    let entities = [
        make_entity("a", "A"),
        make_entity("b", "B"),
        make_entity("c", "C"),
    ];
    let relationships = [make_depends_on("a", "b"), make_depends_on("b", "c")];

    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "a");
    assert!(!flow.has_cycle);

    let visited_ids: Vec<&str> = flow.steps.iter().map(|s| s.entity_id.as_str()).collect();
    assert_eq!(visited_ids, vec!["a", "b", "c"]);
}

#[test]
fn test_calls_chain_depth_gt_zero() {
    // entry_fn → middle_fn → leaf_fn using Calls relationships
    let entities = [
        make_entity("entry_fn", "entry_fn"),
        make_entity("middle_fn", "middle_fn"),
        make_entity("leaf_fn", "leaf_fn"),
    ];
    let relationships = [
        make_calls("entry_fn", "middle_fn"),
        make_calls("middle_fn", "leaf_fn"),
    ];

    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    // entry_fn has no incoming edges, so it's the entry point
    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "entry_fn");
    assert!(!flow.has_cycle);

    // Should have depth >= 2 (entry_fn at 0, middle_fn at 1, leaf_fn at 2)
    assert!(
        flow.steps.len() >= 3,
        "Should visit all 3 functions in call chain"
    );

    let visited_ids: Vec<&str> = flow.steps.iter().map(|s| s.entity_id.as_str()).collect();
    assert!(visited_ids.contains(&"entry_fn"));
    assert!(visited_ids.contains(&"middle_fn"));
    assert!(visited_ids.contains(&"leaf_fn"));

    // Verify depth increases along the chain
    let depths: Vec<usize> = flow.steps.iter().map(|s| s.depth).collect();
    let max_depth = *depths.iter().max().unwrap_or(&0);
    assert!(
        max_depth >= 2,
        "Max depth should be at least 2 for a 3-function chain"
    );
}

#[test]
fn test_calls_and_depends_on_mixed() {
    // Mix of DependsOn and Calls relationships
    // handler (Calls) -> service_fn (DependsOn) -> db
    // handler (DependsOn) -> utils (Calls) -> helper
    // Verify both Calls-only and DependsOn-only paths work
    let entities = [
        make_entity("handler", "handler"),
        make_entity("service_fn", "service_fn"),
        make_entity("db", "db"),
        make_entity("utils", "utils"),
        make_entity("helper", "helper"),
    ];
    let relationships = [
        make_calls("handler", "service_fn"),
        make_depends_on("service_fn", "db"),
        make_depends_on("handler", "utils"),
        make_calls("utils", "helper"),
    ];

    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    // handler has no incoming edges, so it's the entry point
    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "handler");

    // Should visit all 5 entities via both Calls and DependsOn paths
    assert!(
        flow.steps.len() >= 5,
        "Should visit all entities via mixed Calls and DependsOn"
    );

    let visited_ids: Vec<&str> = flow.steps.iter().map(|s| s.entity_id.as_str()).collect();
    assert!(visited_ids.contains(&"handler"));
    assert!(visited_ids.contains(&"service_fn"));
    assert!(visited_ids.contains(&"db"));
    assert!(visited_ids.contains(&"utils"));
    assert!(visited_ids.contains(&"helper"));

    // Verify helper is reachable (only via Calls edge through utils)
    assert!(
        visited_ids.contains(&"helper"),
        "helper should be reachable via Calls edge"
    );
}

#[test]
fn test_depends_on_file_chain_still_works() {
    // file_a (DependsOn) -> file_b (DependsOn) -> file_c
    // Should produce a flow with depth >= 2
    let entities = [
        make_entity_with_tier("file-a", "file-a", EntityTier::File),
        make_entity_with_tier("file-b", "file-b", EntityTier::File),
        make_entity_with_tier("file-c", "file-c", EntityTier::File),
    ];
    let relationships = [
        make_depends_on("file-a", "file-b"),
        make_depends_on("file-b", "file-c"),
    ];

    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    assert_eq!(flows.len(), 1);
    let flow = &flows[0];
    assert_eq!(flow.entry_point_id, "file-a");
    assert!(!flow.has_cycle);

    // Should have depth >= 2 (file-a at 0, file-b at 1, file-c at 2)
    assert!(
        flow.steps.len() >= 3,
        "Should visit all 3 files in DependsOn chain"
    );

    let visited_ids: Vec<&str> = flow.steps.iter().map(|s| s.entity_id.as_str()).collect();
    assert!(visited_ids.contains(&"file-a"));
    assert!(visited_ids.contains(&"file-b"));
    assert!(visited_ids.contains(&"file-c"));

    // Verify depth increases along the chain
    let depths: Vec<usize> = flow.steps.iter().map(|s| s.depth).collect();
    let max_depth = *depths.iter().max().unwrap_or(&0);
    assert!(
        max_depth >= 2,
        "Max depth should be at least 2 for a 3-file DependsOn chain"
    );
}

#[test]
fn test_calls_edge_between_non_functions_is_dropped() {
    // File entity calling a Function entity via RelType::Calls
    // The Calls edge is ignored (only DependsOn applies for cross-tier calls)
    // Result: no valid edges → no entry points → no flows
    let entities = [
        make_entity_with_tier("file-a", "file-a", EntityTier::File),
        make_entity_with_tier("fn-b", "fn-b", EntityTier::Function),
    ];
    let relationships = [make_calls("file-a", "fn-b")];

    let flows = FlowTracer::trace_flows(&entities, &relationships, None);

    // The Calls edge was dropped (source is File tier, not Function tier)
    // No valid edges remain → no entry points detected → no flows
    assert_eq!(
        flows.len(),
        0,
        "No flows should be produced when the only edge is a cross-tier Calls edge"
    );
}
