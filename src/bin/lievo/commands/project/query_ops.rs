// Query operations: status command and its JSON formatter.

#[cfg(not(test))]
use super::json_escape;
#[cfg(test)]
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
use lievo::model::{Entity, EntityTier, Repository};
use lievo::output::OutputFormat;
use lievo::retrieval::project_boundary::same_project;
use lievo::storage::Storage;
use lievo::summarization::pipeline::is_test_entity;
use lievo::{LievoError, Result};

pub fn status(storage: &dyn Storage, project_name: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let projects = if let Some(name) = project_name {
        let p = storage
            .get_project(name)?
            .ok_or_else(|| LievoError::ProjectNotFound(name.to_string()))?;
        vec![p]
    } else {
        storage.list_projects()?
    };

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    for project in &projects {
        let repos = storage.list_repos(&project.id)?;
        let repo_count = repos.len();

        // Gather entities per repo (drives tier counts, relationships, and
        // per-repo summary coverage below).
        let all_entities: Vec<Entity> = repos
            .iter()
            .map(|r| storage.entities_by_repo(&r.id, None))
            .collect::<Result<Vec<Vec<Entity>>>>()?
            .into_iter()
            .flatten()
            .collect();
        let total_entities = all_entities.len();
        let subsystem_count = all_entities
            .iter()
            .filter(|e| e.tier == EntityTier::Subsystem)
            .count();
        let module_count = all_entities
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .count();
        let file_count = all_entities
            .iter()
            .filter(|e| e.tier == EntityTier::File)
            .count();

        // Count relationships by summing outgoing edges from each entity.
        // Project boundary (issue #764): only count edges whose target
        // belongs to this project, so a cross-project edge is never tallied.
        let project_id = &project.id;
        let rel_count: usize = all_entities
            .iter()
            .map(|e| -> Result<usize> {
                Ok(storage
                    .relationships_from(&e.id)?
                    .into_iter()
                    .filter(|(_, target)| same_project(project_id, &target.project_id))
                    .count())
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .sum();

        // Derive last-analyzed commit from any repo that has one.
        let last_commit = repos
            .iter()
            .filter_map(|r| r.last_analyzed_commit.as_deref())
            .next();

        // Per-repo summary coverage: function-tier entities (excluding `test_`
        // names, matching the COUNT_MISSING_SUMMARIES query) and how many have
        // a summary. Surfaces as `summary_coverage` so partial summarization
        // is visible in `admin status` (issue #648).
        let repo_coverage: Vec<(&Repository, u64, u64)> = repos
            .iter()
            .map(|r| {
                let functions = storage
                    .entities_by_repo(&r.id, Some(EntityTier::Function))
                    .map_err(|e| {
                        LievoError::InvalidInput(format!(
                            "Failed to read function entities for repo '{}' (project '{}'): {}",
                            r.name, project.name, e
                        ))
                    })?;
                let (total, missing) = functions.iter().fold((0u64, 0u64), |(t, m), e| {
                    let (t, m) = if is_test_entity(&e.name) {
                        (t, m)
                    } else {
                        (t + 1, m + u64::from(e.summary.is_none()))
                    };
                    (t, m)
                });
                Ok::<_, LievoError>((r, total, missing))
            })
            .collect::<Result<Vec<_>>>()?;

        let stats = StatusStats {
            project_name: &project.name,
            repo_count,
            last_commit,
            total_entities,
            subsystem_count,
            module_count,
            file_count,
            rel_count,
            repo_coverage: &repo_coverage,
        };
        match fmt {
            OutputFormat::Json => {
                println!("{}", format_status_json(&stats));
            }
            OutputFormat::Human => {
                println!("Project: {}", project.name);
                if repos.is_empty() {
                    println!("  (no repositories)");
                    continue;
                }
                println!("  Repositories: {}", repo_count);
                match last_commit {
                    Some(c) => println!("  Last analyzed: commit {}", &c[..c.len().min(12)]),
                    None => {
                        println!("  Last analyzed: not analyzed — run `lievo refresh` to index")
                    }
                }
                println!(
                    "  Entities: {} ({} subsystems, {} modules, {} files)",
                    total_entities, subsystem_count, module_count, file_count
                );
                println!("  Relationships: {}", rel_count);
                println!("  Summary coverage:");
                for (repo, total, missing) in &repo_coverage {
                    println!(
                        "    {}: {}/{} functions summarized ({} missing)",
                        repo.name,
                        total.saturating_sub(*missing),
                        total,
                        missing,
                    );
                }
            }
        }
    }
    Ok(())
}

/// Per-project counts and per-repo summary coverage gathered by `status`.
pub struct StatusStats<'a> {
    pub project_name: &'a str,
    pub repo_count: usize,
    pub last_commit: Option<&'a str>,
    pub total_entities: usize,
    pub subsystem_count: usize,
    pub module_count: usize,
    pub file_count: usize,
    pub rel_count: usize,
    /// Per-repo (repo, total_functions, missing_summaries).
    pub repo_coverage: &'a [(&'a Repository, u64, u64)],
}

