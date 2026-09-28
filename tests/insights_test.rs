// Integration tests for InsightDetector.
// Kept in tests/ to keep src/analysis/insights.rs under 300 lines.
use lievo::analysis::insights::{InsightDetector, insight_id, is_test_file, severity_order};
use lievo::model::{Entity, EntityTier, RelType, Relationship};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

struct EntityBuilder {
    id: String,
    project_id: String,
    repo_id: String,
    tier: EntityTier,
    parent_id: Option<String>,
    name: String,
    path: Option<String>,
    language: Option<String>,
    metrics_json: Option<String>,
}

impl EntityBuilder {
    fn new(id: &str, project_id: &str, repo_id: &str, tier: EntityTier) -> Self {
        Self {
            id: id.to_string(),
            project_id: project_id.to_string(),
            repo_id: repo_id.to_string(),
            tier,
            parent_id: None,
            name: id.to_string(),
            path: None,
            language: None,
            metrics_json: None,
        }
    }

    fn parent(mut self, parent_id: &str) -> Self {
        self.parent_id = Some(parent_id.to_string());
        self
    }

    fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }

    fn language(mut self, language: &str) -> Self {
        self.language = Some(language.to_string());
        self
    }

    fn metrics(mut self, metrics_json: &str) -> Self {
        self.metrics_json = Some(metrics_json.to_string());
        self
    }

    fn build(self) -> Entity {
        let now = "2024-01-01T00:00:00Z".to_string();
        Entity {
            id: self.id,
            project_id: self.project_id,
            repo_id: Some(self.repo_id),
            tier: self.tier,
            parent_id: self.parent_id,
            name: self.name,
            path: self.path,
            language: self.language,
            summary: None,
            summary_commit: None,
            metrics_json: self.metrics_json,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn make_rel(source: &str, target: &str) -> Relationship {
    Relationship {
        source_id: source.to_string(),
        target_id: target.to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    }
}

fn setup_db() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "test-repo", "/tmp/test")
        .unwrap();
    (storage, project.id, repo.id)
}

// ---- complexity_hotspot tests ----

#[test]
fn test_no_file_entities_returns_no_hotspots() {
    let (storage, project_id, _repo_id) = setup_db();
    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(insights.is_empty());
}

#[test]
fn test_files_with_zero_avg_complexity_return_no_hotspots() {
    let (storage, project_id, repo_id) = setup_db();
    storage
        .upsert_entity(
            &EntityBuilder::new("f1", &project_id, &repo_id, EntityTier::File)
                .name("a.rs")
                .path("src/a.rs")
                .build(),
        )
        .unwrap();
    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert_eq!(
        insights
            .iter()
            .filter(|i| i.category == "complexity_hotspot")
            .count(),
        0
    );
}

