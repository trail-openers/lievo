// Integration tests for function-level query helpers (functions_in_file, calls_of_function)

#[cfg(test)]
mod tests {
    use lievo::model::{Entity, EntityTier, RelType, Relationship};
    use lievo::query::entity_queries;
    use lievo::storage::Storage;
    use lievo::storage::sqlite::SqliteStorage;

    fn make_storage_with_file(file_id: &str) -> (SqliteStorage, String) {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let now = "2024-01-01T00:00:00Z".to_string();

        let file = Entity {
            id: file_id.to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "test.rs".to_string(),
            path: Some("src/test.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        storage.upsert_entity(&file).unwrap();
        (storage, project.id)
    }

    #[test]
    fn test_functions_in_file_returns_direct_children() {
        let (storage, project_id) = make_storage_with_file("file-1");
        let now = "2024-01-01T00:00:00Z".to_string();

        let fn1 = Entity {
            id: "fn-1".to_string(),
            project_id,
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: Some("file-1".to_string()),
            name: "validate".to_string(),
            path: Some("src/test.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        storage.upsert_entity(&fn1).unwrap();

        let fn2 = Entity {
            id: "fn-2".to_string(),
            name: "format".to_string(),
            ..fn1.clone()
        };
        storage.upsert_entity(&fn2).unwrap();

        let result = entity_queries::functions_in_file(&storage, "file-1").unwrap();
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"validate"));
        assert!(names.contains(&"format"));
    }

    #[test]
    fn test_functions_in_file_empty_returns_empty() {
        let (storage, _project_id) = make_storage_with_file("file-2");
        let result = entity_queries::functions_in_file(&storage, "file-2").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_calls_of_function_returns_called_functions() {
        let (storage, project_id) = make_storage_with_file("file-1");
        let now = "2024-01-01T00:00:00Z".to_string();

        let fn1 = Entity {
            id: "fn-1".to_string(),
            project_id,
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: Some("file-1".to_string()),
            name: "caller".to_string(),
            path: Some("src/test.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        storage.upsert_entity(&fn1).unwrap();

        let fn2 = Entity {
            id: "fn-2".to_string(),
            name: "callee1".to_string(),
            ..fn1.clone()
        };
        storage.upsert_entity(&fn2).unwrap();

        let fn3 = Entity {
            id: "fn-3".to_string(),
            name: "callee2".to_string(),
            ..fn1.clone()
        };
        storage.upsert_entity(&fn3).unwrap();

        let rel1 = Relationship {
            source_id: "fn-1".to_string(),
            target_id: "fn-2".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel1).unwrap();

        let rel2 = Relationship {
            source_id: "fn-1".to_string(),
            target_id: "fn-3".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel2).unwrap();

        let result = entity_queries::calls_of_function(&storage, "fn-1").unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"fn-2"));
        assert!(ids.contains(&"fn-3"));
    }

    #[test]
    fn test_calls_of_function_empty_returns_empty() {
        let (storage, project_id) = make_storage_with_file("file-1");
        let now = "2024-01-01T00:00:00Z".to_string();

        let fn1 = Entity {
            id: "fn-1".to_string(),
            project_id,
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: Some("file-1".to_string()),
            name: "uncaller".to_string(),
            path: Some("src/test.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now,
        };
        storage.upsert_entity(&fn1).unwrap();

        let result = entity_queries::calls_of_function(&storage, "fn-1").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_calls_of_function_filters_non_function_tiers() {
        let (storage, project_id) = make_storage_with_file("file-1");
        let now = "2024-01-01T00:00:00Z".to_string();

        let fn1 = Entity {
            id: "fn-1".to_string(),
            project_id: project_id.clone(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: Some("file-1".to_string()),
            name: "caller".to_string(),
            path: Some("src/test.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        storage.upsert_entity(&fn1).unwrap();

        // Function target (should be included)
        let fn_target = Entity {
            id: "fn-target".to_string(),
            name: "fn_target".to_string(),
            ..fn1.clone()
        };
        storage.upsert_entity(&fn_target).unwrap();

        // Module target (should be filtered out)
        let module_target = Entity {
            id: "module-target".to_string(),
            name: "module_target".to_string(),
            tier: EntityTier::Module,
            ..fn1.clone()
        };
        storage.upsert_entity(&module_target).unwrap();

        // Calls relationship to function (should be included)
        let rel_fn = Relationship {
            source_id: "fn-1".to_string(),
            target_id: "fn-target".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel_fn).unwrap();

        // Calls relationship to module (should be filtered out)
        let rel_module = Relationship {
            source_id: "fn-1".to_string(),
            target_id: "module-target".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: lievo::model::EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel_module).unwrap();

        let result = entity_queries::calls_of_function(&storage, "fn-1").unwrap();

        // Should only return the function target, not the module
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "fn-target");
        assert_eq!(result[0].tier, EntityTier::Function);
    }

    // --- #764: cross-project boundary regression for calls_of_function ---

    #[test]
    fn test_calls_of_function_does_not_leak_across_project_boundaries() {
        // Two projects share one database. fn-a (projA) calls callee-a
        // (projA, same project — must appear) and callee-b (projB, cross-
        // project — must not appear).
        let storage = SqliteStorage::open_in_memory().unwrap();
        let pa = storage.create_project("proj-a", None).unwrap();
        let pb = storage.create_project("proj-b", None).unwrap();
        let now = "2024-01-01T00:00:00Z".to_string();

        let mk = |id: &str, project_id: &str, name: &str| Entity {
            id: id.into(),
            project_id: project_id.into(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: name.into(),
            path: Some(format!("src/{name}.rs")),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let fn_a = mk("fn-a", &pa.id, "caller_a");
        let callee_a = mk("callee-a", &pa.id, "callee_a");
        let callee_b = mk("callee-b", &pb.id, "callee_b");
        let fn_b = mk("fn-b", &pb.id, "caller_b");
        for e in [&fn_a, &callee_a, &callee_b, &fn_b] {
            storage.upsert_entity(e).unwrap();
        }
        // Same-project edge: fn-a → callee-a (projA).
        storage
            .upsert_relationship(&Relationship {
                source_id: "fn-a".into(),
                target_id: "callee-a".into(),
                rel_type: RelType::Calls,
                weight: 1.0,
                evidence_json: None,
                provenance: lievo::model::EdgeProvenance::Heuristic,
            })
            .unwrap();
        // Cross-project edge: fn-b (projB) → callee-a (projA).
        storage
            .upsert_relationship(&Relationship {
                source_id: "fn-b".into(),
                target_id: "callee-a".into(),
                rel_type: RelType::Calls,
                weight: 1.0,
                evidence_json: None,
                provenance: lievo::model::EdgeProvenance::Heuristic,
            })
            .unwrap();

        // Project A: only the same-project callee must appear.
        let calls_a = entity_queries::calls_of_function(&storage, "fn-a").unwrap();
        assert_eq!(
            calls_a.len(),
            1,
            "only the same-project callee must be returned: {calls_a:?}"
        );
        assert_eq!(calls_a[0].id, "callee-a");

        // Project B: fn-b has only the cross-project edge — must be empty.
        let calls_b = entity_queries::calls_of_function(&storage, "fn-b").unwrap();
        assert!(
            !calls_b.iter().any(|e| e.id == "callee-a"),
            "cross-project callee must not leak into project B: {calls_b:?}"
        );
    }
}
