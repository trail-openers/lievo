// Integration tests for dependency query methods: dependencies_of, dependents_of, impact_analysis.
use lievo::LievoError;
use lievo::model::{Entity, EntityTier, RelType, Relationship};
use lievo::query::dependency;
use lievo::query::dependency::{DependencyResult, ResolutionSignal};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

/// Fixture hierarchy:
///   subsys-a → mod-a → file-a (src/a.rs), file-b (src/b.rs)
///   subsys-b → mod-b → file-c (src/c.rs)
///   subsys-c  (isolated)
///   Relationships: file-a→file-b, mod-a→mod-b
fn build_fixture() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "test-repo", "/tmp/test")
        .unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();

    let base = Entity {
        id: String::new(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: String::new(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now,
    };

    let entities: &[Entity] = &[
        Entity {
            id: "subsys-a".into(),
            tier: EntityTier::Subsystem,
            name: "subsys-a".into(),
            ..base.clone()
        },
        Entity {
            id: "subsys-b".into(),
            tier: EntityTier::Subsystem,
            name: "subsys-b".into(),
            ..base.clone()
        },
        Entity {
            id: "subsys-c".into(),
            tier: EntityTier::Subsystem,
            name: "subsys-c".into(),
            ..base.clone()
        },
        Entity {
            id: "mod-a".into(),
            tier: EntityTier::Module,
            parent_id: Some("subsys-a".into()),
            name: "mod-a".into(),
            path: Some("src/mod-a".into()),
            ..base.clone()
        },
        Entity {
            id: "mod-b".into(),
            tier: EntityTier::Module,
            parent_id: Some("subsys-b".into()),
            name: "mod-b".into(),
            path: Some("src/mod-b".into()),
            ..base.clone()
        },
        Entity {
            id: "file-a".into(),
            parent_id: Some("mod-a".into()),
            name: "a.rs".into(),
            path: Some("src/a.rs".into()),
            ..base.clone()
        },
        Entity {
            id: "file-b".into(),
            parent_id: Some("mod-a".into()),
            name: "b.rs".into(),
            path: Some("src/b.rs".into()),
            ..base.clone()
        },
        Entity {
            id: "file-c".into(),
            parent_id: Some("mod-b".into()),
            name: "c.rs".into(),
            path: Some("src/c.rs".into()),
            ..base.clone()
        },
    ];
    for e in entities {
        storage.upsert_entity(e).unwrap();
    }

    for (src, tgt) in [("file-a", "file-b"), ("mod-a", "mod-b")] {
        storage
            .upsert_relationship(&Relationship {
                source_id: src.into(),
                target_id: tgt.into(),
                rel_type: RelType::DependsOn,
                weight: 1.0,
                evidence_json: None,
                provenance: lievo::model::EdgeProvenance::Heuristic,
            })
            .unwrap();
    }

    (storage, project.id, repo.id)
}

/// Persist repo-wide unresolved-import counts on the repository row's
/// `unresolved_internal`/`unresolved_external` columns — exactly where
/// #856's pipeline writes them. `SqliteStorage` reads them back via
/// `get_unresolved_counts`.
fn seed_unresolved_counts(
    storage: &SqliteStorage,
    _project_id: &str,
    repo_id: &str,
    internal: u64,
    external: u64,
) {
    storage
        .record_unresolved_counts(repo_id, internal, external)
        .unwrap();
}

fn rel(src: &str, tgt: &str) -> Relationship {
    Relationship {
        source_id: src.into(),
        target_id: tgt.into(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    }
}

// ── dependencies_of ───────────────────────────────────────────────────────

#[test]
fn test_dependencies_of_returns_outgoing_relationships_with_entities() {
    let (storage, _, _) = build_fixture();
    let result = dependency::dependencies_of(&storage, "file-a").unwrap();
    assert_eq!(result.len(), 1);
    let (r, entity) = &result[0];
    assert_eq!(r.source_id, "file-a");
    assert_eq!(r.target_id, "file-b");
    assert_eq!(entity.id, "file-b");
}

#[test]
fn test_dependencies_of_entity_with_no_deps_returns_empty() {
    let (storage, _, _) = build_fixture();
    assert!(
        dependency::dependencies_of(&storage, "file-b")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_dependencies_of_nonexistent_entity_returns_entity_not_found() {
    let (storage, _, _) = build_fixture();
    let err = dependency::dependencies_of(&storage, "ghost").unwrap_err();
    assert!(matches!(err, LievoError::EntityNotFound(_)));
}

#[test]
fn test_dependencies_of_result_sorted_by_entity_id() {
    let (storage, _, _) = build_fixture();
    storage
        .upsert_relationship(&rel("mod-a", "subsys-c"))
        .unwrap();
    let result = dependency::dependencies_of(&storage, "mod-a").unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].1.id, "mod-b");
    assert_eq!(result[1].1.id, "subsys-c");
}

// ── dependents_of ─────────────────────────────────────────────────────────

#[test]
fn test_dependents_of_returns_incoming_relationships_with_entities() {
    let (storage, _, _) = build_fixture();
    let result = dependency::dependents_of(&storage, "file-b").unwrap();
    assert_eq!(result.len(), 1);
    let (r, entity) = &result[0];
    assert_eq!(r.source_id, "file-a");
    assert_eq!(r.target_id, "file-b");
    assert_eq!(entity.id, "file-a");
}

#[test]
fn test_dependents_of_entity_with_no_dependents_returns_empty() {
    let (storage, _, _) = build_fixture();
    assert!(
        dependency::dependents_of(&storage, "file-a")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_dependents_of_with_resolution_pre_681_index_is_null() {
    // No unresolved counts are persisted in this fixture (pre-#681 shape):
    // the signal must degrade to None, never a false full-coverage 0.
    let (storage, _, _) = build_fixture();
    let result: DependencyResult =
        dependency::dependents_of_with_resolution(&storage, "file-a").unwrap();
    assert!(result.relationships.is_empty());
    assert_eq!(result.resolution, ResolutionSignal::from_counts(None));
    assert!(!result.resolution.caveat_active());
}

#[test]
fn test_dependents_of_with_resolution_records_unresolved_internal() {
    // 0 dependents but N internal imports unresolved: the signal must fire the
    // caveat so an agent cannot read the empty set as dead code.
    let (storage, project_id, repo_id) = build_fixture();
    seed_unresolved_counts(&storage, &project_id, &repo_id, 4, 2);
    let result: DependencyResult =
        dependency::dependents_of_with_resolution(&storage, "file-a").unwrap();
    assert!(result.relationships.is_empty());
    assert_eq!(result.resolution.unresolved_internal, Some(4));
    assert_eq!(result.resolution.unresolved_external, Some(2));
    assert!(result.resolution.caveat_active());
}

#[test]
fn test_dependents_of_with_resolution_external_only_does_not_fire_caveat() {
    // External (bare-package / non-crate) imports fail resolution by design;
    // they are reported separately but must never drive the caveat.
    let (storage, project_id, repo_id) = build_fixture();
    seed_unresolved_counts(&storage, &project_id, &repo_id, 0, 7);
    let result: DependencyResult =
        dependency::dependents_of_with_resolution(&storage, "file-a").unwrap();
    assert_eq!(result.resolution.unresolved_external, Some(7));
    assert!(!result.resolution.caveat_active());
}

#[test]
fn test_dependents_of_with_resolution_nonzero_dependents_unaffected() {
    // file-b has one dependent (file-a). The resolution signal is still
    // attached; with a recorded unresolved count it fires regardless of the
    // dependent set being non-empty.
    let (storage, project_id, repo_id) = build_fixture();
    seed_unresolved_counts(&storage, &project_id, &repo_id, 3, 0);
    let result: DependencyResult =
        dependency::dependents_of_with_resolution(&storage, "file-b").unwrap();
    assert_eq!(result.relationships.len(), 1);
    assert_eq!(result.resolution.unresolved_internal, Some(3));
    assert!(result.resolution.caveat_active());
}

#[test]
fn test_dependencies_of_with_resolution_attaches_signal() {
    let (storage, _, _) = build_fixture();
    let result: DependencyResult =
        dependency::dependencies_of_with_resolution(&storage, "file-a").unwrap();
    assert_eq!(result.relationships.len(), 1);
    assert_eq!(result.resolution, ResolutionSignal::from_counts(None));
}

#[test]
fn test_dependencies_of_with_resolution_nonexistent_returns_entity_not_found() {
    let (storage, _, _) = build_fixture();
    let err = dependency::dependencies_of_with_resolution(&storage, "ghost").unwrap_err();
    assert!(matches!(err, LievoError::EntityNotFound(_)));
}

#[test]
fn test_dependents_of_with_resolution_nonexistent_returns_entity_not_found() {
    let (storage, _, _) = build_fixture();
    let err = dependency::dependents_of_with_resolution(&storage, "ghost").unwrap_err();
    assert!(matches!(err, LievoError::EntityNotFound(_)));
}

#[test]
fn test_dependents_of_nonexistent_entity_returns_entity_not_found() {
    let (storage, _, _) = build_fixture();
    let err = dependency::dependents_of(&storage, "ghost").unwrap_err();
    assert!(matches!(err, LievoError::EntityNotFound(_)));
}

#[test]
fn test_dependents_of_result_sorted_by_entity_id() {
    let (storage, _, _) = build_fixture();
    storage
        .upsert_relationship(&rel("file-c", "mod-b"))
        .unwrap();
    let result = dependency::dependents_of(&storage, "mod-b").unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].1.id, "file-c");
    assert_eq!(result[1].1.id, "mod-a");
}

// ── impact_analysis ───────────────────────────────────────────────────────

#[test]
fn test_impact_analysis_resolves_changed_files() {
    let (storage, _, repo_id) = build_fixture();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert_eq!(report.changed_files.len(), 1);
    assert_eq!(report.changed_files[0].id, "file-a");
}

#[test]
fn test_impact_analysis_identifies_parent_module() {
    let (storage, _, repo_id) = build_fixture();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert_eq!(report.affected_modules.len(), 1);
    assert_eq!(report.affected_modules[0].id, "mod-a");
}

#[test]
fn test_impact_analysis_identifies_parent_subsystem() {
    let (storage, _, repo_id) = build_fixture();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert_eq!(report.affected_subsystems.len(), 1);
    assert_eq!(report.affected_subsystems[0].id, "subsys-a");
}

#[test]
fn test_impact_analysis_no_inbound_edges_means_empty_downstream() {
    // mod-a has no inbound edges in the fixture
    let (storage, _, repo_id) = build_fixture();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert!(report.downstream_dependents.is_empty());
}

#[test]
fn test_impact_resolution_for_repo_pre_681_is_null() {
    let (storage, _, repo_id) = build_fixture();
    let signal = dependency::impact_resolution_for_repo(&storage, &repo_id);
    assert!(signal.unresolved_internal.is_none());
    assert!(signal.unresolved_external.is_none());
    assert!(!signal.caveat_active());
}

#[test]
fn test_impact_resolution_for_repo_unresolved_internal() {
    let (storage, project_id, repo_id) = build_fixture();
    seed_unresolved_counts(&storage, &project_id, &repo_id, 5, 3);
    let signal = dependency::impact_resolution_for_repo(&storage, &repo_id);
    assert_eq!(signal.unresolved_internal, Some(5));
    assert_eq!(signal.unresolved_external, Some(3));
    assert!(signal.caveat_active());
}

#[test]
fn test_impact_resolution_for_repo_external_only_no_caveat() {
    let (storage, project_id, repo_id) = build_fixture();
    seed_unresolved_counts(&storage, &project_id, &repo_id, 0, 9);
    let signal = dependency::impact_resolution_for_repo(&storage, &repo_id);
    assert_eq!(signal.unresolved_external, Some(9));
    assert!(!signal.caveat_active());
}

#[test]
fn test_impact_resolution_for_repo_unknown_repo_is_null() {
    let (storage, _, _) = build_fixture();
    let signal = dependency::impact_resolution_for_repo(&storage, "ghost-repo");
    assert_eq!(signal, ResolutionSignal::from_counts(None));
}

#[test]
fn test_impact_analysis_collects_downstream_when_inbound_edge_exists() {
    let (storage, _, repo_id) = build_fixture();
    storage
        .upsert_relationship(&rel("file-c", "mod-a"))
        .unwrap();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert_eq!(report.downstream_dependents.len(), 1);
    assert_eq!(report.downstream_dependents[0].id, "file-c");
}

#[test]
fn test_impact_analysis_deduplicates_entities() {
    let (storage, _, repo_id) = build_fixture();
    let report =
        dependency::impact_analysis(&storage, &repo_id, &["src/a.rs", "src/a.rs"]).unwrap();
    assert_eq!(report.changed_files.len(), 1);
    assert_eq!(report.affected_modules.len(), 1);
    assert_eq!(report.affected_subsystems.len(), 1);
}

#[test]
fn test_impact_analysis_multiple_files_merged() {
    let (storage, _, repo_id) = build_fixture();
    let report =
        dependency::impact_analysis(&storage, &repo_id, &["src/a.rs", "src/b.rs"]).unwrap();
    assert_eq!(report.changed_files.len(), 2);
    assert_eq!(report.affected_modules.len(), 1);
    assert_eq!(report.affected_modules[0].id, "mod-a");
    assert_eq!(report.affected_subsystems.len(), 1);
}

#[test]
fn test_impact_analysis_nonexistent_path_returns_entity_not_found() {
    let (storage, _, repo_id) = build_fixture();
    let err = dependency::impact_analysis(&storage, &repo_id, &["src/ghost.rs"]).unwrap_err();
    assert!(matches!(err, LievoError::EntityNotFound(_)));
}

#[test]
fn test_impact_analysis_results_sorted_by_entity_id() {
    let (storage, _, repo_id) = build_fixture();
    // Two entities depend on subsys-a
    storage
        .upsert_relationship(&rel("subsys-b", "subsys-a"))
        .unwrap();
    storage
        .upsert_relationship(&rel("mod-b", "subsys-a"))
        .unwrap();
    let report = dependency::impact_analysis(&storage, &repo_id, &["src/a.rs"]).unwrap();
    assert_eq!(report.downstream_dependents.len(), 2);
    assert_eq!(report.downstream_dependents[0].id, "mod-b");
    assert_eq!(report.downstream_dependents[1].id, "subsys-b");
}

#[test]
fn test_impact_analysis_empty_file_paths() {
    let (storage, _, repo_id) = build_fixture();
    let report = dependency::impact_analysis(&storage, &repo_id, &[]).unwrap();
    assert!(report.changed_files.is_empty());
    assert!(report.affected_modules.is_empty());
    assert!(report.affected_subsystems.is_empty());
    assert!(report.downstream_dependents.is_empty());
}

#[test]
fn test_impact_analysis_file_without_parent() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "test-repo", "/tmp/test")
        .unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();

    // File entity with no parent_id (orphan — not inside any module)
    let orphan = Entity {
        id: "orphan-file".into(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "orphan.rs".into(),
        path: Some("src/orphan.rs".into()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now,
    };
    storage.upsert_entity(&orphan).unwrap();

    let report = dependency::impact_analysis(&storage, &repo.id, &["src/orphan.rs"]).unwrap();

    assert_eq!(report.changed_files.len(), 1);
    assert_eq!(report.changed_files[0].id, "orphan-file");
    assert!(report.affected_modules.is_empty());
    assert!(report.affected_subsystems.is_empty());
    assert!(report.downstream_dependents.is_empty());
}

// ── #764: cross-project boundary regressions ──────────────────────────────

/// Two projects in one database with a genuine cross-project DependsOn edge
/// (foreign module in project B depends on module A in project A).
fn cross_project_fixture() -> (SqliteStorage, String, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pa = storage.create_project("proj-a", None).unwrap();
    let pb = storage.create_project("proj-b", None).unwrap();
    let repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
    let repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();

    let mk =
        |id: &str, project: &str, repo: &str, tier: EntityTier, parent: Option<&&str>| Entity {
            id: id.into(),
            project_id: project.into(),
            repo_id: Some(repo.into()),
            tier,
            parent_id: parent.map(|p| (*p).into()),
            name: id.into(),
            path: Some(format!("src/{id}.rs")),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
    let module_a = mk("pa:mod-a", &pa.id, &repo_a.id, EntityTier::Module, None);
    let file_a = mk(
        "pa:file-a",
        &pa.id,
        &repo_a.id,
        EntityTier::File,
        Some(&"pa:mod-a"),
    );
    let fn_a = mk(
        "pa:fn-a",
        &pa.id,
        &repo_a.id,
        EntityTier::Function,
        Some(&"pa:file-a"),
    );
    let mod_b = mk("pb:mod-b", &pb.id, &repo_b.id, EntityTier::Module, None);
    let file_b = mk(
        "pb:file-b",
        &pb.id,
        &repo_b.id,
        EntityTier::File,
        Some(&"pb:mod-b"),
    );
    let fn_b = mk(
        "pb:fn-b",
        &pb.id,
        &repo_b.id,
        EntityTier::Function,
        Some(&"pb:file-b"),
    );
    for e in [&module_a, &file_a, &fn_a, &mod_b, &file_b, &fn_b] {
        storage.upsert_entity(e).unwrap();
    }
    // Genuine cross-project edges: foreign module depends on project A's
    // module, and foreign function calls project A's function.
    for (src, tgt, rel) in [
        ("pb:mod-b", "pa:mod-a", RelType::DependsOn),
        ("pb:fn-b", "pa:fn-a", RelType::Calls),
    ] {
        storage
            .upsert_relationship(&Relationship {
                source_id: src.into(),
                target_id: tgt.into(),
                rel_type: rel,
                weight: 1.0,
                evidence_json: None,
                provenance: lievo::model::EdgeProvenance::Heuristic,
            })
            .unwrap();
    }
    (storage, pa.id, repo_a.id, repo_b.id)
}

#[test]
fn test_dependencies_of_does_not_leak_across_project_boundaries() {
    let (storage, _pa, _repo_a, repo_b) = cross_project_fixture();
    // Same-project edge in project B: file-b depends on mod-b.
    storage
        .upsert_relationship(&Relationship {
            source_id: "pb:file-b".into(),
            target_id: "pb:mod-b".into(),
            rel_type: RelType::DependsOn,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        })
        .unwrap();

    // Project A: mod-a has an incoming cross-project edge from mod-b;
    // mod-a itself has no same-project outgoing edges, so its dependencies
    // must be empty.
    let deps_a = dependency::dependencies_of(&storage, "pa:mod-a").unwrap();
    assert!(
        deps_a.is_empty(),
        "project A mod-a must have no same-project dependencies: {deps_a:?}"
    );

    // Project B: mod-b's cross-project outgoing edge to mod-a must not leak.
    let deps_b = dependency::dependencies_of(&storage, "pb:mod-b").unwrap();
    assert!(
        !deps_b.iter().any(|(_, e)| e.id == "pa:mod-a"),
        "cross-project edge must not appear in dependencies_of: {deps_b:?}"
    );
    // Same-project dependents must still be returned (file-b is a dependent of mod-b? no — file-b's parent is mod-b via parent_id; its DependsOn edge is file-b→mod-b, so mod-b's dependents include file-b).
    let deps_b2 = dependency::dependencies_of(&storage, "pb:file-b").unwrap();
    assert!(
        deps_b2.iter().any(|(_, e)| e.id == "pb:mod-b"),
        "same-project dependency must still be returned: {deps_b2:?}"
    );
    let _ = repo_b;
}

#[test]
fn test_dependents_of_does_not_leak_across_project_boundaries() {
    let (storage, _pa, _repo_a, _repo_b) = cross_project_fixture();

    // Project A: mod-a has an incoming cross-project edge from mod-b (projB)
    // — it must not leak into mod-a's dependents.
    let dependents_a = dependency::dependents_of(&storage, "pa:mod-a").unwrap();
    assert!(
        !dependents_a.iter().any(|(_, e)| e.id == "pb:mod-b"),
        "cross-project dependent must not leak into project A: {dependents_a:?}"
    );

    // Project B: fn-b (projB) has a cross-project Calls edge to fn-a (projA).
    // fn-a's dependents include fn-b (cross-project) — must not leak.
    let dependents_a_fn = dependency::dependents_of(&storage, "pa:fn-a").unwrap();
    assert!(
        !dependents_a_fn.iter().any(|(_, e)| e.id == "pb:fn-b"),
        "cross-project function caller must not leak: {dependents_a_fn:?}"
    );
}

#[test]
fn test_impact_analysis_does_not_leak_across_project_boundaries() {
    let (storage, _pa, repo_a, repo_b) = cross_project_fixture();

    // Impact of changing project A's file: the cross-project edge from
    // projB's mod-b (DependsOn → pa:mod-a) and from projB's fn-b (Calls →
    // pa:fn-a) must not drag foreign entities into the report.
    let report_a = dependency::impact_analysis(&storage, &repo_a, &["src/pa:file-a.rs"]).unwrap();
    let all_a: Vec<&Entity> = report_a
        .downstream_dependents
        .iter()
        .chain(report_a.affected_functions.iter())
        .collect();
    assert!(
        !all_a
            .iter()
            .any(|e| e.id == "pb:mod-b" || e.id == "pb:fn-b"),
        "cross-project entities must not appear in impact report: {all_a:?}"
    );

    // Symmetric direction: impact of changing project B's file must not
    // surface project A's module or function.
    let report_b = dependency::impact_analysis(&storage, &repo_b, &["src/pb:file-b.rs"]).unwrap();
    let all_b: Vec<&Entity> = report_b
        .downstream_dependents
        .iter()
        .chain(report_b.affected_functions.iter())
        .collect();
    assert!(
        !all_b
            .iter()
            .any(|e| e.id == "pa:mod-a" || e.id == "pa:fn-a"),
        "cross-project entities must not appear in impact report (reverse direction): {all_b:?}"
    );
}