#[test]
fn test_file_exceeding_4x_avg_flagged_critical() {
    let (storage, project_id, repo_id) = setup_db();
    // 5 files at complexity 1, 1 file at complexity 12
    // avg = (5*1 + 12)/6 = 17/6 ≈ 2.83; 12/2.83 ≈ 4.24x → critical (> 4.0)
    for i in 0..5 {
        storage
            .upsert_entity(
                &EntityBuilder::new(&format!("low-{i}"), &project_id, &repo_id, EntityTier::File)
                    .name(&format!("low{i}.rs"))
                    .path(&format!("src/low{i}.rs"))
                    .metrics(r#"{"complexity_max": 1}"#)
                    .build(),
            )
            .unwrap();
    }
    storage
        .upsert_entity(
            &EntityBuilder::new("hot", &project_id, &repo_id, EntityTier::File)
                .name("hot.rs")
                .path("src/hot.rs")
                .metrics(r#"{"complexity_max": 12}"#)
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    let hotspot = insights.iter().find(|i| {
        i.category == "complexity_hotspot" && i.entity_ids_json.as_deref() == Some("[\"hot\"]")
    });
    assert!(hotspot.is_some(), "Expected hotspot insight for 'hot'");
    assert_eq!(hotspot.unwrap().severity.as_deref(), Some("critical"));
}

#[test]
fn test_insight_ids_are_deterministic() {
    let (storage, project_id, repo_id) = setup_db();
    for i in 0..5 {
        storage
            .upsert_entity(
                &EntityBuilder::new(&format!("f{i}"), &project_id, &repo_id, EntityTier::File)
                    .name(&format!("f{i}.rs"))
                    .path(&format!("src/f{i}.rs"))
                    .metrics(r#"{"complexity_max": 1}"#)
                    .build(),
            )
            .unwrap();
    }
    storage
        .upsert_entity(
            &EntityBuilder::new("hot", &project_id, &repo_id, EntityTier::File)
                .name("hot.rs")
                .path("src/hot.rs")
                .metrics(r#"{"complexity_max": 12}"#)
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let first = detector.detect().unwrap();
    let second = detector.detect().unwrap();

    let first_ids: Vec<&str> = first.iter().map(|i| i.id.as_str()).collect();
    let second_ids: Vec<&str> = second.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        first_ids, second_ids,
        "Insight IDs must be stable across runs"
    );
}

// ---- high_coupling tests ----

#[test]
fn test_module_below_coupling_threshold_not_flagged() {
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod1", &project_id, &repo_id, EntityTier::Module)
        .name("mod1")
        .path("src/mod1")
        .build();
    storage.upsert_entity(&module).unwrap();
    // Add 4 outgoing relationships (below small-project fallback threshold of 5).
    // With 5 modules total and fallback (5,5,5), fan-out of 4 is below low_threshold=5.
    for i in 0..4 {
        let target = EntityBuilder::new(
            &format!("dep{i}"),
            &project_id,
            &repo_id,
            EntityTier::Module,
        )
        .name(&format!("dep{i}"))
        .build();
        storage.upsert_entity(&target).unwrap();
        storage
            .upsert_relationship(&make_rel("mod1", &format!("dep{i}")))
            .unwrap();
    }

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| {
            i.category == "high_coupling" && i.entity_ids_json.as_deref() == Some("[\"mod1\"]")
        }),
        "Should NOT flag module with 4 outgoing relationships (below threshold of 5)"
    );
}

#[test]
fn test_module_exceeding_fan_out_threshold_flagged() {
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-hub", &project_id, &repo_id, EntityTier::Module)
        .name("hub")
        .path("src/hub")
        .build();
    storage.upsert_entity(&module).unwrap();

    // Add 11 outgoing relationships (> threshold of 10)
    for i in 0..11 {
        let target = EntityBuilder::new(
            &format!("dep{i}"),
            &project_id,
            &repo_id,
            EntityTier::Module,
        )
        .name(&format!("dep{i}"))
        .build();
        storage.upsert_entity(&target).unwrap();
        storage
            .upsert_relationship(&make_rel("mod-hub", &format!("dep{i}")))
            .unwrap();
    }

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    let coupling = insights.iter().find(|i| {
        i.category == "high_coupling" && i.entity_ids_json.as_deref() == Some("[\"mod-hub\"]")
    });
    assert!(coupling.is_some(), "Expected high_coupling insight");
    assert_eq!(coupling.unwrap().severity.as_deref(), Some("high"));
}

// ---- coverage_gap tests ----

#[test]
fn test_module_with_no_test_files_flagged_high() {
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-coverage", &project_id, &repo_id, EntityTier::Module)
        .name("coverage")
        .path("src/coverage")
        .build();
    storage.upsert_entity(&module).unwrap();

    // Add only non-test files (filenames do NOT contain "test" or "spec")
    for i in 0..3 {
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("src-file-{i}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-coverage")
                .name(&format!("main{i}.rs"))
                .path(&format!("src/coverage/main{i}.rs"))
                .language("Rust")
                .build(),
            )
            .unwrap();
    }

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    let gap = insights.iter().find(|i| {
        i.category == "coverage_gap" && i.entity_ids_json.as_deref() == Some("[\"mod-coverage\"]")
    });
    assert!(gap.is_some(), "Expected coverage_gap insight");
    assert_eq!(gap.unwrap().severity.as_deref(), Some("high"));
}

#[test]
fn test_module_with_adequate_test_ratio_not_flagged() {
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-well-tested", &project_id, &repo_id, EntityTier::Module)
        .name("well-tested")
        .path("src/well-tested")
        .build();
    storage.upsert_entity(&module).unwrap();

    // 2 source, 2 test files → 50% ratio → not flagged
    for i in 0..2 {
        storage
            .upsert_entity(
                &EntityBuilder::new(&format!("src-{i}"), &project_id, &repo_id, EntityTier::File)
                    .parent("mod-well-tested")
                    .name(&format!("src{i}.rs"))
                    .path(&format!("src/well-tested/src{i}.rs"))
                    .build(),
            )
            .unwrap();
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("test-{i}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-well-tested")
                .name(&format!("src{i}_test.rs"))
                .path(&format!("src/well-tested/src{i}_test.rs"))
                .build(),
            )
            .unwrap();
    }

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| {
            i.category == "coverage_gap"
                && i.entity_ids_json.as_deref() == Some("[\"mod-well-tested\"]")
        }),
        "Should NOT flag module with 50% test ratio"
    );
}