/// Build the one-line JSON object for a project in `admin status` output.
///
/// Kept separate from `status` so the `summary_coverage` field is testable
/// without capturing process stdout (issue #648).
pub fn format_status_json(stats: &StatusStats<'_>) -> String {
    let commit_field = match stats.last_commit {
        Some(c) => format!("\"{}\"", json_escape(&c[..c.len().min(12)])),
        None => "null".to_string(),
    };
    let coverage_field = stats
        .repo_coverage
        .iter()
        .map(|(repo, total, missing)| {
            format!(
                "{{\"repo\":\"{}\",\"total_functions\":{},\"summarized\":{},\"missing_summaries\":{}}}",
                json_escape(&repo.name),
                total,
                total.saturating_sub(*missing),
                missing,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"project\":\"{}\",\"repositories\":{},\"last_analyzed_commit\":{},\"entities\":{{\"total\":{},\"subsystems\":{},\"modules\":{},\"files\":{}}},\"relationships\":{},\"summary_coverage\":[{}]}}",
        json_escape(stats.project_name),
        stats.repo_count,
        commit_field,
        stats.total_entities,
        stats.subsystem_count,
        stats.module_count,
        stats.file_count,
        stats.rel_count,
        coverage_field,
    )
}

#[cfg(test)]
mod query_ops_tests {
    use super::{StatusStats, format_status_json, status};
    use lievo::model::{
        AnalysisRun, Convention, Entity, EntityTier, Insight, Project, Relationship, Repository,
    };
    use lievo::output::OutputFormat;
    use lievo::storage::Storage;
    use lievo::summarization::pipeline::is_test_entity;
    use lievo::{LievoError, Result};

    /// Self-contained stub of the `Storage` trait: returns one project with
    /// one repo, and makes `entities_by_repo` fail with a synthetic error for
    /// the function tier.
    struct FailingRepoStorage;

    impl Storage for FailingRepoStorage {
        fn create_project(&self, _n: &str, _d: Option<&str>) -> Result<Project> {
            unimplemented!()
        }
        fn get_project(&self, _name: &str) -> Result<Option<Project>> {
            Ok(Some(Project {
                id: "p1".to_string(),
                name: "proj-a".to_string(),
                description: None,
                output_dirs: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
            }))
        }
        fn get_project_by_id(&self, _id: &str) -> Result<Option<Project>> {
            unimplemented!()
        }
        fn list_projects(&self) -> Result<Vec<Project>> {
            unimplemented!()
        }
        fn delete_project(&self, _id: &str) -> Result<lievo::storage::DeleteStats> {
            unimplemented!()
        }
        fn add_output_dir(&self, _p: &str, _d: &str) -> Result<()> {
            unimplemented!()
        }
        fn get_output_dirs(&self, _p: &str) -> Result<Vec<String>> {
            unimplemented!()
        }
        fn add_repo(&self, _p: &str, _n: &str, _l: &str) -> Result<Repository> {
            unimplemented!()
        }
        fn get_repo(&self, _id: &str) -> Result<Option<Repository>> {
            unimplemented!()
        }
        fn list_repos(&self, _project_id: &str) -> Result<Vec<Repository>> {
            Ok(vec![Repository {
                id: "r1".to_string(),
                project_id: "p1".to_string(),
                name: "repo-one".to_string(),
                git_url: None,
                local_path: "/tmp".to_string(),
                default_branch: "main".to_string(),
                last_analyzed_commit: None,
                index_path: None,
                summarization_unconfigured: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
            }])
        }
        fn update_repo_index_path(&self, _r: &str, _p: &str) -> Result<()> {
            unimplemented!()
        }
        fn update_repo_last_commit(&self, _r: &str, _c: &str) -> Result<()> {
            unimplemented!()
        }
        fn update_repo_project(&self, _r: &str, _p: &str) -> Result<()> {
            unimplemented!()
        }
        fn delete_repo(&self, _r: &str) -> Result<lievo::storage::DeleteStats> {
            unimplemented!()
        }
        fn upsert_entity(&self, _e: &Entity) -> Result<()> {
            unimplemented!()
        }
        fn clear_entity_summary(&self, _id: &str) -> Result<()> {
            unimplemented!()
        }
        fn get_entity(&self, _id: &str) -> Result<Option<Entity>> {
            unimplemented!()
        }
        fn list_entities(&self, _p: &str, _t: Option<EntityTier>) -> Result<Vec<Entity>> {
            unimplemented!()
        }
        fn search_entities_by_name(
            &self,
            _p: &str,
            _w: &[&str],
            _l: usize,
            _t: Option<&str>,
        ) -> Result<Vec<Entity>> {
            unimplemented!()
        }
        fn entities_by_repo(
            &self,
            _repo_id: &str,
            tier: Option<EntityTier>,
        ) -> Result<Vec<Entity>> {
            // Fail on the function tier (the second call site in status). The
            // first call site (all entities, None tier) succeeds with empty.
            match tier {
                Some(EntityTier::Function) => Err(LievoError::DatabaseLocked),
                _ => Ok(Vec::new()),
            }
        }
        fn entities_by_parent(&self, _p: &str) -> Result<Vec<Entity>> {
            unimplemented!()
        }
        fn entity_by_path(&self, _r: &str, _p: &str) -> Result<Option<Entity>> {
            unimplemented!()
        }
        fn entity_ids_for_paths(
            &self,
            _r: &str,
            _paths: &[&str],
        ) -> Result<std::collections::HashMap<String, String>> {
            unimplemented!()
        }
        fn entity_by_path_projectwide(&self, _p: &str, _path: &str) -> Result<Vec<Entity>> {
            unimplemented!()
        }
        fn delete_entities_by_repo(&self, _r: &str) -> Result<u64> {
            unimplemented!()
        }
        fn delete_entities_by_paths(&self, _r: &str, _p: &[String]) -> Result<u64> {
            unimplemented!()
        }
        fn upsert_relationship(&self, _r: &Relationship) -> Result<()> {
            unimplemented!()
        }
        fn relationships_from(&self, _s: &str) -> Result<Vec<(Relationship, Entity)>> {
            Ok(Vec::new())
        }
        fn relationships_to(&self, _t: &str) -> Result<Vec<(Relationship, Entity)>> {
            unimplemented!()
        }
        fn delete_relationships_by_source(&self, _s: &str) -> Result<u64> {
            unimplemented!()
        }
        fn upsert_insight(&self, _i: &Insight) -> Result<()> {
            unimplemented!()
        }
        fn list_insights(
            &self,
            _p: &str,
            _c: Option<&str>,
            _s: Option<&str>,
            _l: usize,
        ) -> Result<Vec<Insight>> {
            unimplemented!()
        }
        fn invalidate_insights(&self, _p: &str) -> Result<()> {
            unimplemented!()
        }
        fn upsert_convention(&self, _c: &Convention) -> Result<()> {
            unimplemented!()
        }
        fn list_conventions(&self, _p: &str, _c: Option<&str>) -> Result<Vec<Convention>> {
            unimplemented!()
        }
        fn create_analysis_run(&self, _r: &str, _c: &str) -> Result<AnalysisRun> {
            unimplemented!()
        }
        fn update_analysis_run(&self, _r: &AnalysisRun) -> Result<()> {
            unimplemented!()
        }
        fn get_file_hash(&self, _r: &str, _f: &str) -> Result<Option<String>> {
            unimplemented!()
        }
        fn upsert_file_hash(&self, _r: &str, _f: &str, _h: &str) -> Result<()> {
            unimplemented!()
        }
        fn get_all_file_hashes(
            &self,
            _r: &str,
        ) -> Result<std::collections::HashMap<String, String>> {
            unimplemented!()
        }
        fn delete_file_hash(&self, _r: &str, _f: &str) -> Result<()> {
            unimplemented!()
        }
        fn persist_analysis_batch(
            &self,
            _e: &[&Entity],
            _r: &[Relationship],
            _run: &AnalysisRun,
            _repo_id: &str,
            _commit: &str,
        ) -> Result<(i64, i64)> {
            unimplemented!()
        }
        fn reconcile_entities(
            &self,
            _p: &str,
            _r: &str,
            _path: &std::path::Path,
        ) -> Result<lievo::storage::reconcile::ReconcileStats> {
            unimplemented!()
        }
        fn clear_all_summaries(&self, _p: &str) -> Result<()> {
            unimplemented!()
        }
        fn clear_repo_summaries(&self, _r: &str) -> Result<()> {
            unimplemented!()
        }
        fn count_missing_summaries(&self, _r: &str) -> Result<u64> {
            unimplemented!()
        }
    }

    #[test]
    fn test_is_test_entity_matches_sql_lower_predicate() {
        // The summarization exclusion must agree with the SQL
        // `NOT LOWER(name) LIKE 'test_%'` predicate — case-insensitive,
        // like SQL `LOWER` (issue #648).
        assert!(is_test_entity("test_alpha"));
        assert!(is_test_entity("Test_Alpha"), "must match SQL LOWER(name)");
        assert!(is_test_entity("TEST_alpha"));
        assert!(!is_test_entity("testalpha"));
        assert!(!is_test_entity("spec_test"));
    }

    #[test]
    fn test_status_error_includes_repo_and_project_identification() {
        // Finding #1: per-repo entities_by_repo errors must carry repo
        // identification context so multi-repo failures are diagnosable.
        let s = FailingRepoStorage;
        let err = status(&s, Some("proj-a"), OutputFormat::Human).expect_err("must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("repo-one"),
            "error must name the failing repo: {msg}"
        );
        assert!(msg.contains("proj-a"), "error must name the project: {msg}");
        assert!(
            msg.contains("Failed to read function entities"),
            "error must explain what failed: {msg}"
        );
    }

    #[test]
    fn test_format_status_json_uses_saturating_sub_for_coverage() {
        // Finding #3: `total - missing` must not underflow in the JSON path.
        // If missing > total, summarized should be 0, not panic/overflow.
        let repo = Repository {
            id: "r1".to_string(),
            project_id: "p1".to_string(),
            name: "repo".to_string(),
            git_url: None,
            local_path: "/tmp".to_string(),
            default_branch: "main".to_string(),
            last_analyzed_commit: None,
            index_path: None,
            summarization_unconfigured: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let coverage: [(&Repository, u64, u64); 1] = [(&repo, 0, 1)];
        let stats = StatusStats {
            project_name: "proj",
            repo_count: 1,
            last_commit: None,
            total_entities: 0,
            subsystem_count: 0,
            module_count: 0,
            file_count: 0,
            rel_count: 0,
            repo_coverage: &coverage,
        };
        let out = format_status_json(&stats);
        // saturating_sub: 0 - 1 = 0, so summarized must be 0
        assert!(out.contains("\"summarized\":0"), "saturating_sub: {out}");
        assert!(out.contains("\"missing_summaries\":1"), "{out}");
    }
}
