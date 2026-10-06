// Issue #856: v9→v10 migration test — split from schema_tests_migrations.rs
// to stay within the 500-line source budget.

use super::*;
use tempfile::NamedTempFile;

/// v9-schema repositories table (no unresolved_* columns — v9→v10 adds them;
/// git_url present since it was added in the original schema).
fn create_v9_repositories_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE repositories (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, git_url TEXT, local_path TEXT NOT NULL UNIQUE)",
        [],
    )
    .unwrap();
}

fn unresolved_columns(conn: &Connection) -> (Option<i64>, Option<i64>) {
    conn.query_row(
        "SELECT unresolved_internal, unresolved_external FROM repositories WHERE id = 'r1'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

#[test]
fn test_migrate_v9_to_v10_adds_null_unresolved_columns() {
    // #856: v9→v10 adds nullable `unresolved_internal`/`unresolved_external`
    // INTEGER columns to repositories. Pre-migration rows must read back as
    // NULL ("not recorded" — pre-fix indexes degrade to null, never a
    // fabricated zero).
    // NOTE: `tf` is held in scope so the NamedTempFile's file isn't
    // unlinked out from under the Connection (macOS unlink-during-open
    // can trigger SQLITE_IOERR).
    let tf = NamedTempFile::new().unwrap();
    let conn = Connection::open(tf.path()).unwrap();
    conn.pragma_update(None, "user_version", 9).unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    create_v9_repositories_table(&conn);
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES ('r1', 'p1', 'Test Repo', '/tmp/test')",
        [],
    )
    .unwrap();
    migrate(&conn).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 10);

    // Both columns exist on repositories.
    let cols: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM pragma_table_info('repositories') WHERE name IN ('unresolved_internal', 'unresolved_external')",
            )
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert_eq!(
        cols.len(),
        2,
        "both unresolved columns must exist: {cols:?}"
    );

    // Pre-migration rows read back NULL (not recorded) — the null-versus-zero
    // distinction the #856 design relies on.
    assert_eq!(unresolved_columns(&conn), (None, None));

    // A post-migration write of recorded values (zeros included) round-trips.
    conn.execute(
        "UPDATE repositories SET unresolved_internal = 0, unresolved_external = 5 WHERE id = 'r1'",
        [],
    )
    .unwrap();
    assert_eq!(unresolved_columns(&conn), (Some(0), Some(5)));

    // Idempotency: a second migrate() pass over the v10 DB advances to v11
    // (creates idx_repos_git_url), not a no-op.
    migrate(&conn).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 11);
}

#[test]
fn test_fresh_db_repositories_columns_match_upgraded_db() {
    // #856: a brand-new database (fresh path, SCHEMA_V1 DDL + version stamp)
    // must carry the same `unresolved_internal`/`unresolved_external`
    // columns as a v9→v10 upgraded one — otherwise fresh indexes would fail
    // reads with "no such column".
    let fresh = Connection::open_in_memory().unwrap();
    migrate(&fresh).unwrap();
    let fresh_cols: Vec<String> = {
        let mut stmt = fresh
            .prepare(
                "SELECT name FROM pragma_table_info('repositories') WHERE name IN ('unresolved_internal', 'unresolved_external')",
            )
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    };
    assert_eq!(
        fresh_cols.len(),
        2,
        "fresh DB must have both unresolved columns: {fresh_cols:?}"
    );
}

/// Issue #31: a v10 database whose `repositories.git_url` is NULL must
/// open and be upgraded to v11 — the v10→v11 step creates the
/// `idx_repos_git_url` index (serving FIND_REPOS_BY_GIT_URL without a
/// full table scan), and a NULL `git_url` reads back as `None`
/// (no fabricated default).
#[test]
fn test_v10_db_null_git_url_opens_unchanged() {
    let tf = NamedTempFile::new().unwrap();
    let conn = Connection::open(tf.path()).unwrap();
    conn.pragma_update(None, "user_version", 10).unwrap();
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE projects (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
            description TEXT, output_dirs TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE repositories (
            id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
            name TEXT NOT NULL, git_url TEXT,
            local_path TEXT NOT NULL UNIQUE,
            default_branch TEXT DEFAULT 'main',
            last_analyzed_commit TEXT, index_path TEXT,
            summarization_unconfigured TEXT,
            unresolved_internal INTEGER, unresolved_external INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO projects (id, name) VALUES ('p1', 'test-project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path, git_url) VALUES ('r1', 'p1', 'Test Repo', '/tmp/test', NULL)",
        [],
    )
    .unwrap();

    // v10 -> v11: the git_url index migration must run and bump the version.
    migrate(&conn).unwrap();
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 11);

    // The v10 -> v11 migration created idx_repos_git_url.
    let idx_name: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_repos_git_url'",
            [],
            |row| row.get(0),
        )
        .ok();
    assert_eq!(
        idx_name.as_deref(),
        Some("idx_repos_git_url"),
        "v10 -> v11 migration must create idx_repos_git_url"
    );

    // The row's git_url reads back NULL (None), not a fabricated default.
    let git_url: Option<String> = conn
        .query_row(
            "SELECT git_url FROM repositories WHERE id = 'r1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(git_url, None);
}