#[test]
fn test_redetect_marks_old_insights_stale_then_reinserts() {
    let (storage, project_id, repo_id) = setup_db();
    for i in 0..5 {
        storage
            .upsert_entity(
                &EntityBuilder::new(&format!("f{i}"), &project_id, &repo_id, EntityTier::File)
                    .name(&format!("f{i}.rs"))
                    .path(&format!("src/f{i}.rs"))
                    .metrics(r#"{"complexity_max": 1}"#)
                    .build(),
            )
            .unwrap();
    }
    storage
        .upsert_entity(
            &EntityBuilder::new("hot", &project_id, &repo_id, EntityTier::File)
                .name("hot.rs")
                .path("src/hot.rs")
                .metrics(r#"{"complexity_max": 12}"#)
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    detector.detect().unwrap();

    // Re-detect: returned insights should all have still_valid = true
    let second_run = detector.detect().unwrap();
    assert!(
        second_run.iter().all(|i| i.still_valid),
        "Re-detected insights must be still_valid"
    );

    // Verify in storage: all stored insights for this project are still_valid
    let stored = storage.list_insights(&project_id, None, None, 100).unwrap();
    assert!(
        stored.iter().all(|i| i.still_valid),
        "All insights in storage must be still_valid after re-detection"
    );
}

#[test]
fn test_list_insights_filters_out_invalid_insights() {
    // Regression test for issue #559: list_insights SQL must filter still_valid = 1.
    let (storage, project_id, repo_id) = setup_db();

    // Create one valid entity for detection
    storage
        .upsert_entity(
            &EntityBuilder::new("hot", &project_id, &repo_id, EntityTier::File)
                .name("hot.rs")
                .path("src/hot.rs")
                .metrics(r#"{"complexity_max": 12}"#)
                .build(),
        )
        .unwrap();

    // First detect: produces an insight (still_valid = true by default)
    let detector = InsightDetector::new(&storage, &project_id);
    detector.detect().unwrap();

    // Manually insert a stale insight directly (simulating an insight from a previous
    // run that was invalidated but not re-detected in the latest run).
    let stale_insight = lievo::model::Insight {
        id: "stale-1".to_string(),
        project_id: project_id.clone(),
        category: "complexity_hotspot".to_string(),
        severity: Some("high".to_string()),
        title: "Stale hotspot".to_string(),
        description: Some("This should be filtered out".to_string()),
        entity_ids_json: Some(r#"["hot"]"#.to_string()),
        detected_at: "2024-01-01T00:00:00Z".to_string(),
        still_valid: false, // intentionally invalid
    };
    storage.upsert_insight(&stale_insight).unwrap();

    // The stale insight must NOT appear in list_insights results
    let listed = storage.list_insights(&project_id, None, None, 100).unwrap();
    assert!(
        listed.iter().all(|i| i.still_valid),
        "list_insights must not return insights with still_valid = false"
    );
    assert!(
        !listed.iter().any(|i| i.id == "stale-1"),
        "Stale insight with id 'stale-1' must not be returned"
    );
}

// ---- helper unit tests ----

#[test]
fn test_is_test_file_detects_test_in_filename() {
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("user_test.rs")
        .path("src/user_test.rs")
        .build();
    assert!(is_test_file(&entity));
}

#[test]
fn test_is_test_file_ignores_test_in_directory_name() {
    // A directory named "notest" should not trigger — only the filename matters
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("main.rs")
        .path("src/notest/main.rs")
        .build();
    assert!(!is_test_file(&entity));
}

#[test]
fn test_is_test_file_detects_spec_in_filename() {
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("user_spec.rb")
        .path("spec/user_spec.rb")
        .build();
    assert!(is_test_file(&entity));
}

#[test]
fn test_is_test_file_rejects_normal_file() {
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("main.rs")
        .path("src/main.rs")
        .build();
    assert!(!is_test_file(&entity));
}

#[test]
fn test_is_test_file_rejects_false_positives() {
    // Filenames containing "test" as a substring (not a word boundary) must NOT match.
    // Old implementation using .contains("test") would produce false positives on these.
    for name in &["latest.rs", "contest.rs", "attestation.rs", "protest.rs"] {
        let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
            .name(name)
            .path(&format!("src/{name}"))
            .build();
        assert!(
            !is_test_file(&entity),
            "is_test_file returned true for {name} — false positive"
        );
    }
}

#[test]
fn test_is_test_file_detects_file_in_tests_directory() {
    // A plain file inside a `tests/` directory is a test file even without a test-naming pattern.
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("integration.rs")
        .path("tests/integration.rs")
        .build();
    assert!(is_test_file(&entity));
}

#[test]
fn test_is_test_file_detects_file_in_nested_tests_directory() {
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("helpers.rs")
        .path("src/commands/tests/helpers.rs")
        .build();
    assert!(is_test_file(&entity));
}

#[test]
fn test_is_test_file_detects_file_in_spec_directory() {
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("user.rb")
        .path("spec/models/user.rb")
        .build();
    assert!(is_test_file(&entity));
}

#[test]
fn test_is_test_file_rejects_file_in_notest_directory() {
    // "notest" does not contain "/test/" (only "test" as a substring), so must not match.
    let entity = EntityBuilder::new("e", "p", "r", EntityTier::File)
        .name("main.rs")
        .path("src/notest/main.rs")
        .build();
    assert!(!is_test_file(&entity));
}

// ---- cross-directory test attribution tests ----

#[test]
fn test_cross_directory_test_clears_coverage_gap_by_module_name() {
    // Module "commands" has 3 source files but no direct test children.
    // A project-wide test file `tests/commands_test.rs` should be attributed
    // to the module, clearing the coverage gap.
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-commands", &project_id, &repo_id, EntityTier::Module)
        .name("commands")
        .path("src/commands")
        .build();
    storage.upsert_entity(&module).unwrap();

    // Source files under the module (no test names)
    for i in 0..3 {
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("cmd-src-{i}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-commands")
                .name(&format!("handler{i}.rs"))
                .path(&format!("src/commands/handler{i}.rs"))
                .build(),
            )
            .unwrap();
    }

    // A root-grouped test file whose stem contains the module name "commands"
    storage
        .upsert_entity(
            &EntityBuilder::new("test-commands", &project_id, &repo_id, EntityTier::File)
                .name("commands_test.rs")
                .path("tests/commands_test.rs")
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| {
            i.category == "coverage_gap"
                && i.entity_ids_json.as_deref() == Some("[\"mod-commands\"]")
        }),
        "coverage gap should be cleared by cross-directory test file"
    );
}

#[test]
fn test_cross_directory_test_clears_coverage_gap_by_child_stem() {
    // Module "extraction" has a child file "grouping.rs".
    // A project-wide test file `tests/grouping_integration_test.rs` should be attributed
    // to the module via child-stem matching.
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-extraction", &project_id, &repo_id, EntityTier::Module)
        .name("extraction")
        .path("src/extraction")
        .build();
    storage.upsert_entity(&module).unwrap();

    // Source files; use a short module name to exercise child-stem matching instead
    for name in &["grouping.rs", "grouping_filter.rs", "entity_id.rs"] {
        let stem = name.trim_end_matches(".rs");
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("ext-{stem}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-extraction")
                .name(name)
                .path(&format!("src/extraction/{name}"))
                .build(),
            )
            .unwrap();
    }

    // Integration test whose stem contains child stem "grouping"
    storage
        .upsert_entity(
            &EntityBuilder::new(
                "test-grouping-integ",
                &project_id,
                &repo_id,
                EntityTier::File,
            )
            .name("grouping_integration_test.rs")
            .path("tests/grouping_integration_test.rs")
            .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| {
            i.category == "coverage_gap"
                && i.entity_ids_json.as_deref() == Some("[\"mod-extraction\"]")
        }),
        "coverage gap should be cleared by cross-directory test matching child stem"
    );
}

#[test]
fn test_cross_directory_test_does_not_double_count_direct_children() {
    // A module where the test file IS a direct child must not be counted twice.
    // 2 source + 1 direct-child test = 33% ratio → not flagged (above 10% threshold).
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-mixed", &project_id, &repo_id, EntityTier::Module)
        .name("mixed")
        .path("src/mixed")
        .build();
    storage.upsert_entity(&module).unwrap();

    for i in 0..2 {
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("mixed-src-{i}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-mixed")
                .name(&format!("impl{i}.rs"))
                .path(&format!("src/mixed/impl{i}.rs"))
                .build(),
            )
            .unwrap();
    }
    // Direct child that IS a test file
    storage
        .upsert_entity(
            &EntityBuilder::new("mixed-test-direct", &project_id, &repo_id, EntityTier::File)
                .parent("mod-mixed")
                .name("mixed_test.rs")
                .path("src/mixed/mixed_test.rs")
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| {
            i.category == "coverage_gap" && i.entity_ids_json.as_deref() == Some("[\"mod-mixed\"]")
        }),
        "module with direct test child must not be flagged"
    );
}

