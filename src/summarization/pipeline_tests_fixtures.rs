// Test fixtures and helpers for summarization pipeline tests.

use crate::model::{Entity, EntityTier, ProjectId, RepoId};
use crate::storage::Storage;
use std::cell::RefCell;
use std::collections::HashMap;

pub fn make_file_entity(id: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: ProjectId::from("test-project"),
        repo_id: Some(RepoId::from("test-repo")),
        tier: EntityTier::File,
        parent_id: None,
        name: path.split('/').next_back().unwrap_or(path).to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

pub fn make_fn_entity(id: &str, _file_id: &str, name: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: ProjectId::from("test-project"),
        repo_id: Some(RepoId::from("test-repo")),
        tier: EntityTier::Function,
        parent_id: None,
        name: name.to_string(),
        path: None,
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

pub fn make_module_entity(id: &str, name: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: ProjectId::from("test-project"),
        repo_id: Some(RepoId::from("test-repo")),
        tier: EntityTier::Module,
        parent_id: None,
        name: name.to_string(),
        path: None,
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

/// Per-test apfel invocation log (issue #869). One line per invocation;
/// appended by the fake apfel script itself (see
/// [`logging_fake_apfel_script`]), so the count is owned by the test's own
/// TempDir and cannot be inflated by a concurrent test's subprocess.
pub fn fake_apfel_invocation_log(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("apfel_invocations.log")
}

/// The POSIX sh prefix every logging fake apfel script starts with. On each
/// invocation it appends one line to the file named by the caller-supplied
/// invocation-log path (the test's own TempDir, issue #869) and then runs
/// the caller-supplied body. The caller path (no `$()`, no backticks) is
/// safe against injection by the script's `$(cat)` body — the shell has
/// already split it into the `echo >>` and body commands before `cat` runs.
const FAKE_APFEL_LOG_PREFIX: &str = "echo 1 >> \"";

/// Build a logging fake apfel script: invocation log line, then body.
/// `invocation_log` must be a path with no embedded double quotes or
/// backticks (the test's own TempDir — see [`fake_apfel_invocation_log`]).
///
/// The log path sits on its own line in the generated script, so `sh`
/// tokenizes and runs the `echo >>` command BEFORE any `$(cat)` in the body
/// is expanded — there is no injection path regardless of the body.
pub fn logging_fake_apfel_script(invocation_log: &std::path::Path, body: &str) -> String {
    format!(
        "#!/bin/sh\n{FAKE_APFEL_LOG_PREFIX}{}\"\n{}",
        invocation_log.display(),
        body
    )
}

/// Count the invocations recorded in the per-test invocation log.
pub fn read_fake_apfel_invocations(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

/// RAII guard that restores PATH and releases the `TempDir` (which removes
/// the directory on drop — the held binding is what keeps the unique path
/// alive for the whole test, issue #863).
pub struct FakeApfelPathGuard {
    pub orig_path: String,
    pub _dir: tempfile::TempDir,
}

impl Drop for FakeApfelPathGuard {
    fn drop(&mut self) {
        unsafe { std::env::set_var("PATH", self.orig_path.clone()) };
    }
}

impl FakeApfelPathGuard {
    /// Install a no-op fake `apfel` on PATH that logs each invocation to the
    /// per-test invocation log (issue #869). The caller must hold the
    /// crate-wide env lock. Returns the log path and the guard.
    pub fn install_noop_fake_apfel() -> (std::path::PathBuf, Self) {
        let dir_tmp = tempfile::TempDir::new().unwrap();
        let bin = dir_tmp.path().join("apfel");
        let log = fake_apfel_invocation_log(dir_tmp.path());
        write_fake_apfel(
            &bin,
            &logging_fake_apfel_script(&log, "echo '{\"content\": \"unused\"}'\n"),
        );
        let orig_path = std::env::var("PATH").unwrap_or_default();
        unsafe {
            std::env::set_var(
                "PATH",
                format!("{}:{}", dir_tmp.path().display(), orig_path),
            )
        };
        (
            log,
            Self {
                orig_path,
                _dir: dir_tmp,
            },
        )
    }
}

/// Write a fake apfel script to `path`, close the write handle, then make
/// the file executable.
///
/// The write handle MUST be closed (dropped) before the script is exec'd,
/// otherwise the kernel returns ETXTBSY (os error 26) when the child
/// execs a file that is still open for writing in the calling process.
/// The tempdir is per-call, so no two tests share the same path.
pub fn write_fake_apfel(path: &std::path::Path, script: &str) {
    use std::io::Write;
    {
        let mut f = std::fs::File::create(path).expect("create fake apfel");
        f.write_all(script.as_bytes()).expect("write fake apfel");
        f.sync_all().expect("sync fake apfel");
        // Drop the file handle before setting permissions and spawning to
        // avoid ETXTBSY (os error 26) when the kernel execs the script
        // while the write handle is still open.
        drop(f);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }
}

/// Configurable in-memory storage for summarization fallback tests.
/// - `files`: path → Entity for entity_by_path lookups
/// - `functions`: fn_entity_id → Entity for get_entity lookups (fn- prefix only)
/// - `upserted`: tracks all upsert_entity calls
pub struct TestStorage {
    pub files: HashMap<String, Entity>,
    pub functions: HashMap<String, Entity>,
    pub upserted: RefCell<Vec<Entity>>,
    /// Additional entity lookups: (repo_id, tier) → Vec<Entity> for entities_by_repo,
    /// parent_id → Vec<Entity> for entities_by_parent.
    pub repo_tier_entities: RefCell<HashMap<String, Vec<Entity>>>,
    pub parent_children: RefCell<HashMap<String, Vec<Entity>>>,
    /// Repo lookups: repo_id → Repository for get_repo (issue #827 source seam).
    pub repos: RefCell<HashMap<String, crate::model::Repository>>,
}

// Re-export CodeUnit for test modules
pub use crate::model::CodeUnit;

impl TestStorage {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            functions: HashMap::new(),
            upserted: RefCell::new(Vec::new()),
            repo_tier_entities: RefCell::new(HashMap::new()),
            parent_children: RefCell::new(HashMap::new()),
            repos: RefCell::new(HashMap::new()),
        }
    }

    /// Register a repo lookup for `get_repo` (issue #827): the repository's
    /// `local_path` is what the `read_source` seam joins the entity path onto.
    pub fn add_repo(&mut self, repo_id: &str, local_path: &str) {
        self.repos.borrow_mut().insert(
            repo_id.to_string(),
            crate::model::Repository {
                id: repo_id.to_string(),
                project_id: "test-project".to_string(),
                name: repo_id.to_string(),
                git_url: None,
                local_path: local_path.to_string(),
                default_branch: "main".to_string(),
                last_analyzed_commit: None,
                index_path: None,
                summarization_unconfigured: None,
                created_at: "2025-01-01T00:00:00Z".to_string(),
                updated_at: "2025-01-01T00:00:00Z".to_string(),
            },
        );
    }

    pub fn add_file(&mut self, repo_id: &str, path: &str, entity: Entity) {
        let key = format!("{}:{}", repo_id, path);
        self.files.insert(key, entity);
    }

    pub fn add_function(&mut self, id: &str, entity: Entity) {
        self.functions.insert(id.to_string(), entity);
    }

    /// Add entities returned by entities_by_repo for a given repo + tier.
    pub fn add_repo_tier_entities(
        &mut self,
        repo_id: &str,
        tier: EntityTier,
        entities: Vec<Entity>,
    ) {
        let key = format!("{}:{:?}", repo_id, tier);
        self.repo_tier_entities.borrow_mut().insert(key, entities);
    }

    /// Add children returned by entities_by_parent for a given parent ID.
    pub fn add_children(&mut self, parent_id: &str, children: Vec<Entity>) {
        self.parent_children
            .borrow_mut()
            .entry(parent_id.to_string())
            .or_default()
            .extend(children);
    }
}

impl Storage for TestStorage {
    fn get_entity(&self, id: &str) -> crate::Result<Option<Entity>> {
        if id.starts_with("fn-") {
            Ok(self.functions.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    fn get_project(&self, _name: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }

    fn entity_by_path(&self, repo_id: &str, path: &str) -> crate::Result<Option<Entity>> {
        let key = format!("{}:{}", repo_id, path);
        Ok(self.files.get(&key).cloned())
    }

    fn entity_ids_for_paths(
        &self,
        _repo_id: &str,
        _paths: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }

    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }

    fn upsert_entity(&self, entity: &Entity) -> crate::Result<()> {
        self.upserted.borrow_mut().push(entity.clone());
        Ok(())
    }
    fn clear_entity_summary(&self, _entity_id: &str) -> crate::Result<()> {
        Ok(())
    }

    fn entities_by_repo(
        &self,
        repo_id: &str,
        tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>> {
        let tier = tier.expect("entities_by_repo always called with Some tier in rollup_to_tier");
        let key = format!("{}:{:?}", repo_id, tier);
        Ok(self
            .repo_tier_entities
            .borrow()
            .get(&key)
            .cloned()
            .unwrap_or_default())
    }
    fn entities_by_parent(&self, parent_id: &str) -> crate::Result<Vec<Entity>> {
        Ok(self
            .parent_children
            .borrow()
            .get(parent_id)
            .cloned()
            .unwrap_or_default())
    }
    fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<crate::model::Project> {
        unimplemented!()
    }

    fn get_project_by_id(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }

    fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
        unimplemented!()
    }
    fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<crate::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, repo_id: &str) -> crate::Result<Option<crate::model::Repository>> {
        Ok(self.repos.borrow().get(repo_id).cloned())
    }
    fn list_repos(&self, _: &str) -> crate::Result<Vec<crate::model::Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn search_entities_by_name(
        &self,
        _: &str,
        _: &[&str],
        _: usize,
        _: Option<&str>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _: &crate::model::Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn relationships_to(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _: &crate::model::Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> crate::Result<Vec<crate::model::Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(&self, _: &str, _: &str) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _: &crate::model::AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _: &[&Entity],
        _: &[crate::model::Relationship],
        _: &crate::model::AnalysisRun,
        _: &str,
        _: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn delete_repo(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _: &str,
        _: &str,
        _: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn add_output_dir(&self, _: &str, _: &str) -> crate::Result<()> {
        Ok(())
    }
    fn get_output_dirs(&self, _: &str) -> crate::Result<Vec<String>> {
        Ok(vec![])
    }
    fn get_all_file_hashes(
        &self,
        _repo_id: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }
    fn delete_file_hash(&self, _repo_id: &str, _file_path: &str) -> crate::Result<()> {
        Ok(())
    }
    fn count_missing_summaries(&self, _repo_id: &str) -> crate::Result<u64> {
        Ok(0)
    }
}
