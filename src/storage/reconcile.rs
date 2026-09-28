use rusqlite::Connection;
/// Orphan entity reconciliation: removes entities whose files no longer exist on disk.
/// Cascades upward through the hierarchy (file → module → subsystem).
use std::path::Path;

use crate::Result;
use crate::storage::queries as q;

/// Statistics returned by reconciliation describing what was cleaned up.
#[derive(Debug, Default, Clone)]
pub struct ReconcileStats {
    pub entities_removed: u64,
    pub relationships_removed: u64,
}

/// Remove entities whose files no longer exist on disk, cascading upward through the hierarchy.
///
/// # Algorithm
/// 1. Query all file-tier entities for the project
/// 2. For each, check if the file exists at `repo_path / entity.path`
/// 3. Delete orphaned file entities and their relationships
/// 4. Delete module-tier entities if all their file children are gone
/// 5. Delete subsystem-tier entities if all their module children are gone
///
/// Returns stats (entities_removed, relationships_removed).
pub(super) fn reconcile_entities(
    conn: &Connection,
    _project_id: &str,
    repo_id: &str,
    repo_path: &Path,
) -> Result<ReconcileStats> {
    // SAFETY: unchecked_transaction is safe here because SqliteStorage has no reentrant
    // entry points. This method is only called from ensure_fresh() with exclusive access.
    let tx = conn.unchecked_transaction()?;

    let mut stats = ReconcileStats::default();

    // Step 1: Find and delete orphaned file-tier entities
    let file_entities: Vec<(String, String)> = {
        let mut stmt = tx.prepare(q::LIST_FILE_ENTITIES_BY_REPO)?;
        stmt.query_map([repo_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for (entity_id, path) in file_entities {
        let full_path = repo_path.join(&path);
        if !full_path.exists() {
            // Delete relationships referencing this entity
            let rels_from = tx.execute(q::DELETE_RELATIONSHIPS_BY_SOURCE, [&entity_id])?;
            let rels_to = tx.execute(q::DELETE_RELATIONSHIPS_BY_TARGET, [&entity_id])?;
            stats.relationships_removed += (rels_from + rels_to) as u64;

            // Delete the orphaned file entity
            tx.execute(q::DELETE_ENTITY_BY_ID, [&entity_id])?;
            stats.entities_removed += 1;
        }
    }

    // Step 2: Cascade upward — remove modules with no file children
    let module_entities: Vec<String> = {
        let mut stmt = tx.prepare(q::LIST_MODULES_BY_REPO)?;
        stmt.query_map([repo_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for module_id in module_entities {
        let child_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM entities WHERE parent_id = ?1 AND tier = 'file'",
            [&module_id],
            |row| row.get(0),
        )?;

        if child_count == 0 {
            // Delete relationships for this module
            let rels_from = tx.execute(q::DELETE_RELATIONSHIPS_BY_SOURCE, [&module_id])?;
            let rels_to = tx.execute(q::DELETE_RELATIONSHIPS_BY_TARGET, [&module_id])?;
            stats.relationships_removed += (rels_from + rels_to) as u64;

            // Delete the empty module
            tx.execute(q::DELETE_ENTITY_BY_ID, [&module_id])?;
            stats.entities_removed += 1;
        }
    }

    // Step 3: Cascade upward — remove subsystems with no module children
    let subsystem_entities: Vec<String> = {
        let mut stmt = tx.prepare(q::LIST_SUBSYSTEMS_BY_REPO)?;
        stmt.query_map([repo_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    for subsystem_id in subsystem_entities {
        let child_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM entities WHERE parent_id = ?1 AND tier = 'module'",
            [&subsystem_id],
            |row| row.get(0),
        )?;

        if child_count == 0 {
            // Delete relationships for this subsystem
            let rels_from = tx.execute(q::DELETE_RELATIONSHIPS_BY_SOURCE, [&subsystem_id])?;
            let rels_to = tx.execute(q::DELETE_RELATIONSHIPS_BY_TARGET, [&subsystem_id])?;
            stats.relationships_removed += (rels_from + rels_to) as u64;

            // Delete the empty subsystem
            tx.execute(q::DELETE_ENTITY_BY_ID, [&subsystem_id])?;
            stats.entities_removed += 1;
        }
    }

    tx.commit()?;

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconcile_stats_default() {
        let stats = ReconcileStats::default();
        assert_eq!(stats.entities_removed, 0);
        assert_eq!(stats.relationships_removed, 0);
    }

    #[test]
    fn test_reconcile_stats_clone() {
        let stats1 = ReconcileStats {
            entities_removed: 5,
            relationships_removed: 3,
        };
        let stats2 = stats1.clone();
        assert_eq!(stats2.entities_removed, 5);
        assert_eq!(stats2.relationships_removed, 3);
    }
}