#[test]
fn test_cross_directory_test_substring_does_not_match_partial_module_name() {
    // Module named "lite" (4 chars) must NOT be attributed sqlite_tests.rs because
    // "lite" is a substring of "sqlite" but not a whole word in that stem.
    let (storage, project_id, repo_id) = setup_db();
    let module = EntityBuilder::new("mod-lite", &project_id, &repo_id, EntityTier::Module)
        .name("lite")
        .path("src/lite")
        .build();
    storage.upsert_entity(&module).unwrap();

    // Source files under the module
    for i in 0..3 {
        storage
            .upsert_entity(
                &EntityBuilder::new(
                    &format!("lite-src-{i}"),
                    &project_id,
                    &repo_id,
                    EntityTier::File,
                )
                .parent("mod-lite")
                .name(&format!("handler{i}.rs"))
                .path(&format!("src/lite/handler{i}.rs"))
                .language("Rust")
                .build(),
            )
            .unwrap();
    }

    // sqlite_tests.rs: "lite" is a substring of "sqlite_tests" but NOT a whole word
    storage
        .upsert_entity(
            &EntityBuilder::new("test-sqlite", &project_id, &repo_id, EntityTier::File)
                .name("sqlite_tests.rs")
                .path("tests/sqlite_tests.rs")
                .build(),
        )
        .unwrap();

    let detector = InsightDetector::new(&storage, &project_id);
    let insights = detector.detect().unwrap();
    assert!(
        insights.iter().any(|i| {
            i.category == "coverage_gap" && i.entity_ids_json.as_deref() == Some("[\"mod-lite\"]")
        }),
        "module 'lite' must NOT have sqlite_tests.rs attributed to it (substring false-positive)"
    );
}

