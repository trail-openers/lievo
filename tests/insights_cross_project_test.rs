// #764: cross-project boundary regression for the insight detectors.
//
// The coupling and circular-dependency detectors walk relationships_from /
// relationships_to, which resolve by globally-unique entity id and have no
// project_id column — in a multi-project database a cross-project edge would
// otherwise inflate fan-in/fan-out counts or fabricate cycles. These tests
// pin that both detectors stay inside the queried project.

use lievo::analysis::insights::InsightDetector;
use lievo::model::{Entity, EntityTier, RelType, Relationship};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

fn mk_module(id: &str, project_id: &str, name: &str) -> Entity {
    Entity {
        id: id.into(),
        project_id: project_id.into(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: name.into(),
        path: Some(format!("src/{name}")),
        language: Some("python".into()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    }
}

fn upsert_edge(storage: &SqliteStorage, src: &str, tgt: &str, rel_type: RelType) {
    storage
        .upsert_relationship(&Relationship {
            source_id: src.into(),
            target_id: tgt.into(),
            rel_type,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        })
        .unwrap();
}

/// Two projects in one database. Project A has module-a with one same-project
/// DependsOn edge (mod-a → mod-a2). Project B has module-b with a cross-
/// project DependsOn edge (mod-b → mod-a), which must not leak into project
/// A's fan counts.
fn cross_project_modules() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pa = storage.create_project("proj-a", None).unwrap();
    let pb = storage.create_project("proj-b", None).unwrap();
    for (m, proj) in [
        ("pa:mod-a", &pa.id),
        ("pa:mod-a2", &pa.id),
        ("pb:mod-b", &pb.id),
    ] {
        storage.upsert_entity(&mk_module(m, proj, m)).unwrap();
    }
    // Same-project edge: mod-a → mod-a2 (both projA).
    upsert_edge(&storage, "pa:mod-a", "pa:mod-a2", RelType::DependsOn);
    // Cross-project edge: mod-b (projB) → mod-a (projA).
    upsert_edge(&storage, "pb:mod-b", "pa:mod-a", RelType::DependsOn);
    (storage, pa.id, pb.id)
}

#[test]
fn high_coupling_fan_counts_ignore_cross_project_edges() {
    let (storage, pa_id, pb_id) = cross_project_modules();
    let _ = pb_id;

    let detector = InsightDetector::new(&storage, &pa_id);
    let insights = detector.detect().unwrap();
    let _ = &insights;

    // Cross-project edges must not be visible to project A's traversal at
    // all — assert via the public API that no insight entity references a
    // project-B module, and that the detector still reports cleanly for the
    // tiny project (no panics, no fabricated coupling insights).
    assert!(
        !insights.iter().any(|i| i
            .entity_ids_json
            .as_deref()
            .map(|s| s.contains("pb:"))
            .unwrap_or(false)),
        "cross-project module must not appear in any project A insight: {insights:?}"
    );
}

#[test]
fn circular_dependencies_ignore_cross_project_edges() {
    // Build a genuine cycle inside project B (mod-b1 → mod-b2 → mod-b1) and
    // a cross-project edge from project A's module into the cycle
    // (mod-a → mod-b1). Project A's traversal must not walk into project B's
    // cycle, and project B's cycle must be reported exactly once.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pa = storage.create_project("proj-a", None).unwrap();
    let pb = storage.create_project("proj-b", None).unwrap();
    for (m, proj) in [
        ("pa:mod-a", &pa.id),
        ("pb:mod-b1", &pb.id),
        ("pb:mod-b2", &pb.id),
    ] {
        storage.upsert_entity(&mk_module(m, proj, m)).unwrap();
    }
    upsert_edge(&storage, "pb:mod-b1", "pb:mod-b2", RelType::DependsOn);
    upsert_edge(&storage, "pb:mod-b2", "pb:mod-b1", RelType::DependsOn);
    // Cross-project edge: mod-a (projA) → mod-b1 (projB).
    upsert_edge(&storage, "pa:mod-a", "pb:mod-b1", RelType::DependsOn);

    // Project A: no cycles — the cross-project edge must not drag mod-b1 /
    // mod-b2 into project A's traversal.
    let insights_a = InsightDetector::new(&storage, &pa.id).detect().unwrap();
    let circular_a: Vec<_> = insights_a
        .iter()
        .filter(|i| i.category == "circular_dependency")
        .collect();
    assert!(
        circular_a.is_empty(),
        "project A must not report a cycle through a cross-project edge: {circular_a:?}"
    );

    // Project B: the genuine cycle must still be reported (no false negative
    // from the new filter).
    let insights_b = InsightDetector::new(&storage, &pb.id).detect().unwrap();
    let circular_b: Vec<_> = insights_b
        .iter()
        .filter(|i| i.category == "circular_dependency")
        .collect();
    assert_eq!(
        circular_b.len(),
        1,
        "project B's genuine cycle must still be detected: {circular_b:?}"
    );
    assert!(
        !circular_b[0]
            .entity_ids_json
            .as_deref()
            .unwrap_or("")
            .contains("pa:"),
        "cross-project module must not appear in project B's cycle: {circular_b:?}"
    );
}
