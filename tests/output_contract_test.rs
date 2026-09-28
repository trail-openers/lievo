// Tests for output contract: deterministic JSON, TTY detection, structured stderr.

use lievo::error::LievoError;
use lievo::model::{Entity, EntityTier};

// ---------------------------------------------------------------------------
// Deterministic JSON ordering tests
// ---------------------------------------------------------------------------

#[test]
fn test_entity_json_fields_have_deterministic_order() {
    // Create an entity with all fields
    let entity = Entity {
        id: "test-id".to_string(),
        project_id: "proj-id".to_string(),
        repo_id: Some("repo-id".to_string()),
        tier: EntityTier::Module,
        parent_id: Some("parent-id".to_string()),
        name: "test-entity".to_string(),
        path: Some("src/test.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("A test entity".to_string()),
        summary_commit: Some("abc123".to_string()),
        metrics_json: Some(r#"{"complexity": 10}"#.to_string()),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
    };

    // Serialize to JSON string
    let json_str = serde_json::to_string(&entity).expect("serialization should succeed");

    // Parse and verify field order matches expected deterministic order
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("should be valid JSON");

    if let serde_json::Value::Object(ref map) = parsed {
        let keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();

        // Expected deterministic field order
        let expected_order = vec![
            "id",
            "project_id",
            "repo_id",
            "tier",
            "parent_id",
            "name",
            "path",
            "language",
            "summary",
            "summary_commit",
            "metrics_json",
            "created_at",
            "updated_at",
        ];

        assert_eq!(
            keys, expected_order,
            "JSON fields should be in deterministic order.\nGot: {keys:?}\nExpected: {expected_order:?}"
        );
    } else {
        panic!("Entity should serialize to JSON object");
    }
}

#[test]
fn test_entity_json_empty_optional_fields_preserved() {
    // Test that null/missing fields are represented consistently
    let entity = Entity {
        id: "test".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "test.rs".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let json_str = serde_json::to_string(&entity).expect("serialization should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("should be valid JSON");

    if let serde_json::Value::Object(ref map) = parsed {
        // Null fields should be present as null, not omitted
        assert_eq!(map["repo_id"], serde_json::Value::Null);
        assert_eq!(map["parent_id"], serde_json::Value::Null);
        assert_eq!(map["path"], serde_json::Value::Null);
        assert_eq!(map["language"], serde_json::Value::Null);
        assert_eq!(map["summary"], serde_json::Value::Null);
        assert_eq!(map["summary_commit"], serde_json::Value::Null);
        assert_eq!(map["metrics_json"], serde_json::Value::Null);
    }
}

#[test]
fn test_entity_json_multiple_serializations_identical() {
    // Verify that serializing the same entity twice produces identical strings
    let entity = Entity {
        id: "id-1".to_string(),
        project_id: "proj-1".to_string(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "module-1".to_string(),
        path: Some("src/mod.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let json1 = serde_json::to_string(&entity).expect("first serialization should succeed");
    let json2 = serde_json::to_string(&entity).expect("second serialization should succeed");

    assert_eq!(
        json1, json2,
        "Multiple serializations of the same entity should be identical"
    );
}

// ---------------------------------------------------------------------------
// Structured stderr envelope tests
// ---------------------------------------------------------------------------

#[test]
fn test_error_envelope_valid_json_structure() {
    let error = LievoError::DatabaseLocked;
    let envelope = lievo::output::format_error_envelope(&error);

    // Envelope should be valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&envelope).expect("error envelope should be valid JSON");

    // Should have required fields
    assert!(parsed.is_object(), "envelope should be a JSON object");
    assert!(parsed.get("error").is_some(), "should have 'error' field");
    assert!(parsed.get("code").is_some(), "should have 'code' field");
    assert!(
        parsed.get("retryable").is_some(),
        "should have 'retryable' field"
    );
}

#[test]
fn test_error_envelope_content_correct() {
    let error = LievoError::DatabaseLocked;
    let envelope = lievo::output::format_error_envelope(&error);

    let parsed: serde_json::Value = serde_json::from_str(&envelope).expect("should be valid JSON");

    // Check specific values
    assert_eq!(
        parsed["error"]["message"].as_str(),
        Some("Database locked — is another lievo process running?"),
        "error message should match"
    );
    assert_eq!(parsed["error"]["kind"].as_str(), Some("DatabaseLocked"));
    assert_eq!(parsed["code"].as_str(), Some("DATABASE_LOCKED"));
    assert_eq!(
        parsed["retryable"], true,
        "database locked should be retryable"
    );
}

#[test]
fn test_error_envelope_non_retryable_errors() {
    let error = LievoError::EntityNotFound("test-id".to_string());
    let envelope = lievo::output::format_error_envelope(&error);

    let parsed: serde_json::Value = serde_json::from_str(&envelope).expect("should be valid JSON");

    assert_eq!(
        parsed["retryable"], false,
        "entity not found should not be retryable"
    );
    assert_eq!(parsed["code"].as_str(), Some("ENTITY_NOT_FOUND"));
}

#[test]
fn test_error_envelope_order_deterministic() {
    let error1 = LievoError::ProjectNotFound("test".to_string());
    let error2 = LievoError::ProjectNotFound("test".to_string());

    let env1 = lievo::output::format_error_envelope(&error1);
    let env2 = lievo::output::format_error_envelope(&error2);

    assert_eq!(
        env1, env2,
        "Error envelopes should be deterministic (same error = same JSON)"
    );
}

#[test]
fn test_error_envelope_field_order_matches_spec() {
    let error = LievoError::DatabaseLocked;
    let envelope = lievo::output::format_error_envelope(&error);

    let parsed: serde_json::Value = serde_json::from_str(&envelope).expect("should be valid JSON");

    if let serde_json::Value::Object(ref map) = parsed {
        let keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();

        // Expected field order per spec: error, code, retryable (top-level)
        let expected_top_level = vec!["error", "code", "retryable"];
        assert_eq!(
            keys, expected_top_level,
            "Top-level error envelope fields should be in deterministic order.\nGot: {keys:?}\nExpected: {expected_top_level:?}"
        );

        // Now verify nested error object field order
        let error_obj = &parsed["error"];
        if let serde_json::Value::Object(obj) = error_obj {
            let error_keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            let expected_error_obj = vec!["message", "kind"];
            assert_eq!(
                error_keys, expected_error_obj,
                "Nested error object fields should be in deterministic order.\nGot: {error_keys:?}\nExpected: {expected_error_obj:?}"
            );
        } else {
            panic!("error field should be an object");
        }
    } else {
        panic!("Error envelope should serialize to JSON object");
    }
}

// ---------------------------------------------------------------------------
// NDJSON formatting tests
// ---------------------------------------------------------------------------

#[test]
fn test_entities_json_ndjson_format() {
    let entities = vec![
        Entity {
            id: "id-1".to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier: EntityTier::Module,
            parent_id: None,
            name: "module-1".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "id-2".to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier: EntityTier::Module,
            parent_id: None,
            name: "module-2".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
    ];

    let mut buf = Vec::new();
    lievo::output::format_entities_json(&entities, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();

    // Should have exactly 2 lines (2 JSON objects, newline-separated)
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "NDJSON should have one JSON object per line"
    );

    // Each line should be valid JSON
    for line in &lines {
        let _: serde_json::Value =
            serde_json::from_str(line).expect("each NDJSON line should be valid JSON");
    }
}

#[test]
fn test_empty_entities_json_returns_empty_string() {
    let entities: Vec<Entity> = vec![];
    let mut buf = Vec::new();
    lievo::output::format_entities_json(&entities, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert_eq!(
        output.trim(),
        "",
        "empty entity list should return empty string"
    );
}
