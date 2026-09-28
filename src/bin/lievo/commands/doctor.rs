// `lievo doctor` handler (issue #865, hardened by #875) — report in one
// screen whether lievo will work here: the repo it resolves (and from which
// source), whether it is registered and indexed, whether the index is
// current, whether a background index is running, and where the data lives,
// with one fix per problem.
//
// Strictly read-only: never registers, indexes, downloads, or holds the
// index lock beyond `indexing_status`'s probe. Doctor opens the storage
// itself and creates nothing — when the database cannot be opened (corrupt
// file, missing LIEVO_DB parent, permission denied, locked) it reports that
// as a problem instead of failing before printing any report.
//
// The JSON `ok` field and the exit code are the same state: ok == (exit
// code == 0). Every error doctor handles becomes a failure problem.

#[path = "doctor_render.rs"]
mod doctor_render;

use std::path::{Path, PathBuf};

use lievo::extraction::{lievo_data_dir, ts_index_dir_for_repo};
use lievo::mcp::repo_resolution::{find_repo_by_path, resolve_project_root_from};
use lievo::output::OutputFormat;
use lievo::refresh::lock::{IndexingStatus, indexing_status};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

/// One diagnosed problem: what is wrong, and the single action that fixes it.
/// `is_failure` marks the problems that flip the exit code (issue decision 5).
#[derive(Clone)]
struct Problem {
    what: String,
    fix: &'static str,
    is_failure: bool,
}

/// Open storage the way doctor must, for the resolved database path (which
/// `doctor` computes from LIEVO_DB or the default location — doctor reads
/// the environment once).
///
/// Absent database: `Ok(None)` (fresh install — not a failure, nothing
/// created) or, when the parent directory is missing, `Err` so the report
/// names the missing directory. Present database: open it (SQLite creates
/// the file itself when its parent exists; a present file that fails to
/// open is corrupt or locked).
fn open_storage_for_doctor(
    db: &Path,
    from_env: bool,
) -> Result<Option<SqliteStorage>, lievo::LievoError> {
    // An absent database is never opened: SQLite would create it, and doctor
    // must not change anything. Absent default DB, or an absent LIEVO_DB file
    // inside an existing directory: not created yet — Ok(None), no failure.
    // An absent LIEVO_DB whose parent is missing or not a directory: the open
    // fails without creating anything, and the report names the cause.
    let parent_is_dir = db.parent().is_some_and(|p| p.is_dir());
    if !db.exists() && (!from_env || parent_is_dir) {
        return Ok(None);
    }
    Ok(Some(SqliteStorage::open_at(db)?))
}

