// Integration tests for RelationshipBuilder::build_cross_repo() (public API)

use lievo::analysis::relationships::RelationshipBuilder;
use lievo::model::{CodeUnit, Entity, EntityTier, RelType};

fn make_entity(id: &str, tier: EntityTier, path: &str, parent_id: Option<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier,
        parent_id: parent_id.map(str::to_string),
        name: path.split('/').next_back().unwrap_or(path).to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_code_unit(file: &str, imports: Vec<&str>, calls: Vec<&str>) -> CodeUnit {
    CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: format!("{}::fn_name", file.replace('/', "::")),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: calls.into_iter().map(str::to_string).collect(),
        imports: imports.into_iter().map(str::to_string).collect(),
    }
}

#[test]
fn test_build_cross_repo_basic() {
    let sub_a = make_entity("proj:repo-a:subsystem:.", EntityTier::Subsystem, ".", None);
    let mut sub_b = make_entity("proj:repo-b:subsystem:.", EntityTier::Subsystem, ".", None);
    sub_b.name = "repo-b".to_string();

    let all_subsystems = vec![
        ("repo-a".to_string(), vec![sub_a]),
        ("repo-b".to_string(), vec![sub_b]),
    ];
    let unit = make_code_unit("src/main.rs", vec!["repo_b::something"], vec![]);
    let all_code_units = vec![
        ("repo-a".to_string(), vec![unit]),
        ("repo-b".to_string(), vec![]),
    ];

    let rels = RelationshipBuilder::build_cross_repo(&all_subsystems, &all_code_units).unwrap();

    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].source_id, "proj:repo-a:subsystem:.");
    assert_eq!(rels[0].target_id, "proj:repo-b:subsystem:.");
    assert_eq!(rels[0].rel_type, RelType::DependsOn);
}

#[test]
fn test_build_cross_repo_no_self_deps() {
    let mut sub = make_entity("proj:repo-a:subsystem:.", EntityTier::Subsystem, ".", None);
    sub.name = "repo-a".to_string();
    let all_subsystems = vec![("repo-a".to_string(), vec![sub])];
    let unit = make_code_unit("src/main.rs", vec!["repo_a::something"], vec![]);
    let all_code_units = vec![("repo-a".to_string(), vec![unit])];

    let rels = RelationshipBuilder::build_cross_repo(&all_subsystems, &all_code_units).unwrap();
    assert!(rels.is_empty());
}

#[test]
fn test_build_cross_repo_std_collections_no_false_positive() {
    // Repo B subsystem named "collections"; Repo A imports "std::collections::HashMap".
    // First segment is "std", not "collections" → no cross-repo edge.
    let sub_a = make_entity("proj:repo-a:subsystem:.", EntityTier::Subsystem, ".", None);
    let mut sub_b = make_entity("proj:repo-b:subsystem:.", EntityTier::Subsystem, ".", None);
    sub_b.name = "collections".to_string();

    let all_subsystems = vec![
        ("repo-a".to_string(), vec![sub_a]),
        ("repo-b".to_string(), vec![sub_b]),
    ];
    let unit = make_code_unit("src/main.rs", vec!["std::collections::HashMap"], vec![]);
    let all_code_units = vec![
        ("repo-a".to_string(), vec![unit]),
        ("repo-b".to_string(), vec![]),
    ];

    let rels = RelationshipBuilder::build_cross_repo(&all_subsystems, &all_code_units).unwrap();
    assert!(
        rels.is_empty(),
        "Expected no cross-repo edges, but got: {:?}",
        rels
    );
}

#[test]
fn test_build_cross_repo_only_importing_subsystem_gets_edge() {
    // Repo A: 3 subsystems; only sub_a1 imports repo-b. Verify exactly 1 edge (not 3).
    let sub_a1 = make_entity(
        "proj:repo-a:subsystem:crates/a1",
        EntityTier::Subsystem,
        "crates/a1",
        None,
    );
    let sub_a2 = make_entity(
        "proj:repo-a:subsystem:crates/a2",
        EntityTier::Subsystem,
        "crates/a2",
        None,
    );
    let sub_a3 = make_entity(
        "proj:repo-a:subsystem:crates/a3",
        EntityTier::Subsystem,
        "crates/a3",
        None,
    );
    let mut sub_b1 = make_entity("proj:repo-b:subsystem:.", EntityTier::Subsystem, ".", None);
    sub_b1.name = "repo-b".to_string();
    let all_subsystems = vec![
        ("repo-a".to_string(), vec![sub_a1, sub_a2, sub_a3]),
        ("repo-b".to_string(), vec![sub_b1]),
    ];
    let unit_a1 = make_code_unit("crates/a1/src/lib.rs", vec!["repo_b::something"], vec![]);
    let unit_a2 = make_code_unit("crates/a2/src/lib.rs", vec!["serde::Serialize"], vec![]);
    let unit_a3 = make_code_unit("crates/a3/src/lib.rs", vec!["tokio::runtime"], vec![]);
    let all_code_units = vec![
        ("repo-a".to_string(), vec![unit_a1, unit_a2, unit_a3]),
        ("repo-b".to_string(), vec![]),
    ];
    let rels = RelationshipBuilder::build_cross_repo(&all_subsystems, &all_code_units).unwrap();
    let cross: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();
    assert_eq!(
        cross.len(),
        1,
        "Expected 1 cross-repo edge, got: {:?}",
        cross
    );
    assert_eq!(cross[0].source_id, "proj:repo-a:subsystem:crates/a1");
    assert_eq!(cross[0].target_id, "proj:repo-b:subsystem:.");
}

#[test]
fn test_build_cross_repo_path_prefix_no_false_match() {
    // "crates/a10/lib.rs" must NOT match subsystem "crates/a1" (prefix without separator).
    let sub_a1 = make_entity(
        "proj:repo-a:subsystem:crates/a1",
        EntityTier::Subsystem,
        "crates/a1",
        None,
    );
    let sub_a10 = make_entity(
        "proj:repo-a:subsystem:crates/a10",
        EntityTier::Subsystem,
        "crates/a10",
        None,
    );
    let mut sub_b = make_entity("proj:repo-b:subsystem:.", EntityTier::Subsystem, ".", None);
    sub_b.name = "repo-b".to_string();

    let all_subsystems = vec![
        ("repo-a".to_string(), vec![sub_a1, sub_a10]),
        ("repo-b".to_string(), vec![sub_b]),
    ];
    // File lives under crates/a10 and imports repo-b
    let unit = make_code_unit("crates/a10/lib.rs", vec!["repo_b::Foo"], vec![]);
    let all_code_units = vec![
        ("repo-a".to_string(), vec![unit]),
        ("repo-b".to_string(), vec![]),
    ];

    let rels = RelationshipBuilder::build_cross_repo(&all_subsystems, &all_code_units).unwrap();
    let cross: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert_eq!(
        cross.len(),
        1,
        "Expected exactly 1 cross-repo edge, got: {:?}",
        cross
    );
    assert_eq!(
        cross[0].source_id, "proj:repo-a:subsystem:crates/a10",
        "Edge must originate from crates/a10, not crates/a1"
    );
    assert_eq!(cross[0].target_id, "proj:repo-b:subsystem:.");
}
