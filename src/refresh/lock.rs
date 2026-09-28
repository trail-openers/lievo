//! Per-repo cross-process indexing lock and status file (issue #864).
//!
//! The lock is an OS advisory lock (std::fs::File::try_lock, MSRV 1.89) on
//! a file under the lievo data dir, one per repo. The OS releases the lock
//! when the holder dies — that is the ONLY stale-lock mechanism (no signal
//! handlers, no reliance on Drop). "Lock file exists" is NOT "lock held":
//! a process that acquires a lock left by a dead holder starts a fresh
//! refresh transparently; "indexing in progress" is reported ONLY when
//! try_lock fails against a live holder.
//!
//! The holder writes a status file beside the lock (`started_at`,
//! `trigger`); observers read it via `indexing_status`, which probes the
//! lock without holding it past the probe. Shared by `lievo_explore`'s
//! in-progress response and `lievo doctor` (#865).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The trigger recorded in the status file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexTrigger {
    /// The repo had never been indexed; a full first index is running.
    FirstIndex,
    /// The index existed but was stale; an incremental refresh is running.
    IncrementalRefresh,
}

impl IndexTrigger {
    /// Label written to the status file and reported to observers.
    pub fn as_str(self) -> &'static str {
        match self {
            IndexTrigger::FirstIndex => "first_index",
            IndexTrigger::IncrementalRefresh => "incremental_refresh",
        }
    }
}

/// A read-only observation of the per-repo indexing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexingStatus {
    /// No process currently holds this repo's index lock.
    NotRunning,
    /// A live process holds the lock; the status file was readable.
    Running {
        started_at: u64,
        trigger: IndexTrigger,
    },
    /// A live process holds the lock; the status file is missing or
    /// unreadable — `elapsed` is omitted rather than invented.
    RunningUnknown,
}

impl IndexingStatus {
    /// Seconds the index has been running; `None` when unknown.
    pub fn elapsed_secs(&self) -> Option<u64> {
        let IndexingStatus::Running { started_at, .. } = self else {
            return None;
        };
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|now| now.as_secs().saturating_sub(*started_at))
    }
}

/// Per-repo lock file path under the lievo data dir.
pub fn lock_path(repo_path: &Path) -> Result<PathBuf, std::io::Error> {
    lock_path_in_dir(repo_path, &data_dir()?)
}

/// Per-repo lock file path under an explicit directory. `acquire_in_dir` / `indexing_status_in_dir` use this so tests can point the lock at a unique temp dir instead of the real data dir (issue #864 follow-up: the data dir is a hash of the repo path, so tests that share it interfere with each other).
pub(crate) fn lock_path_in_dir(repo_path: &Path, dir: &Path) -> Result<PathBuf, std::io::Error> {
    Ok(dir.join(format!("lock-{}.lock", repo_hash(repo_path)?)))
}

/// Per-repo status file path (beside the lock).
pub fn status_path(repo_path: &Path) -> Result<PathBuf, std::io::Error> {
    status_path_in_dir(repo_path, &data_dir()?)
}

/// Per-repo status file path under an explicit directory (see `lock_path_in_dir`).
pub(crate) fn status_path_in_dir(repo_path: &Path, dir: &Path) -> Result<PathBuf, std::io::Error> {
    Ok(dir.join(format!("status-{}.json", repo_hash(repo_path)?)))
}

/// Hash-suffix key shared by the lock and status file names. `repo_hash`
/// canonicalises the repo path, so the hash is stable across invocations
/// and across any number of `lievo mcp` processes on the same repo.
fn repo_hash(repo_path: &Path) -> std::io::Result<String> {
    crate::extraction::repo_hash(repo_path).map_err(std::io::Error::other)
}

fn data_dir() -> Result<PathBuf, std::io::Error> {
    crate::extraction::lievo_data_dir().map_err(std::io::Error::other)
}

