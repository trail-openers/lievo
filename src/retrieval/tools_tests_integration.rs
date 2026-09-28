// Integration and helper tests for tools (issue #119).
//
// Split from tools_tests.rs for file size management.

use serde_json::json;

use crate::model::{EdgeProvenance, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::storage::{Storage, sqlite::SqliteStorage};

// In the test module: super = tools_impl, super::super = tools
use super::super::{ListRelationshipsTool, create_tools};
use super::tools_tests_helpers::{make_ctx, setup_storage, test_entity};

#[test]
fn test_list_relationships_returns_both_directions() {
    // Check from e1 perspective: depends_on e2
    let (storage, project_id) = setup_storage();
    let rel = Relationship {
        source_id: "e1".to_string(),
        target_id: "e2".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage
        .upsert_entity(&test_entity("e1", "source_mod", None, &project_id))
        .unwrap();
    storage
        .upsert_entity(&test_entity("e2", "target_mod", None, &project_id))
        .unwrap();
    storage.upsert_relationship(&rel).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };

    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["imports"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["imports"][0]["entity_id"], "e2");
    assert_eq!(parsed["depended_by"].as_array().unwrap().len(), 0);

    // Check from e2 perspective: depended_by e1
    let (storage2, project_id2) = setup_storage();
    let rel2 = Relationship {
        source_id: "e1".to_string(),
        target_id: "e2".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage2
        .upsert_entity(&test_entity("e1", "source_mod", None, &project_id2))
        .unwrap();
    storage2
        .upsert_entity(&test_entity("e2", "target_mod", None, &project_id2))
        .unwrap();
    storage2.upsert_relationship(&rel2).unwrap();

    let ctx2 = make_ctx(storage2, project_id2);
    let tool2 = ListRelationshipsTool { ctx: ctx2 };
    let result2 = tool2.call(json!({"entity_id": "e2"})).unwrap();
    let parsed2: serde_json::Value = serde_json::from_str(&result2).unwrap();
    assert_eq!(parsed2["depended_by"].as_array().unwrap().len(), 1);
    assert_eq!(parsed2["depended_by"][0]["entity_id"], "e1");
}

#[test]
fn test_list_relationships_missing_entity_id() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };

    assert!(tool.call(json!({})).is_err());
}

#[test]
fn test_create_tools_returns_twelve() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tools = create_tools(ctx);
    assert_eq!(tools.len(), 15); // 12 + get_impact + get_hotspots + explore (#680)
}

#[test]
fn test_tool_names_are_stable() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tools = create_tools(ctx);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"search_entities"));
    assert!(names.contains(&"get_entity"));
    assert!(names.contains(&"list_relationships"));
    assert!(names.contains(&"list_subsystems"));
    assert!(names.contains(&"get_conventions"));
    assert!(names.contains(&"get_insights"));
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"list_directory"));
    assert!(names.contains(&"get_function"));
    assert!(names.contains(&"get_execution_flows"));
    assert!(names.contains(&"list_project_docs"));
    assert!(names.contains(&"read_project_doc"));
    assert!(names.contains(&"get_impact"));
    assert!(names.contains(&"get_hotspots"));
    assert!(names.contains(&"explore"));
}

// --- #690: list_relationships additive unresolved_imports / resolution_coverage ---

/// Record the repo-wide unresolved-import counts on the repository row's
/// columns (#856).
fn seed_unresolved_marker(
    storage: &SqliteStorage,
    _project_id: &str,
    repo_id: &str,
    _repo_name: &str,
    internal: u64,
    external: u64,
) {
    storage
        .record_unresolved_counts(repo_id, internal, external)
        .unwrap();
}

fn entity_in_repo(id: &str, name: &str, project_id: &str, repo_id: &str) -> crate::model::Entity {
    let mut e = test_entity(id, name, Some(name), project_id);
    e.repo_id = Some(repo_id.to_string());
    e
}

#[test]
fn test_list_relationships_entity_without_repo_null_never_zero_or_full() {
    // test_entity leaves repo_id = None -> signal must be null (never a
    // project-wide aggregate, never a false full-coverage claim).
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity("e1", "source_mod", None, &project_id))
        .unwrap();
    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["unresolved_imports"].is_null());
    assert!(parsed["resolution_coverage"].is_null());
    // Existing keys remain intact.
    for key in ["imports", "children", "depended_by"] {
        assert!(parsed.get(key).is_some(), "missing key {key}");
    }
}

