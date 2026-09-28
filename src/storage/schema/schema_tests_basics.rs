use super::*;
use tempfile::NamedTempFile;

#[test]
fn test_migrate_fresh_db() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Verify DB starts at version 0
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);

    // Run migration
    migrate(&conn).unwrap();

    // Verify schema version is now 3
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    // Check table existence by querying each one
    assert!(conn.execute("SELECT 1 FROM projects", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM repositories", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM entities", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM relationships", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM conventions", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM insights", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM analysis_runs", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM file_hashes", []).is_ok());
}
#[test]
fn test_migrate_already_migrated() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Run migration once
    migrate(&conn).unwrap();

    // Run migration again - should not fail
    migrate(&conn).unwrap();

    // Verify version is still 3
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn test_migrate_sets_pragmas() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    migrate(&conn).unwrap();

    // Check busy_timeout
    let timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert_eq!(timeout, 5000);

    // Check journal_mode is WAL
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    // Check foreign_keys is enabled
    let fk_enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk_enabled, 1, "foreign_keys pragma must be enabled");
}

#[test]
fn test_migrate_enables_foreign_key_enforcement() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    migrate(&conn).unwrap();

    // Attempting to insert a repository referencing a non-existent project
    // must fail when foreign_keys is enabled.
    let result = conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES ('r1', 'nonexistent', 'repo', '/tmp/r1')",
        [],
    );
    assert!(
        result.is_err(),
        "foreign key constraint must prevent orphan repository"
    );
}

#[test]
fn test_migrate_rejects_newer_version() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Manually set a newer schema version
    conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();

    // migrate() should return MigrationFailed error
    let result = migrate(&conn);
    assert!(
        result.is_err(),
        "should reject database from newer lievo version"
    );

    let err = result.unwrap_err();
    assert!(
        matches!(err, LievoError::MigrationFailed { from, to, reason } if {
            from == SCHEMA_VERSION + 1 &&
            to == SCHEMA_VERSION &&
            reason.contains("newer version")
        })
    );
}

#[test]
fn test_migrate_within_transaction() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    // Run migration
    migrate(&conn).unwrap();

    // Verify schema was applied (tables exist)
    assert!(conn.execute("SELECT 1 FROM projects", []).is_ok());
    assert!(conn.execute("SELECT 1 FROM repositories", []).is_ok());

    // Verify version is set
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn test_schema_v1_contains_all_tables() {
    // Verify SCHEMA_V1 contains CREATE TABLE for all 8 tables
    let schema = SCHEMA_V1;
    assert!(schema.contains("CREATE TABLE projects"));
    assert!(schema.contains("CREATE TABLE repositories"));
    assert!(schema.contains("CREATE TABLE entities"));
    assert!(schema.contains("CREATE TABLE relationships"));
    assert!(schema.contains("CREATE TABLE conventions"));
    assert!(schema.contains("CREATE TABLE insights"));
    assert!(schema.contains("CREATE TABLE analysis_runs"));
    assert!(schema.contains("CREATE TABLE file_hashes"));
}

#[test]
fn test_schema_v1_contains_all_indexes() {
    // Verify SCHEMA_V1 contains all required indexes
    let schema = SCHEMA_V1;
    assert!(schema.contains("CREATE INDEX idx_entities_tier"));
    assert!(schema.contains("CREATE INDEX idx_entities_parent"));
    assert!(schema.contains("CREATE INDEX idx_entities_repo"));
    assert!(schema.contains("CREATE INDEX idx_rel_source"));
    assert!(schema.contains("CREATE INDEX idx_rel_target"));
}

#[test]
fn test_schema_v1_constraint_exists() {
    let temp_file = NamedTempFile::new().unwrap();
    let conn = Connection::open(temp_file.path()).unwrap();

    migrate(&conn).unwrap();

    // Test entity tier constraint
    let result = conn.execute(
        "INSERT INTO entities (id, project_id, name, tier) VALUES ('e1', 'p1', 'test', 'invalid_tier')",
        []
    );
    assert!(result.is_err(), "Should reject invalid entity tier");

    // Test relationship type constraint
    let result = conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('e1', 'e2', 'invalid_rel')",
        []
    );
    assert!(result.is_err(), "Should reject invalid relationship type");

    // Test insight severity constraint
    let result = conn.execute(
        "INSERT INTO insights (id, project_id, title, category, severity) VALUES ('i1', 'p1', 'test', 'test', 'invalid_sev')",
        []
    );
    assert!(result.is_err(), "Should reject invalid severity");
}

#[cfg(test)]
mod debug_mig_removed {}
