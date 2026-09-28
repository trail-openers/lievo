// Tests for entity_ids_for_paths function
// Lives in a separate file to keep sqlite_ops.rs under the 500-line limit

#[cfg(test)]
mod tests {
    use crate::model::{Entity, EntityTier};
    use crate::storage::Storage;
    use crate::storage::sqlite::SqliteStorage;

    #[test]
    fn test_entities_by_paths_batch_empty() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "/tmp/test")
            .unwrap();

        // Empty paths should return empty map
        let result = storage.entity_ids_for_paths(&repo.id, &[]).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_entities_by_paths_batch_single_match() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "/tmp/test")
            .unwrap();

        let entity = Entity {
            id: "e1".to_string(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "test_file.rs".to_string(),
            path: Some("src/test_file.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        storage.upsert_entity(&entity).unwrap();

        let result = storage
            .entity_ids_for_paths(&repo.id, &["src/test_file.rs"])
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("src/test_file.rs"), Some(&"e1".to_string()));
    }

    #[test]
    fn test_entities_by_paths_batch_multiple_matches() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "./tmp/test")
            .unwrap();

        let entities = vec![
            Entity {
                id: "e1".to_string(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "a.rs".to_string(),
                path: Some("src/a.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: "e2".to_string(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "b.rs".to_string(),
                path: Some("src/b.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: "e3".to_string(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "c.rs".to_string(),
                path: Some("src/c.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        for entity in &entities {
            storage.upsert_entity(entity).unwrap();
        }

        let paths = ["src/a.rs", "src/b.rs", "src/c.rs"];
        let result = storage.entity_ids_for_paths(&repo.id, &paths).unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result.get("src/a.rs"), Some(&"e1".to_string()));
        assert_eq!(result.get("src/b.rs"), Some(&"e2".to_string()));
        assert_eq!(result.get("src/c.rs"), Some(&"e3".to_string()));
    }

    #[test]
    fn test_entities_by_paths_batch_partial_matches() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "./tmp/test")
            .unwrap();

        let entity = Entity {
            id: "e1".to_string(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "exists.rs".to_string(),
            path: Some("src/exists.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        storage.upsert_entity(&entity).unwrap();

        let paths = ["src/exists.rs", "src/missing.rs"];
        let result = storage.entity_ids_for_paths(&repo.id, &paths).unwrap();

        // Only existing path should be in result
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("src/exists.rs"), Some(&"e1".to_string()));
        assert!(!result.contains_key("src/missing.rs"));
    }

    #[test]
    fn test_entities_by_paths_batch_matches_sequential_behavior() {
        // Verify batch returns same entity_ids as sequential entity_by_path
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "test-repo", "./tmp/test")
            .unwrap();

        let entities = vec![
            Entity {
                id: "e1".to_string(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "file1.rs".to_string(),
                path: Some("./file1.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: "e2".to_string(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "file2.rs".to_string(),
                path: Some("src/file2.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        for entity in &entities {
            storage.upsert_entity(entity).unwrap();
        }

        let paths = ["./file1.rs", "src/file2.rs"];

        // Batch lookup
        let batch_result = storage.entity_ids_for_paths(&repo.id, &paths).unwrap();

        // Sequential lookup for verification
        assert_eq!(
            batch_result.get("./file1.rs").unwrap(),
            &storage
                .entity_by_path(&repo.id, "./file1.rs")
                .unwrap()
                .unwrap()
                .id
        );
        assert_eq!(
            batch_result.get("src/file2.rs").unwrap(),
            &storage
                .entity_by_path(&repo.id, "src/file2.rs")
                .unwrap()
                .unwrap()
                .id
        );
    }
}