#[test]
fn test_severity_order_sorts_correctly() {
    assert!(severity_order(Some("critical")) < severity_order(Some("high")));
    assert!(severity_order(Some("high")) < severity_order(Some("medium")));
    assert!(severity_order(Some("medium")) < severity_order(Some("low")));
    assert!(severity_order(Some("low")) < severity_order(None));
}

#[test]
fn test_insight_id_format() {
    let id = insight_id("proj-123", "complexity_hotspot", "entity-456");
    assert_eq!(id, "proj-123:insight:complexity_hotspot:entity-456");
}

// ---- StubStorage-based unit tests (no SQLite, deterministic) ----
//
// These tests exercise InsightDetector logic in isolation using an in-memory
// stub. They were formerly unit tests inside src/analysis/insights.rs;
// moved here to keep that file under 500 lines.

use std::cell::RefCell;

struct StubStorage {
    entities: Vec<Entity>,
    relationships_from: Vec<(Relationship, Entity)>,
    relationships_to: Vec<(Relationship, Entity)>,
    upserted: RefCell<Vec<lievo::model::Insight>>,
}

impl StubStorage {
    fn new(entities: Vec<Entity>) -> Self {
        Self {
            entities,
            relationships_from: vec![],
            relationships_to: vec![],
            upserted: RefCell::new(vec![]),
        }
    }

    fn with_relationships(
        mut self,
        from: Vec<(Relationship, Entity)>,
        to: Vec<(Relationship, Entity)>,
    ) -> Self {
        self.relationships_from = from;
        self.relationships_to = to;
        self
    }
}

fn stub_entity(id: &str, tier: EntityTier) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier,
        parent_id: None,
        name: id.to_string(),
        path: Some(format!("src/{id}.rs")),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn stub_rel(source: &str, target: &str, rel_type: RelType) -> (Relationship, Entity) {
    let rel = Relationship {
        source_id: source.to_string(),
        target_id: target.to_string(),
        rel_type,
        weight: 1.0,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    };
    let entity = stub_entity(target, EntityTier::File);
    (rel, entity)
}

impl Storage for StubStorage {
    fn list_insights(
        &self,
        _project_id: &str,
        _category: Option<&str>,
        _severity: Option<&str>,
        _limit: usize,
    ) -> lievo::Result<Vec<lievo::model::Insight>> {
        Ok(self
            .upserted
            .borrow()
            .iter()
            .filter(|i| i.still_valid)
            .cloned()
            .collect())
    }
    fn list_entities(
        &self,
        _project_id: &str,
        tier: Option<EntityTier>,
    ) -> lievo::Result<Vec<Entity>> {
        Ok(match tier {
            Some(t) => self
                .entities
                .iter()
                .filter(|e| e.tier == t)
                .cloned()
                .collect(),
            None => self.entities.clone(),
        })
    }