/// Run `lievo doctor [PATH]` and print the human or JSON report.
///
/// Returns `Ok(0)` when lievo will work here, `Ok(1)` when it will not
/// (outside a git repository, the database cannot be opened, or the data
/// directory is not writable). Warnings such as "not yet registered" or
/// "stale index" do not fail.
pub fn doctor(path: Option<&str>, fmt: OutputFormat) -> lievo::Result<i32> {
    let start_dir = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    // The behaviour-changing variables the build reads — each reported only
    // when set (CLAUDE_PROJECT_DIR is NOT listed here; it appears only as a
    // repo_source value, per issue decision 2).
    let env_vars: Vec<(&'static str, String)> = [
        "LIEVO_DB",
        "LIEVO_NO_REFRESH",
        "LIEVO_MCP_TOOLS",
        "LIEVO_PROJECT_DIR",
    ]
    .into_iter()
    .filter_map(|var| std::env::var(var).ok().map(|v| (var, v)))
    .collect();
    // Same gate as src/refresh/auto.rs: only the exact value "1" disables
    // automatic indexing (LIEVO_NO_REFRESH=0 leaves it enabled).
    let no_refresh = std::env::var("LIEVO_NO_REFRESH").is_ok_and(|v| v == "1");

    // Database path actually used — LIEVO_DB (read once, above) when set
    // and non-empty, else ~/.lievo/lievo.db. The data dir is its parent.
    // `db_from_env` is true only when LIEVO_DB is set AND non-empty.
    let (db_path, db_from_env) = match env_vars.iter().find(|(k, _)| *k == "LIEVO_DB") {
        Some((_, v)) if !v.is_empty() => (PathBuf::from(v), true),
        _ => (
            lievo_data_dir()
                .ok()
                .map(|d| d.join("lievo.db"))
                .unwrap_or_else(|| PathBuf::from("~/.lievo/lievo.db")),
            false,
        ),
    };
    let data_dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut problems: Vec<Problem> = Vec::new();

    // Database open: doctor opens storage itself and creates nothing (issue
    // #875 D1). Any failure is one "database cannot be opened" failure with
    // a cause-specific fix. With LIEVO_DB unset and the default absent, this
    // is a fresh install — not a failure: the DB is created on first use and
    // registration is reported false.
    let storage = match open_storage_for_doctor(&db_path, db_from_env) {
        Ok(storage) => {
            // An absent database (Ok(None)) is not a failure — but the report
            // says it does not exist yet, with the right origin for each case:
            // the default path (created on first use under the data dir), or
            // the LIEVO_DB path itself (created on first use at that path).
            // A present database is opened read-only and needs no line.
            if storage.is_none() && !db_path.exists() {
                let what = if db_from_env {
                    format!(
                        "{} does not exist yet; it will be created on first use",
                        db_path.display()
                    )
                } else {
                    "database does not exist yet; it will be created on first use".to_string()
                };
                problems.push(Problem {
                    what,
                    fix: "nothing to do — the database is created on first use",
                    is_failure: false,
                });
            }
            storage
        }
        Err(e) => {
            // "!parent.is_dir()" covers both a missing parent and one that
            // exists but is not a directory; the next branch distinguishes
            // the second case so each cause gets its own fix.
            let db_parent = db_path.parent().filter(|p| !p.as_os_str().is_empty());
            let fix = if db_path.exists() && !db_path.is_file() {
                "database path is not a file; move it aside and let lievo recreate the database"
            } else if db_path.exists() {
                "database file is corrupt or locked; move it aside (or fix permissions/locks) and let lievo recreate it"
            } else if db_parent.is_some_and(|p| p.is_file()) {
                "the database path's parent is not a directory; point LIEVO_DB at a file inside an existing directory"
            } else if db_parent.is_some_and(|p| !p.is_dir()) {
                "database file does not exist and its parent directory is missing; create the directory or correct LIEVO_DB"
            } else {
                "fix the directory's permissions, or set LIEVO_DB to a writable path"
            };
            problems.push(Problem {
                what: format!("database cannot be opened: {e}"),
                fix,
                is_failure: true,
            });
            None
        }
    };

    // "data directory not writable" is reported only when the directory
    // exists and the write probe fails — never double-report a missing dir
    // (that is a "database cannot be opened" failure above).
    let db_dir_exists = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .is_some_and(|p| p.is_dir());
    let db_writable = db_dir_exists && path_is_writable(&db_path);
    if db_dir_exists && !db_writable {
        problems.push(Problem {
            what: "data directory not writable".to_string(),
            fix: "set LIEVO_DB to a writable path, or fix the directory's permissions",
            is_failure: true,
        });
    }

    // Repo resolution (issue #863) — exactly what `lievo mcp` would do if
    // launched from PATH (or cwd).
    let resolved = match resolve_project_root_from(&start_dir) {
        Ok(r) => Some(r),
        Err(not_in_git) => {
            let not_git_error = not_in_git.checked.exists();
            let what = if not_git_error {
                format!(
                    "{} is not inside a git repository",
                    not_in_git.checked.display()
                )
            } else {
                format!(
                    "{} does not exist (and is not inside a git repository)",
                    not_in_git.checked.display()
                )
            };
            problems.push(Problem {
                what,
                fix: "open your agent in a git repository, or set LIEVO_PROJECT_DIR to one",
                is_failure: true,
            });
            None
        }
    };
    let repo_root = resolved.as_ref().map(|r| r.root.clone());
    let source_label = resolved.as_ref().map(|r| r.source);

    // Registration: the canonical resolved path matches a stored local_path.
    let (registered, project_name, last_analyzed_commit, entity_count) = match (
        &repo_root,
        storage.as_ref(),
    ) {
        (Some(root), Some(storage)) => match find_repo_by_path(storage, root) {
            Ok(Some(repo)) => {
                let project = storage
                    .get_project_by_id(&repo.project_id)
                    .ok()
                    .flatten()
                    .map(|p| p.name)
                    .unwrap_or_default();
                let entity_count = storage.count_entities(&repo.id).ok();
                (
                    true,
                    Some(project),
                    repo.last_analyzed_commit.clone(),
                    entity_count,
                )
            }
            Ok(None) => (false, None, None, None),
            Err(e) => {
                problems.push(Problem {
                        what: format!("could not look up registration: {e}"),
                        fix: "run `lievo doctor` again; if it persists, delete the database file and let lievo recreate it",
                        is_failure: true,
                    });
                (false, None, None, None)
            }
        },
        _ => (false, None, None, None),
    };

    // Index state: stored last_analyzed_commit vs HEAD, plain comparison. An
    // unborn HEAD (a git repo with no commits yet) is a distinct,
    // documented state: "unborn".
    let head_commit = repo_root.as_deref().and_then(git_head);
    let head_is_unborn = head_commit.is_none() && repo_root.as_deref().is_some_and(is_unborn_head);
    let index_state = if head_is_unborn {
        Some("unborn".to_string())
    } else {
        match (&repo_root, &last_analyzed_commit) {
            (Some(_), Some(stored)) => head_commit.as_ref().map(|head| {
                if stored == head || stored.starts_with(&head[..stored.len().min(head.len())]) {
                    "current".to_string()
                } else {
                    "stale".to_string()
                }
            }),
            (Some(_), None) => Some("never_indexed".to_string()),
            (None, _) => None,
        }
    };
    if head_is_unborn {
        problems.push(Problem {
            what: "repo has no commits yet (unborn HEAD)".to_string(),
            fix: "make the repository's first commit; lievo indexes after the first commit",
            is_failure: false,
        });
    }

    if registered {
        // Registered repos need no problem line.
    } else if repo_root.is_some() && !head_is_unborn {
        problems.push(Problem {
            what: "git repo not registered".to_string(),
            fix: "`lievo mcp` will register and index it automatically on first use",
            is_failure: false,
        });
    }
    if let (Some(_), Some(state)) = (repo_root.as_ref(), &index_state)
        && matches!(state.as_str(), "stale" | "never_indexed")
    {
        problems.push(Problem {
            what: if state == "stale" {
                "index is stale (behind git HEAD)".to_string()
            } else {
                "repo has never been indexed".to_string()
            },
            fix: if no_refresh {
                "automatic indexing is disabled by LIEVO_NO_REFRESH; run `lievo refresh`, or unset it"
            } else {
                "start your agent (or `lievo mcp`) here; indexing runs automatically in the background"
            },
            is_failure: false,
        });
    }

    // Indexing in progress (issue #864) — read-only lock probe. A probe
    // error is a problem line, not a silent "not running".
    let (indexing_in_progress, indexing_elapsed) = match &repo_root {
        Some(root) => match indexing_status(root) {
            Ok(IndexingStatus::NotRunning) => (false, None),
            Ok(status) => (true, status.elapsed_secs()),
            Err(e) => {
                problems.push(Problem {
                    what: format!("could not read indexing status: {e}"),
                    fix: "fix the lievo data directory (permissions, or delete it and let lievo recreate it)",
                    is_failure: true,
                });
                (false, None)
            }
        },
        None => (false, None),
    };

    // Semantic vector index + embedding model on disk (informational only —
    // file existence, never a download, never required for lievo_explore).
    let vector_index_present = repo_root
        .as_ref()
        .and_then(|root| ts_index_dir_for_repo(root).ok())
        .is_some_and(|d| d.join("vectors.usearch").exists());
    let embedding_model_present =
        lievo::retrieval::model_cache::model_dir().is_some_and(|d| d.exists());

    let ok = !problems.iter().any(|p| p.is_failure);

    let report = doctor_render::DoctorReport {
        db_path: db_path.clone(),
        data_dir: data_dir.clone(),
        db_writable,
        repo_root: repo_root.clone(),
        source_label,
        registered,
        project_name,
        index_state,
        entity_count,
        last_analyzed_commit,
        head_commit,
        indexing_in_progress,
        indexing_elapsed,
        env: env_vars,
        vector_index_present,
        embedding_model_present,
        problems,
    };
    match fmt {
        OutputFormat::Json => {
            println!("{}", doctor_render::build_json(&report, ok));
        }
        OutputFormat::Human => {
            doctor_render::print_human(&report);
        }
    }
    Ok(exit_code_from(ok))
}

