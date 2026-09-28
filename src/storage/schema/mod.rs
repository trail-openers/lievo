// SQLite schema DDL and migration system
use crate::Result;
use crate::error::LievoError;
use rusqlite::Connection;

pub const SCHEMA_VERSION: u32 = 10;
pub const SCHEMA_V1: &str = r#"
-- Projects (top-level umbrella, spans multiple repos)
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    output_dirs TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Repositories linked to projects
CREATE TABLE repositories (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    git_url TEXT,
    local_path TEXT NOT NULL UNIQUE,
    default_branch TEXT DEFAULT 'main',
    last_analyzed_commit TEXT,
    index_path TEXT,
    summarization_unconfigured TEXT,
    unresolved_internal INTEGER,
    unresolved_external INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Entities at four tiers
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
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX idx_entities_unique_non_function ON entities(project_id, repo_id, tier, path) WHERE tier != 'function';
CREATE INDEX idx_entities_tier ON entities(project_id, tier);
CREATE INDEX idx_entities_parent ON entities(parent_id);
CREATE INDEX idx_entities_repo ON entities(repo_id, tier);
CREATE INDEX idx_entities_name_lower ON entities(LOWER(name));
CREATE INDEX idx_entities_path_lower ON entities(LOWER(path));

-- Relationships between entities
CREATE TABLE relationships (
    source_id TEXT NOT NULL REFERENCES entities(id),
    target_id TEXT NOT NULL REFERENCES entities(id),
    rel_type TEXT NOT NULL CHECK (rel_type IN (
        'contains', 'depends_on', 'imports', 'calls', 'implements'
    )),
    weight REAL DEFAULT 1.0,
    evidence_json TEXT,
    provenance TEXT NOT NULL DEFAULT 'heuristic' CHECK (provenance IN ('resolved', 'heuristic')),
    PRIMARY KEY (source_id, target_id, rel_type)
);
CREATE INDEX idx_rel_source ON relationships(source_id);
CREATE INDEX idx_rel_target ON relationships(target_id);
CREATE INDEX idx_relationships_rel_type ON relationships(rel_type);

-- Detected conventions
CREATE TABLE conventions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    category TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    example_code TEXT,
    confidence REAL DEFAULT 0.7,
    entity_ids_json TEXT,
    detected_at TEXT NOT NULL DEFAULT (datetime('now')),
    still_valid INTEGER DEFAULT 1
);

-- Quality insights
CREATE TABLE insights (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    category TEXT NOT NULL,
    severity TEXT CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    title TEXT NOT NULL,
    description TEXT,
    entity_ids_json TEXT,
    detected_at TEXT NOT NULL DEFAULT (datetime('now')),
    still_valid INTEGER DEFAULT 1
);

-- Analysis run audit trail
CREATE TABLE analysis_runs (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repositories(id),
    commit_hash TEXT NOT NULL,
    files_analyzed INTEGER DEFAULT 0,
    files_changed INTEGER DEFAULT 0,
    entities_upserted INTEGER DEFAULT 0,
    relationships_upserted INTEGER DEFAULT 0,
    duration_ms INTEGER,
    status TEXT DEFAULT 'pending',
    completed_at TEXT
);

-- File content hashes for incremental detection
CREATE TABLE file_hashes (
    repo_id TEXT NOT NULL REFERENCES repositories(id),
    file_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    last_analyzed TEXT NOT NULL,
    PRIMARY KEY (repo_id, file_path)
);
"#;

// Removed unused v6 migration block (fn_calls table does not exist)