    fn entities_by_parent(&self, parent_id: &str) -> lievo::Result<Vec<Entity>> {
        Ok(self
            .entities
            .iter()
            .filter(|e| e.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect())
    }

    fn search_entities_by_name(
        &self,
        _: &str,
        _: &[&str],
        _: usize,
        _: Option<&str>,
    ) -> lievo::Result<Vec<Entity>> {
        Ok(vec![])
    }

    fn relationships_from(&self, source_id: &str) -> lievo::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .relationships_from
            .iter()
            .filter(|(rel, _)| rel.source_id == source_id)
            .cloned()
            .collect())
    }

    fn relationships_to(&self, target_id: &str) -> lievo::Result<Vec<(Relationship, Entity)>> {
        Ok(self
            .relationships_to
            .iter()
            .filter(|(rel, _)| rel.target_id == target_id)
            .cloned()
            .collect())
    }

    fn upsert_insight(&self, insight: &lievo::model::Insight) -> lievo::Result<()> {
        let mut upserted = self.upserted.borrow_mut();
        // Remove existing insight with same ID (upsert semantics)
        upserted.retain(|i| i.id != insight.id);
        upserted.push(insight.clone());
        Ok(())
    }

    fn invalidate_insights(&self, _project_id: &str) -> lievo::Result<()> {
        let mut upserted = self.upserted.borrow_mut();
        for insight in upserted.iter_mut() {
            insight.still_valid = false;
        }
        Ok(())
    }

    // ---- Unneeded stubs ----
    fn create_project(
        &self,
        _name: &str,
        _description: Option<&str>,
    ) -> lievo::Result<lievo::model::Project> {
        unimplemented!()
    }
    fn get_project(&self, _name: &str) -> lievo::Result<Option<lievo::model::Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _id: &str) -> lievo::Result<Option<lievo::model::Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> lievo::Result<Vec<lievo::model::Project>> {
        unimplemented!()
    }
    fn add_repo(
        &self,
        _project_id: &str,
        _name: &str,
        _local_path: &str,
    ) -> lievo::Result<lievo::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _repo_id: &str) -> lievo::Result<Option<lievo::model::Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _project_id: &str) -> lievo::Result<Vec<lievo::model::Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _repo_id: &str, _path: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _repo_id: &str, _commit: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _repo_id: &str, _project_id: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn upsert_entity(&self, _entity: &Entity) -> lievo::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn get_entity(&self, _entity_id: &str) -> lievo::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entities_by_repo(
        &self,
        _repo_id: &str,
        _tier: Option<EntityTier>,
    ) -> lievo::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entity_by_path(&self, _repo_id: &str, _path: &str) -> lievo::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entity_ids_for_paths(
        &self,
        _: &str,
        _: &[&str],
    ) -> lievo::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> lievo::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _repo_id: &str) -> lievo::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(
        &self,
        _repo_id: &str,
        _exclude_paths: &[String],
    ) -> lievo::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _rel: &Relationship) -> lievo::Result<()> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _source_id: &str) -> lievo::Result<u64> {
        unimplemented!()
    }

    fn upsert_convention(&self, _convention: &lievo::model::Convention) -> lievo::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _project_id: &str,
        _category: Option<&str>,
    ) -> lievo::Result<Vec<lievo::model::Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(
        &self,
        _repo_id: &str,
        _commit_hash: &str,
    ) -> lievo::Result<lievo::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _run: &lievo::model::AnalysisRun) -> lievo::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _repo_id: &str, _file_path: &str) -> lievo::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(
        &self,
        _repo_id: &str,
        _file_path: &str,
        _content_hash: &str,
    ) -> lievo::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _entities: &[&Entity],
        _relationships: &[Relationship],
        _run: &lievo::model::AnalysisRun,
        _repo_id: &str,
        _last_commit: &str,
    ) -> lievo::Result<(i64, i64)> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> lievo::Result<lievo::storage::DeleteStats> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _: &str,
        _: &str,
        _: &std::path::Path,
    ) -> lievo::Result<lievo::storage::reconcile::ReconcileStats> {
        Ok(lievo::storage::reconcile::ReconcileStats::default())
    }
    fn clear_all_summaries(&self, _: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _: &str) -> lievo::Result<()> {
        unimplemented!()
    }
    fn add_output_dir(&self, _: &str, _: &str) -> lievo::Result<()> {
        Ok(())
    }
    fn get_output_dirs(&self, _: &str) -> lievo::Result<Vec<String>> {
        Ok(vec![])
    }
    fn delete_repo(&self, _: &str) -> lievo::Result<lievo::storage::DeleteStats> {
        Ok(lievo::storage::DeleteStats::default())
    }
    fn get_all_file_hashes(
        &self,
        _repo_id: &str,
    ) -> lievo::Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }
    fn delete_file_hash(&self, _repo_id: &str, _file_path: &str) -> lievo::Result<()> {
        Ok(())
    }
    fn count_missing_summaries(&self, _repo_id: &str) -> lievo::Result<u64> {
        Ok(0)
    }
}

