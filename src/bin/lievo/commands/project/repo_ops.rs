// Repository-level operations: add, link, list, delete repos

use std::path::Path;

use super::json_escape;
use lievo::config::RepoConfig;
use lievo::identity::derive_identity;
use lievo::mcp::repo_resolution::{RegistrationRequest, register};
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::{LievoError, Result};

pub fn delete_repo(
    storage: &dyn Storage,
    repo_name: &str,
    project_name: &str,
    force: bool,
) -> Result<()> {
    let project = storage
        .get_project(project_name)?
        .ok_or_else(|| LievoError::ProjectNotFound(project_name.to_string()))?;

    // Find the repo by name within the project
    let repos = storage.list_repos(&project.id)?;
    let matching: Vec<_> = repos.iter().filter(|r| r.name == repo_name).collect();
    let repo = match matching.len() {
        0 => return Err(LievoError::RepoNotFound(repo_name.to_string())),
        1 => matching[0],
        _ => {
            return Err(LievoError::InvalidInput(format!(
                "multiple repos named '{}' in project '{}' — use `lievo list-repos` to find the repo ID",
                repo_name, project_name
            )));
        }
    };

    if !force {
        print!(
            "Are you sure you want to delete repository '{}' and all its data? [y/N] ",
            repo_name
        );
        use std::io::{self, BufRead, Write};
        io::stdout().flush().map_err(LievoError::Io)?;
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(LievoError::Io)?;
        let answer = line.trim();
        if answer != "y" && answer != "Y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    let stats = storage.delete_repo(&repo.id)?;
    println!(
        "Deleted repository '{}' ({} entities, {} relationships removed)",
        repo_name, stats.entities_deleted, stats.relationships_deleted
    );
    if stats.index_dirs_removed > 0 {
        println!(
            "Removed {} index director{}",
            stats.index_dirs_removed,
            if stats.index_dirs_removed == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    Ok(())
}

/// Whether two paths refer to the same on-disk location (raw comparison or
/// canonicalised). Used for the idempotent "already registered at this path"
/// check in `add_repo` (issue #29 CLI explicit project name).
fn is_same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let ca = a.canonicalize().ok();
    let cb = b.canonicalize().ok();
    ca == Some(b.to_path_buf()) || ca == cb
}

pub fn add_repo(storage: &dyn Storage, path: &Path, project_name: Option<&str>) -> Result<()> {
    // Validate path exists and is a git repo (bare rejected).
    let canonical = path
        .canonicalize()
        .map_err(|_| LievoError::InvalidRepoPath(path.display().to_string()))?;

    let git_repo = git2::Repository::open(&canonical)
        .map_err(|_| LievoError::InvalidRepoPath(canonical.display().to_string()))?;
    // Bare repos have no working tree and are useless for source analysis.
    if git_repo.is_bare() {
        return Err(LievoError::InvalidRepoPath(format!(
            "{} is a bare repository",
            canonical.display()
        )));
    }

    // Explicit project-name validation (issue #29): the name is free, or
    // already owned by a repo at THIS path (idempotent success). A project
    // name that exists but contains no repo at this path is treated as a
    // hard error ONLY if it is owned by a different identity; otherwise the
    // shared registration will add the repo under that project name.
    if let Some(explicit) = project_name
        && let Some(project) = storage.get_project(explicit)?
    {
        let repos = storage.list_repos(&project.id)?;
        let already_registered_here = repos
            .iter()
            .any(|r| is_same_path(Path::new(&r.local_path), &canonical));
        if already_registered_here {
            println!(
                "Repository already registered at {} in project '{}'",
                canonical.display(),
                project.name
            );
            return Ok(());
        }
        // If the project already contains a repo at a different path whose
        // recorded identity is non-NULL and differs from this path's derived
        // identity, the name is taken by a different identity — a hard error
        // distinct from the idempotent "already registered here" success.
        // A NULL git_url is NOT an identity, so it cannot conflict.
        let derived = derive_identity(&canonical, &RepoConfig::default());
        let conflicting = repos.iter().any(|r| {
            let stored = Path::new(&r.local_path);
            !is_same_path(stored, &canonical)
                && r.git_url.as_deref().is_some_and(|existing| {
                    existing != derived.as_deref().unwrap_or("") && derived.is_some()
                })
        });
        if conflicting {
            return Err(LievoError::InvalidInput(format!(
                "project name '{explicit}' is already registered at a different path (identity conflict). \
                 Choose a different project name or register this path under its own project."
            )));
        }
    }

    // Delegate to the shared registration function (issue #29).
    let config = RepoConfig::default();
    let request = RegistrationRequest {
        repo_root: &canonical,
        project_name,
        config: &config,
    };
    let resolved = register(storage, &request)?;
    println!(
        "Registered repository '{}' (id: {}) in project '{}'",
        resolved.repo.name, resolved.repo.id, resolved.project.name
    );
    Ok(())
}

pub fn link_repo(
    storage: &dyn Storage,
    project_name: &str,
    repo_id: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let project = storage
        .get_project(project_name)?
        .ok_or_else(|| LievoError::ProjectNotFound(project_name.to_string()))?;

    let repo = storage
        .get_repo(repo_id)?
        .ok_or_else(|| LievoError::RepoNotFound(repo_id.to_string()))?;

    // Path 1: already in the requested project — nothing to do.
    if repo.project_id == project.id {
        match fmt {
            OutputFormat::Json => println!(
                "{{\"action\":\"already_linked\",\"repo_id\":\"{}\",\"repo_name\":\"{}\",\"project\":\"{}\"}}",
                json_escape(&repo.id),
                json_escape(&repo.name),
                json_escape(&project.name)
            ),
            OutputFormat::Human => println!(
                "Repository '{}' is already linked to project '{}'.",
                repo.name, project.name
            ),
        }
        return Ok(());
    }

    // Path 2: repo belongs to a different project — move it.
    let old_project_name = storage
        .list_projects()?
        .into_iter()
        .find(|p| p.id == repo.project_id)
        .map(|p| p.name)
        .unwrap_or_else(|| repo.project_id.clone());

    storage.update_repo_project(&repo.id, &project.id)?;
    match fmt {
        OutputFormat::Json => println!(
            "{{\"action\":\"linked\",\"repo_id\":\"{}\",\"repo_name\":\"{}\",\"project\":\"{}\"}}",
            json_escape(&repo.id),
            json_escape(&repo.name),
            json_escape(&project.name)
        ),
        OutputFormat::Human => println!(
            "Moved repository '{}' from project '{}' to project '{}'.",
            repo.name, old_project_name, project.name
        ),
    }
    Ok(())
}

pub fn list_repos(storage: &dyn Storage, project_name: Option<&str>) -> Result<()> {
    let projects = if let Some(name) = project_name {
        let p = storage
            .get_project(name)?
            .ok_or_else(|| LievoError::ProjectNotFound(name.to_string()))?;
        vec![p]
    } else {
        storage.list_projects()?
    };

    let mut found_any = false;
    for project in &projects {
        let repos = storage.list_repos(&project.id)?;
        for repo in &repos {
            if !found_any {
                println!(
                    "{:<36}  {:<20}  {:<30}  Status",
                    "Repo ID", "Name", "Project"
                );
                println!("{}", "-".repeat(100));
                found_any = true;
            }
            let status = match &repo.last_analyzed_commit {
                Some(c) => format!("analyzed ({})", &c[..c.len().min(8)]),
                None => "not analyzed".to_string(),
            };
            println!(
                "{:<36}  {:<20}  {:<30}  {}",
                repo.id, repo.name, project.name, status
            );
        }
    }

    if !found_any {
        println!("No repositories found.");
    }
    Ok(())
}