#[test]
fn test_list_relationships_pre681_repo_null() {
    let (storage, project_id) = setup_storage();
    let repo = storage.add_repo(&project_id, "myrepo", "/path").unwrap();
    storage
        .upsert_entity(&entity_in_repo("e1", "main.rs", &project_id, &repo.id))
        .unwrap();
    // No unresolved-count marker seeded: pre-#681 shape.
    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(
        parsed["unresolved_imports"].is_null(),
        "pre-#681: unresolved_imports must be null, not 0"
    );
    assert!(
        parsed["resolution_coverage"].is_null(),
        "pre-#681: resolution_coverage must be null, not 1.0"
    );
}

#[test]
fn test_list_relationships_empty_depended_by_with_unresolved_imports_reports_count() {
    // The critical safety path: entity with zero dependents but unresolved
    // internal imports in scope — the response must carry the count so an
    // agent cannot read empty `depended_by` as dead code.
    let (storage, project_id) = setup_storage();
    let repo = storage.add_repo(&project_id, "myrepo", "/path").unwrap();
    storage
        .upsert_entity(&entity_in_repo("e1", "main.rs", &project_id, &repo.id))
        .unwrap();
    seed_unresolved_marker(&storage, &project_id, &repo.id, "myrepo", 4, 9);

    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["depended_by"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["unresolved_imports"], 4);
    assert_eq!(parsed["resolution_coverage"], 0.0);
}

#[test]
fn test_list_relationships_zero_unresolved_full_coverage() {
    let (storage, project_id) = setup_storage();
    let repo = storage.add_repo(&project_id, "myrepo", "/path").unwrap();
    storage
        .upsert_entity(&entity_in_repo("e1", "main.rs", &project_id, &repo.id))
        .unwrap();
    seed_unresolved_marker(&storage, &project_id, &repo.id, "myrepo", 0, 2);

    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["unresolved_imports"], 0);
    assert_eq!(parsed["resolution_coverage"], 1.0);
}

// --- #714: list_relationships additive provenance field ---

#[test]
fn test_list_relationships_edges_carry_provenance_field() {
    // The additive `provenance` field must appear on every edge in the
    // response with a valid value ("resolved" or "heuristic"), and both
    // values must round-trip through storage (v8 column) unchanged.
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity("e1", "source", None, &project_id))
        .unwrap();
    storage
        .upsert_entity(&test_entity("e2", "target-a", None, &project_id))
        .unwrap();
    storage
        .upsert_entity(&test_entity("e3", "target-b", None, &project_id))
        .unwrap();

    let rel1 = Relationship {
        source_id: "e1".to_string(),
        target_id: "e2".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Resolved,
    };
    let rel2 = Relationship {
        source_id: "e1".to_string(),
        target_id: "e3".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage.upsert_relationship(&rel1).unwrap();
    storage.upsert_relationship(&rel2).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let imports = parsed["imports"].as_array().unwrap();
    assert_eq!(imports.len(), 2, "expected 2 imports, got: {:?}", imports);

    // Every edge carries the field with a valid value.
    let mut values = std::collections::HashSet::new();
    for edge in imports {
        let prov = edge
            .get("provenance")
            .expect("edge missing provenance field")
            .as_str()
            .expect("provenance must be a string");
        assert!(
            prov == "resolved" || prov == "heuristic",
            "provenance must be resolved or heuristic, got: {prov}"
        );
        values.insert(prov.to_string());
    }
    // Both values must survive the storage round-trip.
    assert!(
        values
            == ["resolved".to_string(), "heuristic".to_string()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
        "both resolved and heuristic values must appear, got: {:?}",
        values
    );

    // Existing #690 fields remain intact.
    for key in ["unresolved_imports", "resolution_coverage"] {
        assert!(parsed.get(key).is_some(), "missing key {key}");
    }
}

#[test]
fn test_list_relationships_depended_by_carries_provenance() {
    // The provenance field must appear on depended_by edges too.
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity("e1", "target", None, &project_id))
        .unwrap();
    storage
        .upsert_entity(&test_entity("e2", "source", None, &project_id))
        .unwrap();

    let rel = Relationship {
        source_id: "e2".to_string(),
        target_id: "e1".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Resolved,
    };
    storage.upsert_relationship(&rel).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = ListRelationshipsTool { ctx };
    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let depended_by = parsed["depended_by"].as_array().unwrap();
    assert_eq!(depended_by.len(), 1);
    let prov = depended_by[0]
        .get("provenance")
        .expect("missing provenance")
        .as_str()
        .expect("provenance must be a string");
    assert_eq!(prov, "resolved");
}
