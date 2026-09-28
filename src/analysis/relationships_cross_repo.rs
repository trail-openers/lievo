// Cross-repository relationship building.
// Matches subsystem names against the FIRST path segment of import paths (crate root).
// Included via #[path] from relationships.rs.

use super::*;

impl RelationshipBuilder {
    /// Builds cross-repository `depends_on` relationships between subsystems.
    ///
    /// Matches subsystem names against the FIRST path segment of import paths (the
    /// crate root name). Only the first `::` segment is considered — matching on inner
    /// segments such as `collections` in `std::collections::HashMap` would produce
    /// false positives when a subsystem shares a name with a common module.
    pub fn build_cross_repo(
        all_subsystems: &[(String, Vec<Entity>)],
        all_code_units: &[(String, Vec<CodeUnit>)],
    ) -> Result<Vec<Relationship>> {
        // subsystem name (lowercased, dashes→underscores) → subsystem entity ID
        let mut name_to_subsystem: HashMap<String, String> = HashMap::new();
        for (_repo, subsystems) in all_subsystems {
            for entity in subsystems {
                let key = normalize_crate_name(&entity.name);
                name_to_subsystem.insert(key, entity.id.clone());
            }
        }

        // For each repo, build a list of (subsystem_path, subsystem_id) sorted by
        // path length descending so longest-prefix wins.
        let mut repo_subsystem_paths: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for (repo, subsystems) in all_subsystems {
            let mut entries: Vec<(&str, &str)> = subsystems
                .iter()
                .filter_map(|e| e.path.as_deref().map(|p| (p, e.id.as_str())))
                .collect();
            // Longest path first so the most-specific subsystem wins
            entries.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
            repo_subsystem_paths.insert(repo.as_str(), entries);
        }

        let mut edge_weights: HashMap<(String, String), u32> = HashMap::new();

        for (repo, units) in all_code_units {
            let sub_paths = repo_subsystem_paths
                .get(repo.as_str())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            for unit in units {
                // Resolve the subsystem that owns this code unit via longest path prefix.
                let src_id = sub_paths.iter().find_map(|(sub_path, id)| {
                    // "." is the root subsystem — it matches everything
                    if *sub_path == "."
                        || unit.file == *sub_path
                        || unit.file.starts_with(&format!("{}/", sub_path))
                    {
                        Some(*id)
                    } else {
                        None
                    }
                });
                let src_id = match src_id {
                    Some(id) => id,
                    None => continue, // unit not associated with any local subsystem
                };

                for import in &unit.imports {
                    // Issue 1 fix: only match the FIRST path segment (crate root),
                    // not all segments. `std::collections::HashMap` should only
                    // match a subsystem named "std", not "collections" or "HashMap".
                    let crate_root = import.split("::").next().unwrap_or(import);
                    let key = normalize_crate_name(crate_root);
                    if let Some(target_id) = name_to_subsystem.get(&key)
                        && src_id != target_id.as_str()
                    {
                        let w = edge_weights
                            .entry((src_id.to_string(), target_id.clone()))
                            .or_insert(0);
                        // Issue 4 fix: saturating add
                        *w = w.saturating_add(1);
                    }
                }
            }
        }

        let rels = edge_weights
            .into_iter()
            .map(|((src, tgt), count)| Relationship {
                source_id: src,
                target_id: tgt,
                rel_type: RelType::DependsOn,
                weight: count as f64,
                evidence_json: None,
                // Cross-repo DependsOn is matched by crate-name string
                // equality against the first import-path segment — a
                // name-matching heuristic, not exact resolution (issue #714).
                provenance: crate::model::EdgeProvenance::Heuristic,
            })
            .collect();

        Ok(rels)
    }
}
