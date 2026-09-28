use super::*;
use tempfile::NamedTempFile;

#[test]
fn test_migrate_v2_to_v3_cleans_orphaned_parent_id() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Initialize to v2
    conn.pragma_update(None, "user_version", 2).unwrap();
    // Foreign keys must be enabled for CASCADE to work
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Create schema v2 tables
    conn.execute_batch(
        r#"
        CREATE TABLE projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            output_dirs TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE repositories (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            name TEXT NOT NULL,
            git_url TEXT,
            local_path TEXT NOT NULL UNIQUE,
            default_branch TEXT DEFAULT 'main',
            last_analyzed_commit TEXT,
            colgrep_index_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE entities (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            repo_id TEXT REFERENCES repositories(id),
            tier TEXT NOT NULL CHECK (tier IN ('subsystem', 'module', 'file', 'function')),
            parent_id TEXT REFERENCES entities(id),
            name TEXT NOT NULL,
            path TEXT,
            language TEXT,
            summary TEXT,
            summary_commit TEXT,
            metrics_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(project_id, repo_id, tier, path)
        );
        CREATE INDEX idx_entities_tier ON entities(project_id, tier);
        CREATE INDEX idx_entities_parent ON entities(parent_id);
        CREATE INDEX idx_entities_repo ON entities(repo_id, tier);
        CREATE TABLE relationships (
            source_id TEXT NOT NULL REFERENCES entities(id),
            target_id TEXT NOT NULL REFERENCES entities(id),
            rel_type TEXT NOT NULL CHECK (rel_type IN (
                'contains', 'depends_on', 'imports', 'calls', 'implements'
            )),
            weight REAL DEFAULT 1.0,
            evidence_json TEXT,
            PRIMARY KEY (source_id, target_id, rel_type)
        );
        CREATE INDEX idx_rel_source ON relationships(source_id);
        CREATE INDEX idx_rel_target ON relationships(target_id);
        "#,
    )
    .unwrap();

    // Disable FKs to allow creating orphaned references in v2
    conn.pragma_update(None, "foreign_keys", false).unwrap();

    // Insert test data
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier) VALUES ('parent', 'p1', 'Parent', 'module')",
        [],
    )
    .unwrap();
    // Insert entity with orphaned parent_id (references 'nonexistent' which doesn't exist)
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier, parent_id) VALUES ('orphan', 'p1', 'Orphan', 'file', 'nonexistent')",
        [],
    )
    .unwrap();

    // Re-enable FKs for migration and cascade testing
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Verify v2 state: orphan has parent_id set
    let orphan_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM entities WHERE id = 'orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphan_parent, Some("nonexistent".to_string()));
    // Run migration to v3
    migrate(&conn).unwrap();

    // Verify v3 state (not SCHEMA_VERSION since v3 can't migrate to v5)
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);

    // Verify orphan's parent_id was reset to NULL by migration
    let orphan_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM entities WHERE id = 'orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        orphan_parent, None,
        "orphaned parent_id should be reset to NULL during migration"
    );
}

#[test]
fn test_migrate_v2_to_v3_cascade_deep_hierarchy() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Initialize to v2
    conn.pragma_update(None, "user_version", 2).unwrap();
    // Foreign keys must be enabled for CASCADE to work
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Create schema v2 tables
    conn.execute_batch(
        r#"
        CREATE TABLE projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            output_dirs TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE repositories (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            name TEXT NOT NULL,
            git_url TEXT,
            local_path TEXT NOT NULL UNIQUE,
            default_branch TEXT DEFAULT 'main',
            last_analyzed_commit TEXT,
            colgrep_index_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE entities (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id),
            repo_id TEXT REFERENCES repositories(id),
            tier TEXT NOT NULL CHECK (tier IN ('subsystem', 'module', 'file', 'function')),
            parent_id TEXT REFERENCES entities(id),
            name TEXT NOT NULL,
            path TEXT,
            language TEXT,
            summary TEXT,
            summary_commit TEXT,
            metrics_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(project_id, repo_id, tier, path)
        );
        CREATE INDEX idx_entities_tier ON entities(project_id, tier);
        CREATE INDEX idx_entities_parent ON entities(parent_id);
        CREATE INDEX idx_entities_repo ON entities(repo_id, tier);
        CREATE TABLE relationships (
            source_id TEXT NOT NULL REFERENCES entities(id),
            target_id TEXT NOT NULL REFERENCES entities(id),
            rel_type TEXT NOT NULL CHECK (rel_type IN (
                'contains', 'depends_on', 'imports', 'calls', 'implements'
            )),
            weight REAL DEFAULT 1.0,
            evidence_json TEXT,
            PRIMARY KEY (source_id, target_id, rel_type)
        );
        CREATE INDEX idx_rel_source ON relationships(source_id);
        CREATE INDEX idx_rel_target ON relationships(target_id);
        "#,
    )
    .unwrap();

    // Disable FKs during v2 data insertion
    conn.pragma_update(None, "foreign_keys", false).unwrap();

    // Insert test data: 4-level hierarchy
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier) VALUES ('subsystem', 'p1', 'Subsystem', 'subsystem')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier, parent_id) VALUES ('module', 'p1', 'Module', 'module', 'subsystem')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier, parent_id) VALUES ('file', 'p1', 'File', 'file', 'module')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities (id, project_id, name, tier, parent_id) VALUES ('function', 'p1', 'Function', 'function', 'file')",
        [],
    )
    .unwrap();

    // Add relationships between entities
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('subsystem', 'module', 'contains')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('module', 'file', 'contains')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('file', 'function', 'contains')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('function', 'module', 'depends_on')",
        [],
    )
    .unwrap();

    // Re-enable FKs for migration and cascade testing
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Verify v2 state
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entity_count, 4);
    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 4);

    // Run migration to v3
    migrate(&conn).unwrap();

    // Verify foreign keys are enabled for cascade to work
    let fk_enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        fk_enabled, 1,
        "foreign_keys must be enabled after migration"
    );

    // Verify v3 state (not SCHEMA_VERSION since v3 can't migrate to v5)
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entity_count, 4, "entities should be preserved");
    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 4, "relationships should be preserved");

    // Delete the top-level subsystem entity
    conn.execute("DELETE FROM entities WHERE id = 'subsystem'", [])
        .unwrap();

    // Verify CASCADE deleted all entities in the hierarchy
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        entity_count, 0,
        "CASCADE should delete entire hierarchy when root is deleted"
    );

    // Verify CASCADE deleted all relationships
    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        rel_count, 0,
        "CASCADE should delete all relationships when entities are deleted"
    );
}