#[test]
fn test_stub_contains_only_module_not_flagged_as_high_coupling() {
    // A module with 15 Contains relationships but 0 imports must NOT be flagged.
    let module = stub_entity("mod-root", EntityTier::Module);
    let storage = StubStorage::new(vec![module]).with_relationships(
        (0..15)
            .map(|i| stub_rel("mod-root", &format!("child-{i}"), RelType::Contains))
            .collect(),
        (0..15)
            .map(|i| stub_rel(&format!("parent-{i}"), "mod-root", RelType::Contains))
            .collect(),
    );

    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    let coupling: Vec<_> = insights
        .iter()
        .filter(|i| i.category == "high_coupling")
        .collect();

    assert!(
        coupling.is_empty(),
        "module with only Contains relationships must not be flagged for high coupling"
    );
}

#[test]
fn test_stub_contains_filtered_but_imports_still_trigger_coupling() {
    // A module with 5 Contains + 11 Imports fan-out should be flagged.
    let module = stub_entity("busy-mod", EntityTier::Module);
    let mut from_rels: Vec<(Relationship, Entity)> = (0..5)
        .map(|i| stub_rel("busy-mod", &format!("child-{i}"), RelType::Contains))
        .collect();
    from_rels.extend((0..11).map(|i| stub_rel("busy-mod", &format!("dep-{i}"), RelType::Imports)));

    let storage = StubStorage::new(vec![module]).with_relationships(from_rels, vec![]);

    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    let coupling: Vec<_> = insights
        .iter()
        .filter(|i| i.category == "high_coupling")
        .collect();

    assert_eq!(
        coupling.len(),
        1,
        "module with 11 imports should be flagged"
    );
    assert!(
        coupling[0]
            .description
            .as_deref()
            .unwrap()
            .contains("fan-out=11")
    );
}