pub fn migrate(conn: &Connection) -> Result<()> {
    // Read current schema version
    let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    // Reject databases from newer versions of lievo
    if version > SCHEMA_VERSION {
        return Err(LievoError::MigrationFailed {
            from: version,
            to: SCHEMA_VERSION,
            reason: "database was created by a newer version of lievo".into(),
        });
    }

    // If version is 0, fresh DB - run initial schema in transaction
    if version == 0 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute_batch(SCHEMA_V1)?;

        // Create trigger for CASCADE delete on entities.parent_id
        // Uses recursive CTE to delete entire subtree in bottom-up order
        // (children before parents) to avoid FK constraint violations
        conn.execute(
            r#"
            CREATE TRIGGER IF NOT EXISTS fk_entities_parent_id_cascade
            BEFORE DELETE ON entities
            FOR EACH ROW
            BEGIN
                DELETE FROM entities WHERE id IN (
                    WITH RECURSIVE subtree AS (
                        -- Start with direct children
                        SELECT id FROM entities WHERE parent_id = OLD.id
                        UNION ALL
                        -- Add descendants recursively
                        SELECT e.id FROM entities e
                        INNER JOIN subtree s ON e.parent_id = s.id
                    )
                    SELECT id FROM subtree
                );
            END
            "#,
            [],
        )?;

        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 1 -> 2: add output_dirs column to projects table
    if version == 1 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute("ALTER TABLE projects ADD COLUMN output_dirs TEXT", [])?;
        conn.pragma_update(None, "user_version", 2)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 2 -> 3: add ON DELETE CASCADE to relationships and entities FKs
    // SQLite doesn't support ALTER TABLE ADD CONSTRAINT, so we recreate tables
    if version == 2 {
        conn.execute_batch("BEGIN IMMEDIATE")?;

        // Disable FK enforcement during migration to avoid CASCADE issues
        conn.pragma_update(None, "foreign_keys", false)?;

        // Migration integrity check: identify and reset orphaned parent_id references
        // These are entities whose parent_id references a non-existent entity.
        // Setting them to NULL prevents migration failures due to FK violations.
        conn.execute(
            r#"
            UPDATE entities
            SET parent_id = NULL
            WHERE parent_id IS NOT NULL
              AND NOT EXISTS (
                  SELECT 1 FROM entities AS parent WHERE parent.id = entities.parent_id
              )
          "#,
            [],
        )
        .map_err(|e| LievoError::MigrationFailed {
            from: 2,
            to: 3,
            reason: format!("Failed to clean orphaned parent_id references: {e}"),
        })?;

        // Recreate relationships table with CASCADE on source_id and target_id
        conn.execute_batch(
            r#"
            CREATE TABLE relationships_new (
                source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                rel_type TEXT NOT NULL CHECK (rel_type IN (
                    'contains', 'depends_on', 'imports', 'calls', 'implements'
                )),
                weight REAL DEFAULT 1.0,
                evidence_json TEXT,
                PRIMARY KEY (source_id, target_id, rel_type)
            );
            INSERT INTO relationships_new SELECT * FROM relationships;
            DROP TABLE relationships;
            ALTER TABLE relationships_new RENAME TO relationships;
            CREATE INDEX idx_rel_source ON relationships(source_id);
            CREATE INDEX idx_rel_target ON relationships(target_id);
            CREATE INDEX idx_relationships_rel_type ON relationships(rel_type);
            "#,
        )?;

        // Note: We do NOT add CASCADE to entities.parent_id because SQLite requires
        // recreating the entire table to add/mod FK constraints, and recreating a table
        // with a self-referential CASCADE FK is impossible (DROPPING the old table
        // would CASCADE delete from the new table). Instead, we use a trigger:
        // BEFORE DELETE with recursive CTE to delete entire subtree in bottom-up order
        conn.execute(
            r#"
            CREATE TRIGGER IF NOT EXISTS fk_entities_parent_id_cascade
            BEFORE DELETE ON entities
            FOR EACH ROW
            BEGIN
                DELETE FROM entities WHERE id IN (
                    WITH RECURSIVE subtree AS (
                        -- Start with direct children
                        SELECT id FROM entities WHERE parent_id = OLD.id
                        UNION ALL
                        -- Add descendants recursively
                        SELECT e.id FROM entities e
                        INNER JOIN subtree s ON e.parent_id = s.id
                    )
                    SELECT id FROM subtree
                );
            END
            "#,
            [],
        )?;

        // Re-enable FK enforcement
        conn.pragma_update(None, "foreign_keys", true)?;

        conn.pragma_update(None, "user_version", 3)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 3 -> 4: add computed indexes for efficient name/path LIKE queries
    if version == 3 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_entities_name_lower ON entities(LOWER(name))",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_entities_path_lower ON entities(LOWER(path))",
            [],
        )?;
        conn.pragma_update(None, "user_version", 4)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 4 -> 5: add colgrep_index_path column to repositories table
    // Check if column exists first (SQLite ALTER TABLE doesn't support IF NOT EXISTS)
    if version == 4 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let column_exists: bool = conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('repositories') WHERE name='colgrep_index_path'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !column_exists {
            conn.execute(
                "ALTER TABLE repositories ADD COLUMN colgrep_index_path TEXT",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 5)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 5 -> 6: rename colgrep_index_path to index_path
    // Since we're pre-production, we can rename outright (drop + recreate, no migration shim)
    if version == 5 {
        conn.execute_batch("BEGIN IMMEDIATE")?;

        // Disable FK enforcement during migration
        conn.pragma_update(None, "foreign_keys", false)?;

        // Recreate repositories table with index_path instead of colgrep_index_path
        conn.execute_batch(
            r#"
            CREATE TABLE repositories_new (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id),
                name TEXT NOT NULL,
                git_url TEXT,
                local_path TEXT NOT NULL UNIQUE,
                default_branch TEXT DEFAULT 'main',
                last_analyzed_commit TEXT,
                index_path TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            INSERT INTO repositories_new SELECT id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, colgrep_index_path, created_at, updated_at FROM repositories;
            DROP TABLE repositories;
            ALTER TABLE repositories_new RENAME TO repositories;
            "#,
        )?;

        // Re-enable FK enforcement
        conn.pragma_update(None, "foreign_keys", true)?;

        conn.pragma_update(None, "user_version", 6)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 6 -> 7: add composite index for (repo_id, path) batch queries
    if version == 6 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_entities_repo_path ON entities(repo_id, path)",
            [],
        )?;
        conn.pragma_update(None, "user_version", 7)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 7 -> 8: add provenance column to relationships table (issue #714).
    // Additive, backfill default 'heuristic' for all pre-existing rows (the
    // resolved-vs-heuristic distinction is not derivable post-hoc from
    // evidence_json — see the PM decision on issue #714). Fresh rows created
    // after this migration are written with an explicit provenance value at
    // creation time by the relationship builder; the column default applies
    // only to backfilled pre-migration rows.
    if version == 7 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let column_exists: bool = conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('relationships') WHERE name='provenance'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !column_exists {
            conn.execute(
                "ALTER TABLE relationships ADD COLUMN provenance TEXT NOT NULL DEFAULT 'heuristic'",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 8)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 8 -> 9: add summarization_unconfigured column to repositories
    // table (issue #788). Records that the summarization gate was enabled for
    // this repo but no backend was configured or available in the last run,
    // so the staleness check can skip re-entering the full pipeline for that
    // repo until a new commit arrives or the config changes. NULL = no
    // pending marker (the normal state).
    if version == 8 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let column_exists: bool = conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('repositories') WHERE name='summarization_unconfigured'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !column_exists {
            conn.execute(
                "ALTER TABLE repositories ADD COLUMN summarization_unconfigured TEXT",
                [],
            )?;
        }
        conn.pragma_update(None, "user_version", 9)?;
        conn.execute_batch("COMMIT")?;
    }

    // Version 9 -> 10: add nullable unresolved-import counter columns to the
    // repositories table (issue #856). The repo-wide UnresolvedCounts
    // previously persisted onto a file entity whose path equals the repo
    // name — an entity no real index contains. The counts are a repo-level
    // fact, so they now live on the repo row: NULL means "not recorded"
    // (pre-fix indexes read as null until re-indexed), a stored 0 means a
    // fully-resolved fresh index. Existing rows migrate to NULL.
    if version == 9 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let column_exists = |conn: &Connection, name: &str| {
            conn.query_row(
                "SELECT 1 FROM pragma_table_info('repositories') WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap_or(false)
        };
        if !column_exists(conn, "unresolved_internal") {
            conn.execute_batch("ALTER TABLE repositories ADD COLUMN unresolved_internal INTEGER")?;
        }
        if !column_exists(conn, "unresolved_external") {
            conn.execute_batch("ALTER TABLE repositories ADD COLUMN unresolved_external INTEGER")?;
        }
        conn.pragma_update(None, "user_version", 10)?;
        conn.execute_batch("COMMIT")?;
    }

    // Set pragmas for better concurrency and data integrity.
    // foreign_keys must be set per-connection as SQLite resets it on each open.
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;

    Ok(())
}

#[cfg(test)]
mod schema_tests_basics;
#[cfg(test)]
mod schema_tests_migrations;

#[cfg(test)]
mod schema_tests_v2_v3;
#[cfg(test)]
mod schema_tests_v7_v8;

#[cfg(test)]
mod schema_tests_v9_v10;
