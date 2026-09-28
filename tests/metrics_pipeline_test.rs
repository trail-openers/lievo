// Metrics computation and aggregation tests, extracted from analysis_pipeline_test.rs
// to keep that file under the 500-line limit.
use lievo::analysis::metrics::{EntityMetrics, MetricsComputer};
use lievo::model::{CodeUnit, Entity, EntityTier, RelType, Relationship};

fn make_code_unit_for_metrics(file: &str, line: i64, end_line: i64, complexity: i64) -> CodeUnit {
    CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: "mod::fn_name".to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line,
        end_line,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
    }
}

fn make_file_entity_for_metrics(id: &str, path: &str, parent_id: Option<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: parent_id.map(|s| s.to_string()),
        name: path.to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

fn make_module_entity_for_metrics(id: &str, name: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::Module,
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

fn make_rel(source: &str, target: &str, rel_type: RelType) -> Relationship {
    Relationship {
        source_id: source.to_string(),
        target_id: target.to_string(),
        rel_type,
        weight: 1.0,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    }
}

#[test]
fn test_file_metrics_are_serialized_to_entities() {
    let units = vec![
        make_code_unit_for_metrics("src/lib.rs", 1, 20, 3),
        make_code_unit_for_metrics("src/lib.rs", 21, 30, 2),
        make_code_unit_for_metrics("src/main.rs", 1, 10, 1),
    ];

    let file_metrics = MetricsComputer::compute_file_metrics(&units);
    assert_eq!(file_metrics.len(), 2);

    let mut entities = vec![
        make_file_entity_for_metrics("file-lib", "src/lib.rs", Some("mod-1")),
        make_file_entity_for_metrics("file-main", "src/main.rs", Some("mod-1")),
    ];

    for entity in entities.iter_mut() {
        if entity.tier == EntityTier::File
            && let Some(ref path) = entity.path
            && let Some(metrics) = file_metrics.get(path)
        {
            entity.metrics_json = Some(serde_json::to_string(metrics).expect("serialize metrics"));
        }
    }

    for entity in &entities {
        assert!(
            entity.metrics_json.is_some(),
            "metrics_json should be set for file entity {}",
            entity.id
        );
    }

    let lib_entity = entities.iter().find(|e| e.id == "file-lib").unwrap();
    let metrics: EntityMetrics = serde_json::from_str(lib_entity.metrics_json.as_deref().unwrap())
        .expect("deserialize metrics");
    assert_eq!(metrics.code_units, 2);
    assert_eq!(metrics.complexity_sum, 5);
    assert_eq!(metrics.complexity_max, 3);

    let main_entity = entities.iter().find(|e| e.id == "file-main").unwrap();
    let metrics: EntityMetrics = serde_json::from_str(main_entity.metrics_json.as_deref().unwrap())
        .expect("deserialize metrics");
    assert_eq!(metrics.code_units, 1);
    assert_eq!(metrics.complexity_sum, 1);
}

/// Verifies that coupling runs before aggregation: module metrics reflect file fan_in/fan_out.
// NOTE: This test exercises the metrics computation and aggregation logic
// directly rather than through AnalysisPipeline::run_pipeline_steps(), because
// the pipeline requires a real semantic index. The ordering invariant (coupling
// before aggregation) is enforced by the step comments in pipeline.rs.
// A full integration test with a real repo is in tests/fixture_tests.rs.
#[test]
fn test_coupling_before_aggregation_propagates_to_module() {
    let units = vec![
        make_code_unit_for_metrics("src/a.rs", 1, 10, 2),
        make_code_unit_for_metrics("src/b.rs", 1, 10, 1),
    ];
    let file_metrics = MetricsComputer::compute_file_metrics(&units);

    let mut all_entities: Vec<Entity> = vec![
        make_module_entity_for_metrics("mod-1", "src"),
        make_file_entity_for_metrics("file-a", "src/a.rs", Some("mod-1")),
        make_file_entity_for_metrics("file-b", "src/b.rs", Some("mod-1")),
    ];

    // file-a imports file-b → file-a fan_out=1, file-b fan_in=1
    let relationships = vec![make_rel("file-a", "file-b", RelType::Imports)];

    // Step 10a: file metrics → entities
    for entity in all_entities.iter_mut() {
        if entity.tier == EntityTier::File
            && let Some(ref path) = entity.path
            && let Some(metrics) = file_metrics.get(path)
        {
            entity.metrics_json = Some(serde_json::to_string(metrics).expect("serialize metrics"));
        }
    }

    // Step 10b: coupling BEFORE aggregation
    let file_ids: Vec<String> = all_entities
        .iter()
        .filter(|e| e.tier == EntityTier::File)
        .map(|e| e.id.clone())
        .collect();
    for file_id in &file_ids {
        let (fan_in, fan_out) = MetricsComputer::compute_coupling(file_id, &relationships);
        if let Some(entity) = all_entities.iter_mut().find(|e| e.id == *file_id)
            && let Some(ref json) = entity.metrics_json
            && let Ok(mut metrics) = serde_json::from_str::<EntityMetrics>(json)
        {
            metrics.fan_in = fan_in;
            metrics.fan_out = fan_out;
            entity.metrics_json = Some(serde_json::to_string(&metrics).expect("serialize metrics"));
        }
    }

    // Verify file-level coupling
    let fa = all_entities.iter().find(|e| e.id == "file-a").unwrap();
    let fa_m: EntityMetrics =
        serde_json::from_str(fa.metrics_json.as_deref().unwrap()).expect("deserialize metrics");
    assert_eq!(fa_m.fan_out, 1, "file-a should have fan_out=1");
    assert_eq!(fa_m.fan_in, 0, "file-a should have fan_in=0");

    let fb = all_entities.iter().find(|e| e.id == "file-b").unwrap();
    let fb_m: EntityMetrics =
        serde_json::from_str(fb.metrics_json.as_deref().unwrap()).expect("deserialize metrics");
    assert_eq!(fb_m.fan_in, 1, "file-b should have fan_in=1");
    assert_eq!(fb_m.fan_out, 0, "file-b should have fan_out=0");

    // Step 10c: aggregate module metrics (must see coupling from 10b)
    let module_ids: Vec<String> = all_entities
        .iter()
        .filter(|e| e.tier == EntityTier::Module)
        .map(|e| e.id.clone())
        .collect();
    for module_id in &module_ids {
        let child_metrics: Vec<EntityMetrics> = all_entities
            .iter()
            .filter(|e| e.parent_id.as_deref() == Some(module_id))
            .filter_map(|e| {
                e.metrics_json.as_deref().map(|j| {
                    serde_json::from_str::<EntityMetrics>(j).expect("deserialize metrics in test")
                })
            })
            .collect();
        let refs: Vec<&EntityMetrics> = child_metrics.iter().collect();
        if !refs.is_empty() {
            let agg = MetricsComputer::aggregate_module_metrics(&refs);
            if let Some(entity) = all_entities.iter_mut().find(|e| e.id == *module_id) {
                entity.metrics_json = Some(serde_json::to_string(&agg).expect("serialize metrics"));
            }
        }
    }

    // Module rolls up coupling: fan_in=1 + fan_out=1
    let module = all_entities.iter().find(|e| e.id == "mod-1").unwrap();
    assert!(
        module.metrics_json.is_some(),
        "module entity should have aggregated metrics"
    );
    let mod_m: EntityMetrics =
        serde_json::from_str(module.metrics_json.as_deref().unwrap()).expect("deserialize metrics");
    assert_eq!(
        mod_m.fan_in, 1,
        "module fan_in should aggregate from child files"
    );
    assert_eq!(
        mod_m.fan_out, 1,
        "module fan_out should aggregate from child files"
    );
}