#[test]
fn test_stub_complexity_hotspot_low_threshold_is_inclusive() {
    // ratio == 1.5 should produce a "low" severity insight (>= 1.5).
    // Avg = 2.0, so a file with complexity 3.0 gives ratio exactly 1.5.
    let mut e1 = stub_entity("file-a", EntityTier::File);
    e1.metrics_json = Some(r#"{"complexity_max": 1.0}"#.to_string());
    let mut e2 = stub_entity("file-b", EntityTier::File);
    e2.metrics_json = Some(r#"{"complexity_max": 3.0}"#.to_string());

    let storage = StubStorage::new(vec![e1, e2]);
    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    let hotspots: Vec<_> = insights
        .iter()
        .filter(|i| i.category == "complexity_hotspot")
        .collect();

    let b_insight = hotspots.iter().find(|i| i.title.contains("file-b"));
    assert!(
        b_insight.is_some(),
        "file at exactly 1.5x avg should produce a 'low' insight"
    );
    assert_eq!(b_insight.unwrap().severity.as_deref(), Some("low"));
}

#[test]
fn test_stub_no_entities_returns_empty() {
    // Verifies that with no entities, detect returns an empty Vec (no panic).
    let storage = StubStorage::new(vec![]);
    let detector = InsightDetector::new(&storage, "proj");
    let result = detector.detect();
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_percentile_thresholds_with_10_or_more_modules() {
    // With 10+ modules, the p95/p90 percentile path is used.
    // Give every module exactly 4 fan-out (uniform distribution):
    //   - fan_outs sorted: [4,4,4,4,4,4,4,4,4,4]
    //   - p95_idx = 9, fan_outs[9] = 4 → high_fo = max(4,3) = 4
    //   - fan_ins sorted: [0,...,0] (no incoming) → high_fi = max(0,3) = 3
    //   - high_threshold = min(4, 3) = 3
    //   - max_edges = 4, which IS >= 3 → all modules get flagged
    // Verify that the percentile path is actually taken (no fallback) and flags correctly.
    let modules: Vec<Entity> = (0..10)
        .map(|i| stub_entity(&format!("mod-{i}"), EntityTier::Module))
        .collect();

    // All 10 modules get 4 outgoing imports each
    let from_rels: Vec<(Relationship, Entity)> = (0..10)
        .flat_map(|i| {
            (0..4).map(move |j| {
                stub_rel(
                    &format!("mod-{i}"),
                    &format!("ext-{i}-{j}"),
                    RelType::Imports,
                )
            })
        })
        .collect();

    let storage = StubStorage::new(modules).with_relationships(from_rels, vec![]);
    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    let coupling: Vec<_> = insights
        .iter()
        .filter(|i| i.category == "high_coupling")
        .collect();

    // All 10 modules have fan-out=4, which is >= threshold=3 → all should be flagged
    assert_eq!(
        coupling.len(),
        10,
        "with 10+ modules and uniform fan-out=4 >= min threshold=3, all must be flagged"
    );
}

#[test]
fn test_percentile_description_contains_rank_and_total() {
    // With 10 modules where one dominates, description must include percentile context.
    let modules: Vec<Entity> = (0..10)
        .map(|i| stub_entity(&format!("mod-{i}"), EntityTier::Module))
        .collect();

    let hub = &modules[0];
    // Give mod-0 11 fan-out. With all others having 0:
    //   p95 = sorted[9] = 11 → high_fo = max(11,3) = 11
    //   fan_ins all 0 → high_fi = max(0,3) = 3
    //   high_threshold = min(11,3) = 3 → 11 >= 3 → flagged.
    let from_rels: Vec<(Relationship, Entity)> = (0..11)
        .map(|i| stub_rel(&hub.id, &format!("ext-{i}"), RelType::Imports))
        .collect();

    let storage = StubStorage::new(modules).with_relationships(from_rels, vec![]);
    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    let coupling: Vec<_> = insights
        .iter()
        .filter(|i| i.category == "high_coupling")
        .collect();

    assert_eq!(
        coupling.len(),
        1,
        "module with 11 fan-out should be flagged"
    );
    let desc = coupling[0].description.as_deref().unwrap();
    assert!(
        desc.contains("rank") && desc.contains("of 10"),
        "description must include rank and total module count, got: {desc}"
    );
    assert!(
        desc.contains("top") && desc.contains('%'),
        "description must include percentile context, got: {desc}"
    );
}

#[test]
fn test_small_project_fallback_below_10_modules() {
    // With fewer than 10 modules, fixed fallback threshold of 5 is used.
    // A module with 4 fan-out (< 5) must NOT be flagged.
    // (With >= semantics, exactly 5 IS flagged; 4 is below and not flagged.)
    let modules: Vec<Entity> = (0..5)
        .map(|i| stub_entity(&format!("mod-{i}"), EntityTier::Module))
        .collect();
    let hub = &modules[0];
    let from_rels: Vec<(Relationship, Entity)> = (0..4)
        .map(|i| stub_rel(&hub.id, &format!("ext-{i}"), RelType::Imports))
        .collect();

    let storage = StubStorage::new(modules).with_relationships(from_rels, vec![]);
    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| i.category == "high_coupling"
            && i.entity_ids_json.as_deref() == Some("[\"mod-0\"]")),
        "module with 4 fan-out in small project (below threshold 5) must not be flagged"
    );
}

#[test]
fn test_min_absolute_threshold_of_3_respected() {
    // Even in a 10+ module project, if p95 would be 1, the minimum threshold of 3 is used.
    // Set up 10 modules all with fan-in/fan-out of 1 → p95 = 1 → max(1,3) = 3 threshold.
    // A module with fan-out=2 (< 3) must NOT be flagged; fan-out=3 (>= 3) IS flagged.
    let modules: Vec<Entity> = (0..10)
        .map(|i| stub_entity(&format!("mod-{i}"), EntityTier::Module))
        .collect();

    // Each module gets 1 outgoing import except mod-0 which gets 2 (below min threshold of 3)
    let mut from_rels: Vec<(Relationship, Entity)> = (1..10)
        .map(|i| stub_rel(&format!("mod-{i}"), "some-dep", RelType::Imports))
        .collect();
    from_rels.push(stub_rel("mod-0", "dep-a", RelType::Imports));
    from_rels.push(stub_rel("mod-0", "dep-b", RelType::Imports));

    let storage = StubStorage::new(modules).with_relationships(from_rels, vec![]);
    let detector = InsightDetector::new(&storage, "proj");
    let insights = detector.detect().unwrap();
    assert!(
        !insights.iter().any(|i| i.category == "high_coupling"
            && i.entity_ids_json.as_deref() == Some("[\"mod-0\"]")),
        "module with fan-out=2 (below MIN_COUPLING_THRESHOLD=3) must not be flagged"
    );
}
