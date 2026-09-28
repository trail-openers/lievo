// Integration tests for src/output.rs formatting functions.
// Tests the public API using realistic entity and relationship data.

use lievo::model::{Entity, EntityTier, RelType, Relationship};
use lievo::output::{
    OutputFormat, ResolutionContext, format_entities_human, format_entities_json,
    format_impact_human, format_impact_json, format_relationships_human, format_relationships_json,
};
use lievo::query::dependency::{ImpactReport, ResolutionSignal};

fn make_entity(id: &str, name: &str, tier: EntityTier, metrics: Option<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier,
        parent_id: None,
        name: name.to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: metrics.map(|s| s.to_string()),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

// ---------------------------------------------------------------------------
// OutputFormat
// ---------------------------------------------------------------------------

#[test]
fn test_output_format_from_str_human() {
    use clap::ValueEnum;
    let fmt = OutputFormat::from_str("human", true).expect("human is valid");
    assert_eq!(fmt, OutputFormat::Human);
}

#[test]
fn test_output_format_from_str_json() {
    use clap::ValueEnum;
    let fmt = OutputFormat::from_str("json", true).expect("json is valid");
    assert_eq!(fmt, OutputFormat::Json);
}

#[test]
fn test_output_format_from_str_invalid_returns_error() {
    use clap::ValueEnum;
    let result = OutputFormat::from_str("xml", true);
    assert!(result.is_err(), "invalid format should return Err");
}

// ---------------------------------------------------------------------------
// format_entities_human — subsystem format
// ---------------------------------------------------------------------------

#[test]
fn test_format_entities_human_subsystem_empty() {
    let result = format_entities_human(&[], EntityTier::Subsystem);
    assert_eq!(result, "No subsystems found.");
}

#[test]
fn test_format_entities_human_module_empty() {
    let result = format_entities_human(&[], EntityTier::Module);
    assert_eq!(result, "No modules found.");
}

#[test]
fn test_format_entities_human_file_empty() {
    let result = format_entities_human(&[], EntityTier::File);
    assert_eq!(result, "No files found.");
}

#[test]
fn test_format_entities_human_subsystem_has_header_and_name() {
    let e = make_entity(
        "s1",
        "my-subsystem",
        EntityTier::Subsystem,
        Some(r#"{"module_count":3,"file_count":12,"complexity_max":42}"#),
    );
    let result = format_entities_human(&[e], EntityTier::Subsystem);
    assert!(result.contains("SUBSYSTEMS"), "missing SUBSYSTEMS header");
    assert!(result.contains("Name"), "missing Name column");
    assert!(result.contains("Modules"), "missing Modules column");
    assert!(result.contains("Files"), "missing Files column");
    assert!(result.contains("Complexity"), "missing Complexity column");
    assert!(result.contains("my-subsystem"), "missing entity name");
}

#[test]
fn test_format_entities_human_subsystem_metrics_rendered() {
    let e = make_entity(
        "s1",
        "my-sub",
        EntityTier::Subsystem,
        Some(r#"{"module_count":3,"file_count":12,"complexity_max":42.5}"#),
    );
    let result = format_entities_human(&[e], EntityTier::Subsystem);
    assert!(result.contains("3"), "module count missing");
    assert!(result.contains("12"), "file count missing");
    assert!(result.contains("42.5"), "complexity missing");
}

#[test]
fn test_format_entities_human_module_has_header() {
    let e = make_entity(
        "m1",
        "my-module",
        EntityTier::Module,
        Some(r#"{"file_count":5,"complexity_max":10}"#),
    );
    let result = format_entities_human(&[e], EntityTier::Module);
    assert!(result.contains("MODULES"), "missing MODULES header");
    assert!(result.contains("my-module"), "missing name");
    assert!(result.contains("5"), "file count missing");
}

#[test]
fn test_format_entities_human_file_shows_language() {
    let mut e = make_entity("f1", "main.rs", EntityTier::File, None);
    e.language = Some("Rust".to_string());
    let result = format_entities_human(&[e], EntityTier::File);
    assert!(result.contains("FILES"), "missing FILES header");
    assert!(result.contains("main.rs"), "missing name");
    assert!(result.contains("Rust"), "missing language");
}

// ---------------------------------------------------------------------------
// format_entities_json
// ---------------------------------------------------------------------------

fn write_entities_json(entities: &[Entity]) -> String {
    let mut buf = Vec::new();
    format_entities_json(entities, &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn test_format_entities_json_empty_returns_empty_string() {
    let result = write_entities_json(&[]);
    assert_eq!(result.trim(), "");
}

#[test]
fn test_format_entities_json_single_entity_is_valid_json() {
    let e = make_entity(
        "s1",
        "my-subsystem",
        EntityTier::Subsystem,
        Some(r#"{"module_count":3,"file_count":12,"complexity_max":42}"#),
    );
    let result = write_entities_json(&[e]);
    let parsed: serde_json::Value =
        serde_json::from_str(result.trim()).expect("must be valid JSON");
    assert_eq!(parsed["name"], "my-subsystem");
    assert_eq!(parsed["tier"], "subsystem");
    assert_eq!(parsed["module_count"], 3);
    assert_eq!(parsed["file_count"], 12);
    assert_eq!(parsed["complexity_max"], 42.0);
}

#[test]
fn test_format_entities_json_multiple_entities_ndjson() {
    let entities = vec![
        make_entity("s1", "sub-a", EntityTier::Subsystem, None),
        make_entity("s2", "sub-b", EntityTier::Subsystem, None),
    ];
    let result = write_entities_json(&entities);
    let lines: Vec<&str> = result.trim().lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 NDJSON lines");
    for line in lines {
        serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
    }
}

#[test]
fn test_format_entities_json_includes_id_and_name() {
    let e = make_entity("entity-42", "cool-name", EntityTier::Module, None);
    let result = write_entities_json(&[e]);
    let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
    assert_eq!(parsed["id"], "entity-42");
    assert_eq!(parsed["name"], "cool-name");
}

// ---------------------------------------------------------------------------
// format_relationships_human
// ---------------------------------------------------------------------------

#[test]
fn test_format_relationships_human_empty_shows_none() {
    let result =
        format_relationships_human(&[], "Dependencies of X", &ResolutionContext::unknown());
    assert!(result.contains("Dependencies of X"));
    assert!(result.contains("(none)"));
}

#[test]
fn test_format_relationships_human_shows_name_type_weight() {
    let entity = make_entity("e1", "target-module", EntityTier::Module, None);
    let rel = Relationship {
        source_id: "src".to_string(),
        target_id: "e1".to_string(),
        rel_type: RelType::DependsOn,
        weight: 2.5,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    };
    let result = format_relationships_human(
        &[(rel, entity)],
        "Dependencies of src",
        &ResolutionContext::unknown(),
    );
    assert!(result.contains("target-module"), "name missing");
    assert!(result.contains("depends_on"), "rel type missing");
    assert!(result.contains("2.5"), "weight missing");
}

// ---------------------------------------------------------------------------
// format_relationships_json
// ---------------------------------------------------------------------------

fn write_relationships_json(rels: &[(Relationship, Entity)]) -> String {
    let mut buf = Vec::new();
    format_relationships_json(rels, &ResolutionContext::unknown(), &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn test_format_relationships_json_empty_returns_empty_array() {
    let result = write_relationships_json(&[]);
    assert_eq!(result.trim(), "[]");
    // Verify it's valid JSON
    let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 0);
}

#[test]
fn test_format_relationships_json_single_is_valid_json() {
    let entity = make_entity("e1", "target-mod", EntityTier::Module, None);
    let rel = Relationship {
        source_id: "src".to_string(),
        target_id: "e1".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: lievo::model::EdgeProvenance::Heuristic,
    };
    let result = write_relationships_json(&[(rel, entity)]);
    let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
    assert_eq!(parsed["entity_id"], "e1");
    assert_eq!(parsed["entity_name"], "target-mod");
    assert_eq!(parsed["rel_type"], "imports");
    assert_eq!(parsed["weight"], 1.0);
}

// ---------------------------------------------------------------------------
// format_impact_human / format_impact_json
// ---------------------------------------------------------------------------

fn make_impact_report() -> ImpactReport {
    ImpactReport {
        changed_files: vec![make_entity("f1", "main.rs", EntityTier::File, None)],
        affected_modules: vec![make_entity("m1", "core", EntityTier::Module, None)],
        affected_subsystems: vec![make_entity("s1", "api", EntityTier::Subsystem, None)],
        downstream_dependents: vec![],
        affected_functions: vec![],
    }
}

#[test]
fn test_format_impact_human_has_all_sections() {
    let report = make_impact_report();
    let signal = ResolutionSignal::from_counts(None);
    let result = format_impact_human(&report, &signal);
    assert!(result.contains("Impact Analysis"));
    assert!(result.contains("Changed files (1)"));
    assert!(result.contains("Affected modules (1)"));
    assert!(result.contains("Affected subsystems (1)"));
    assert!(result.contains("Downstream dependents (0)"));
    assert!(result.contains("Affected callers (0)"));
}

#[test]
fn test_format_impact_human_shows_entity_names() {
    let report = make_impact_report();
    let signal = ResolutionSignal::from_counts(None);
    let result = format_impact_human(&report, &signal);
    assert!(result.contains("main.rs"));
    assert!(result.contains("core"));
    assert!(result.contains("api"));
}

fn write_impact_json(report: &ImpactReport) -> String {
    let signal = ResolutionSignal::from_counts(None);
    let mut buf = Vec::new();
    format_impact_json(report, &signal, &mut buf).unwrap();
    String::from_utf8(buf).unwrap()
}

#[test]
fn test_format_impact_json_produces_five_ndjson_lines() {
    let report = make_impact_report();
    let result = write_impact_json(&report);
    let lines: Vec<&str> = result.trim().lines().collect();
    // 5 section lines + 1 resolution line = 6 lines total.
    // The existing 5 section labels must appear in order; the resolution
    // line is additive (last line) and does not break consumers that
    // filter by section label.
    assert_eq!(
        lines.len(),
        6,
        "expected 5 section lines + 1 resolution line"
    );
    for line in &lines {
        serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
    }
}

#[test]
fn test_format_impact_json_section_labels_in_order() {
    let report = make_impact_report();
    let result = write_impact_json(&report);
    let sections: Vec<String> = result
        .trim()
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["section"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(
        sections,
        vec![
            "changed_files",
            "affected_modules",
            "affected_subsystems",
            "downstream_dependents",
            "affected_callers",
            "resolution"
        ]
    );
}

#[test]
fn test_format_impact_json_entities_included_in_section() {
    let report = make_impact_report();
    let result = write_impact_json(&report);
    let first_line = result.trim().lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let entities = v["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0]["name"], "main.rs");
}
