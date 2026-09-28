use super::*;
use tempfile::NamedTempFile;

#[test]
fn test_migrate_v4_to_v5_adds_colgrep_index_path() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Initialize to v4
    conn.pragma_update(None, "user_version", 4).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Create schema v4 tables (same as v3, just different user_version)
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
        CREATE INDEX idx_entities_name_lower ON entities(LOWER(name));
        CREATE INDEX idx_entities_path_lower ON entities(LOWER(path));
        CREATE TABLE relationships (
            source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            rel_type TEXT NOT NULL CHECK (rel_type IN (
                'contains', 'depends_on', 'imports', 'calls', 'implements'
            )),
            weight REAL DEFAULT 1.0,
            evidence_json TEXT,
            PRIMARY KEY (source_id, target_id, rel_type)
        );
        CREATE INDEX idx_rel_source ON relationships(source_id);
        CREATE INDEX idx_rel_target ON relationships(target_id);
        CREATE INDEX idx_relationships_rel_type ON relationships(rel_type);
        CREATE TRIGGER fk_entities_parent_id_cascade
        BEFORE DELETE ON entities
        FOR EACH ROW
        BEGIN
            DELETE FROM entities WHERE id IN (
                WITH RECURSIVE subtree AS (
                    SELECT id FROM entities WHERE parent_id = OLD.id
                    UNION ALL
                    SELECT e.id FROM entities e
                    INNER JOIN subtree s ON e.parent_id = s.id
                )
                SELECT id FROM subtree
            );
        END;
        "#,
    )
    .unwrap();

    // Insert test data
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES ('r1', 'p1', 'Test Repo', '/tmp/test')",
        [],
    )
    .unwrap();

    // Run migration to v5 (colgrep_index_path already exists - IF NOT EXISTS no-op)
    migrate(&conn).unwrap();

    // Verify schema version is now 5
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);

    // Verify colgrep_index_path column exists and can be written/read
    conn.execute(
        "UPDATE repositories SET colgrep_index_path = '/tmp/colgrep.idx' WHERE id = 'r1'",
        [],
    )
    .unwrap();

    let path: Option<String> = conn
        .query_row(
            "SELECT colgrep_index_path FROM repositories WHERE id = 'r1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(path, Some("/tmp/colgrep.idx".to_string()));

    // Verify data integrity after migration
    let project_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(project_count, 1);
    let repo_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repo_count, 1);
}

#[test]
fn test_migrate_v4_to_v5_without_colgrep_index_path_column() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Initialize to v4
    conn.pragma_update(None, "user_version", 4).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Create schema v4 tables WITHOUT colgrep_index_path column (simulating a buggy state)
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
        CREATE INDEX idx_entities_name_lower ON entities(LOWER(name));
        CREATE INDEX idx_entities_path_lower ON entities(LOWER(path));
        CREATE TABLE relationships (
            source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            rel_type TEXT NOT NULL CHECK (rel_type IN (
                'contains', 'depends_on', 'imports', 'calls', 'implements'
            )),
            weight REAL DEFAULT 1.0,
            evidence_json TEXT,
            PRIMARY KEY (source_id, target_id, rel_type)
        );
        CREATE INDEX idx_rel_source ON relationships(source_id);
        CREATE INDEX idx_rel_target ON relationships(target_id);
        CREATE INDEX idx_relationships_rel_type ON relationships(rel_type);
        CREATE TRIGGER fk_entities_parent_id_cascade
        BEFORE DELETE ON entities
        FOR EACH ROW
        BEGIN
            DELETE FROM entities WHERE id IN (
                WITH RECURSIVE subtree AS (
                    SELECT id FROM entities WHERE parent_id = OLD.id
                    UNION ALL
                    SELECT e.id FROM entities e
                    INNER JOIN subtree s ON e.parent_id = s.id
                )
                SELECT id FROM subtree
            );
        END;
        "#,
    )
    .unwrap();

    // Insert test data
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES ('r1', 'p1', 'Test Repo', '/tmp/test')",
        [],
    )
    .unwrap();

    // Run migration to v5 (adds colgrep_index_path column)
    migrate(&conn).unwrap();

    // Verify schema version is now 5
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);

    // Verify colgrep_index_path column was added and can be written/read
    conn.execute(
        "UPDATE repositories SET colgrep_index_path = '/tmp/colgrep.idx' WHERE id = 'r1'",
        [],
    )
    .unwrap();

    let path: Option<String> = conn
        .query_row(
            "SELECT colgrep_index_path FROM repositories WHERE id = 'r1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(path, Some("/tmp/colgrep.idx".to_string()));

    // Verify data integrity after migration
    let project_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(project_count, 1);
    let repo_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repo_count, 1);
}

