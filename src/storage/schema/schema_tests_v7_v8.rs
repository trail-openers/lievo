// Issue #714: v7→v8 migration test — split from schema_tests_migrations.rs
// to stay within the 500-line source budget.

use super::*;
use tempfile::NamedTempFile;

/// v7-schema relationships table (no provenance column — v7→v8 adds it).
fn create_v7_relationships_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE relationships (source_id TEXT NOT NULL, target_id TEXT NOT NULL, rel_type TEXT\n\
         NOT NULL CHECK (rel_type IN ('contains','depends_on','imports','calls','implements')),\n\
         weight REAL DEFAULT 1.0, evidence_json TEXT, PRIMARY KEY (source_id, target_id, rel_type))",
        [],
    )
    .unwrap();
}

fn provenance_for_target(conn: &Connection, target: &str) -> String {
    conn.query_row(
        "SELECT provenance FROM relationships WHERE target_id = ?1",
        [target],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn test_migrate_v7_to_v8_adds_provenance_with_heuristic_backfill() {
    // #714: v7→v8 adds `provenance TEXT NOT NULL DEFAULT 'heuristic'`.
    // Pre-migration rows cannot be retro-classified (evidence_json doesn't
    // record which resolver path hit), so they must read back the
    // documented backfill default.
    // NOTE: `tf` is held in scope so the NamedTempFile's file isn't
    // unlinked out from under the Connection (macOS unlink-during-open
    // can trigger SQLITE_IOERR).
    let tf = NamedTempFile::new().unwrap();
    let conn = Connection::open(tf.path()).unwrap();
    conn.pragma_update(None, "user_version", 7).unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    create_v7_relationships_table(&conn);
    // The v8 -> v9 migration touches the repositories table (adds the
    // summarization_unconfigured marker column), so the v7 fixture needs a
    // minimal repositories table for the migration to succeed.
    conn.execute(
        "CREATE TABLE repositories (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, local_path TEXT NOT NULL UNIQUE)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type)\n\
         VALUES ('e1', 'e2', 'imports'), ('e1', 'e3', 'calls')",
        [],
    )
    .unwrap();
    migrate(&conn).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);

    // Column exists on relationships.
    let col: Option<String> = conn
        .query_row(
            "SELECT name FROM pragma_table_info('relationships') WHERE name='provenance'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(None);
    assert_eq!(col, Some("provenance".to_string()));
    // Backfill: both pre-migration rows read back the documented default.
    assert_eq!(provenance_for_target(&conn, "e2"), "heuristic");
    assert_eq!(provenance_for_target(&conn, "e3"), "heuristic");
    // A post-migration insert can explicitly carry 'resolved' and read it back.
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type, provenance)\n\
         VALUES ('e1', 'e4', 'imports', 'resolved')",
        [],
    )
    .unwrap();
    assert_eq!(provenance_for_target(&conn, "e4"), "resolved");
    // An insert that omits provenance lands on the documented default.
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type) VALUES ('e1', 'e5', 'imports')",
        [],
    )
    .unwrap();
    assert_eq!(provenance_for_target(&conn, "e5"), "heuristic");
    // Idempotency: a second migrate() pass over the v9 DB is a no-op.
    migrate(&conn).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 9);
}