/// Try to acquire the per-repo index lock. On success the caller owns the
/// File handle for the duration of the refresh; dropping it (or process
/// death) releases the OS lock. The status file is written (atomically via
/// temp-file + rename) before returning, so an observer that sees a held
/// lock almost always also sees the status.
pub fn acquire(
    repo_path: &Path,
    trigger: IndexTrigger,
    data_dir: &Path,
) -> Result<std::fs::File, std::io::Error> {
    acquire_in_dir(repo_path, trigger, data_dir)
}

/// Acquire the per-repo index lock with the lock/status files written under
/// `dir` instead of the lievo data dir. Production callers use
/// `acquire` (the data-dir path); tests pass a unique temp dir so parallel
/// runs of the same test (and other tests) can never share a lock file.
fn acquire_in_dir(
    repo_path: &Path,
    trigger: IndexTrigger,
    dir: &Path,
) -> Result<std::fs::File, std::io::Error> {
    fs::create_dir_all(dir)?;
    let path = lock_path_in_dir(repo_path, dir)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    // Advisory lock: fails while a LIVE process holds it (TryLockError).
    // A dead holder's lock is released by the OS, so a failed acquire here
    // means a live holder — this is the only "in progress" source.
    if file.try_lock().is_err() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "index lock is held by another live process",
        ));
    };

    let status = status_path_in_dir(repo_path, dir)?;
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let tmp = status.with_extension("tmp");
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        write!(
            f,
            "{{\"started_at\":{started_at},\"trigger\":\"{}\"}}",
            trigger.as_str()
        )?;
    }
    fs::rename(&tmp, &status)?;

    Ok(file)
}

/// Read-only probe: is a live process holding this repo's index lock?
///
/// Opens the lock file strictly read-only (never creates it — a missing
/// lock file means NotRunning and the probe must leave the data dir
/// unchanged), tries a non-blocking lock, and releases the probe
/// immediately. On a successful probe the status file is read for
/// `started_at`/`trigger`.
pub fn indexing_status(repo_path: &Path) -> std::io::Result<IndexingStatus> {
    indexing_status_in_dir(repo_path, &data_dir()?)
}

/// Probe the index lock state with the lock/status files read from `dir`
/// instead of the lievo data dir (see `acquire_in_dir`).
fn indexing_status_in_dir(repo_path: &Path, dir: &Path) -> std::io::Result<IndexingStatus> {
    if !dir.exists() {
        return Ok(IndexingStatus::NotRunning);
    }
    let path = lock_path_in_dir(repo_path, dir)?;
    // Read-only open: this probe must never create the lock file — a
    // missing lock file means nobody has ever indexed here in this data
    // dir, and `acquire` is the only path that creates the file. (A lock
    // file created by an in-flight acquire is held by that process from
    // open to try_lock; a probe racing it sees not-yet-locked, which is
    // indistinguishable from not-running and resolves itself within
    // microseconds.)
    let probe = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(IndexingStatus::NotRunning);
        }
        Err(e) => return Err(e),
    };
    // Try to lock the file: if it succeeds, nobody live holds the lock
    // (the lock is released immediately after the probe, so the File is
    // dropped at end of scope); if it fails, a LIVE process holds the
    // lock — read the status file for the payload.
    match probe.try_lock() {
        Ok(()) => Ok(IndexingStatus::NotRunning),
        Err(_) => {
            let status = status_path_in_dir(repo_path, dir).ok().and_then(|p| {
                fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            });
            match status {
                Some(v) => {
                    let started_at = v.get("started_at").and_then(|x| x.as_u64()).unwrap_or(0);
                    let trigger = v
                        .get("trigger")
                        .and_then(|t| t.as_str())
                        .filter(|t| *t == "first_index")
                        .map_or(IndexTrigger::IncrementalRefresh, |_| {
                            IndexTrigger::FirstIndex
                        });
                    Ok(IndexingStatus::Running {
                        started_at,
                        trigger,
                    })
                }
                None => Ok(IndexingStatus::RunningUnknown),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (see refresh/lock_tests.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "lock_tests.rs"]
mod tests;