#[test]
fn test_migrate_v4_to_v6_is_idempotent() {
    // Test that migrating from v4 to v8 works correctly:
    // - v4 -> v5: adds colgrep_index_path column (if missing)
    // - v5 -> v6: renames colgrep_index_path to index_path
    // - v6 -> v7: adds idx_entities_repo_path index
    // - v7 -> v8: adds provenance column
    // - v8 -> v8: idempotent (no-op)
    // Each migrate() call reads user_version once and advances exactly one
    // step, so one migrate() call is required per version bump.
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Initialize to v4
    conn.pragma_update(None, "user_version", 4).unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();

    // Create schema v4 tables WITH colgrep_index_path column
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
        CREATE INDEX idx_entities_name_lower ON entities(LOWER(name));
        CREATE INDEX idx_entities_path_lower ON entities(LOWER(path));
        CREATE TABLE relationships (
            source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            rel_type TEXT NOT NULL CHECK (rel_type IN (
                'contains', 'depends_on', 'imports', 'calls', 'implements'
            )),
            weight REAL DEFAULT 1.0,
            evidence_json TEXT,
            PRIMARY KEY (source_id, target_id, rel_type)
        );
        CREATE INDEX idx_rel_source ON relationships(source_id);
        CREATE INDEX idx_rel_target ON relationships(target_id);
        CREATE INDEX idx_relationships_rel_type ON relationships(rel_type);
        CREATE TRIGGER fk_entities_parent_id_cascade
        BEFORE DELETE ON entities
        FOR EACH ROW
        BEGIN
            DELETE FROM entities WHERE id IN (
                WITH RECURSIVE subtree AS (
                    SELECT id FROM entities WHERE parent_id = OLD.id
                    UNION ALL
                    SELECT e.id FROM entities e
                    INNER JOIN subtree s ON e.parent_id = s.id
                )
                SELECT id FROM subtree
            );
        END;
        "#,
    )
    .unwrap();

    // Insert test data
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'Test Project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES ('r1', 'p1', 'Test Repo', '/tmp/test')",
        [],
    )
    .unwrap();

    // First migration: v4 -> v5 (incremental)
    migrate(&conn).unwrap();

    // Verify schema version is now 5
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);

    // Verify data integrity remains intact after first migration
    let project_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(project_count, 1);
    let repo_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repo_count, 1);

    // Second migration: v5 -> v6
    migrate(&conn).unwrap();

    // Verify schema version is now 6
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 6);

    // Third migration: v6 -> v7 (adds index)
    migrate(&conn).unwrap();

    // Verify version is now 7
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);

    // Fourth migration: v7 -> v8 (adds provenance column)
    migrate(&conn).unwrap();

    // Verify version is now 8
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);

    // Fifth migration: v8 -> v9 (adds summarization_unconfigured column)
    migrate(&conn).unwrap();

    // Verify version is now 9
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);

    // Sixth migration: v9 -> v10 (adds unresolved-import counter columns)
    migrate(&conn).unwrap();

    // Verify version is now 10
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 10);

    // Seventh migration: v10 -> v10 (should be idempotent - no-op)
    migrate(&conn).unwrap();

    // Verify version is still 10 and no error occurred
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 10);

    // Verify data integrity remains intact
    let project_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(project_count, 1);
    let repo_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM repositories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(repo_count, 1);
}