/// Exit code per issue decision 5: 0 when lievo will work here, 1 when not.
fn exit_code_from(ok: bool) -> i32 {
    if ok { 0 } else { 1 }
}

/// Current HEAD of the git repo at `root` (None for an unborn branch or a
/// repo that cannot be opened).
fn git_head(root: &Path) -> Option<String> {
    let repo = git2::Repository::open(root).ok()?;
    let head = repo.head().ok()?;
    // An unborn branch (a repo with no commits yet) has a branch ref that
    // cannot be peeled to a commit: report no HEAD, and let is_unborn_head
    // (below) classify the repo as "unborn" in the index state.
    let oid = head.peel_to_commit().ok()?;
    Some(oid.id().to_string())
}

fn is_unborn_head(root: &Path) -> bool {
    // A repo is unborn when it has no commits yet: `Repository::head`
    // fails with `UnbornBranch` (HEAD points at a branch ref that cannot
    // be peeled to a commit). No `git` binary needed — git2 reads the
    // repository directly.
    match git2::Repository::open(root) {
        Ok(repo) => repo
            .head()
            .err()
            .is_some_and(|e| e.code() == git2::ErrorCode::UnbornBranch),
        Err(_) => false,
    }
}

/// Probe whether `path` (a file) is writable without writing real data.
///
/// Method: create a uniquely named empty temp file inside the parent
/// directory and delete it immediately. Metadata/permission checks alone are
/// unreliable (POSIX file metadata does not expose write permission), so the
/// only reliable probe is a real create; the probe file is empty and removed
/// before returning.
fn path_is_writable(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(
        ".lievo-doctor-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => {
            let _ = std::fs::remove_file(&probe);
            false
        }
    }
}
