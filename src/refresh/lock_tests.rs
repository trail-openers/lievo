//! Tests for the per-repo cross-process index lock (issue #864).

use super::*;
use std::time::Duration;

/// A tempdir standing in for a repo (canonicalized, so the hash suffix is
/// stable for the test's lifetime).
fn temp_repo() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A unique temp directory standing in for the lievo data dir (lock/status
/// files live under it). Since issue #865 the lock API takes the data dir
/// explicitly, so the lock tests use a unique `TempDir` directly — no HOME
/// guard, no env lock (no env var is mutated), and no way for parallel
/// tests in the same process to share a lock file.
struct DataDir {
    dir: tempfile::TempDir,
}

impl DataDir {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The isolated data dir: a unique temp directory (never the real
    /// `~/.lievo`).
    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

fn acquire(
    dir: &std::path::Path,
    repo: &std::path::Path,
    trigger: crate::refresh::lock::IndexTrigger,
) -> Result<std::fs::File, std::io::Error> {
    crate::refresh::lock::acquire(repo, trigger, dir)
}

fn indexing_status(
    dir: &std::path::Path,
    repo: &std::path::Path,
) -> std::io::Result<crate::refresh::lock::IndexingStatus> {
    crate::refresh::lock::indexing_status_in_dir(repo, dir)
}

/// Poll `indexing_status` until it reports `NotRunning` (bounded 2s wait),
/// then assert `NotRunning`. Why poll: after `drop(holder)` another test may
/// be forking a child at that moment; the child inherits a duplicate of the
/// lock descriptor until it execs (close-on-exec), so the probe can briefly
/// still see a live holder.
fn wait_until_not_running(dir: &std::path::Path, repo: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let status = indexing_status(dir, repo).unwrap();
        if status == crate::refresh::lock::IndexingStatus::NotRunning {
            return;
        }
        assert!(
            deadline.elapsed() < Duration::from_secs(2),
            "indexing_status did not settle to NotRunning within 2s after the holder was dropped (got {status:?})"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn first_acquire_succeeds_second_reports_in_progress() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    let holder = acquire(dir, repo.path(), IndexTrigger::FirstIndex).unwrap();

    // A concurrent probe must report running with the recorded trigger.
    let status = indexing_status(dir, repo.path()).unwrap();
    assert!(
        matches!(
            status,
            IndexingStatus::Running {
                trigger: IndexTrigger::FirstIndex,
                ..
            }
        ),
        "expected Running(first_index), got {status:?}"
    );
    assert_eq!(status.elapsed_secs(), Some(0));

    // A second acquire against a live holder must fail (WouldBlock).
    let err = acquire(dir, repo.path(), IndexTrigger::IncrementalRefresh).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
    // The error message is descriptive for debugging.
    assert!(
        err.to_string().contains("held by another live process"),
        "got: {err}"
    );

    // Release: dropping the File releases the OS lock.
    drop(holder);
    wait_until_not_running(dir, repo.path());
}

#[test]
fn indexing_status_reports_not_running_without_lock_file() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    let status = indexing_status(dir, repo.path()).unwrap();
    assert_eq!(status, IndexingStatus::NotRunning);
    assert_eq!(status.elapsed_secs(), None);
}

/// The probe is strictly read-only: probing an unknown repo (no lock file
/// yet, and the data dir does not even exist) must NOT create the lock
/// file or the data dir — a missing lock file simply means NotRunning
/// (issue #872; the probe was previously opening the file read+write,
/// which created an empty lock file for every repo probed).
#[test]
fn indexing_status_probe_never_creates_files() {
    let data = DataDir::new();
    let dir = data.path();
    let before = std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0);
    let repo = temp_repo();

    let status = indexing_status(dir, repo.path()).unwrap();
    assert_eq!(status, IndexingStatus::NotRunning);

    // The probe must leave the data dir unchanged: no new entries at all
    // (no lock file, no status file, nothing).
    let after = std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(
        after, before,
        "probe must not create any file in the data dir"
    );
    let expected_lock = crate::extraction::repo_hash(repo.path())
        .map(|suffix| dir.join(format!("lock-{suffix}.lock")))
        .map(|p| p.exists())
        .unwrap_or(false);
    assert!(!expected_lock, "probe must not create the lock file");
}

/// The acquire path still creates the lock file on first use (issue #872
/// binding decision: only the read-only probe is side-effect-free).
#[test]
fn acquire_creates_the_lock_file_on_first_use() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    let suffix = crate::extraction::repo_hash(repo.path()).unwrap();
    let lock_file = dir.join(format!("lock-{suffix}.lock"));
    let status_file = dir.join(format!("status-{suffix}.json"));
    assert!(!lock_file.exists(), "data dir must start empty");

    let holder = acquire(dir, repo.path(), IndexTrigger::FirstIndex).unwrap();
    assert!(lock_file.exists(), "acquire must create the lock file");
    assert!(status_file.exists(), "acquire must write the status file");
    drop(holder);

    // After release the probe is read-only again: nothing new may appear.
    wait_until_not_running(dir, repo.path());
}

#[test]
fn indexing_status_reports_running_unknown_without_status_file() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    let holder = acquire(dir, repo.path(), IndexTrigger::IncrementalRefresh).unwrap();
    // Remove the status file: a live holder whose status is unreadable must
    // be reported as running WITHOUT a fabricated started_at (binding
    // decision 4).
    let suffix = crate::extraction::repo_hash(repo.path()).unwrap();
    std::fs::remove_file(dir.join(format!("status-{suffix}.json"))).unwrap();

    let status = indexing_status(dir, repo.path()).unwrap();
    assert_eq!(status, IndexingStatus::RunningUnknown);
    assert_eq!(status.elapsed_secs(), None);
    drop(holder);
}

#[test]
fn status_file_carries_started_at_and_trigger() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let _holder = acquire(dir, repo.path(), IndexTrigger::IncrementalRefresh).unwrap();
    let suffix = crate::extraction::repo_hash(repo.path()).unwrap();
    let raw = std::fs::read_to_string(dir.join(format!("status-{suffix}.json"))).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["trigger"], "incremental_refresh");
    let started = v["started_at"].as_u64().unwrap();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        started >= before && started <= after,
        "started_at {started} outside [{before}, {after}]"
    );
}

/// Stale-lock recovery (binding decision 2): a holder killed with SIGKILL
/// cannot release the lock in userspace — the OS releases it. A subsequent
/// acquire must then succeed within a bounded wait.
#[cfg(unix)]
#[test]
fn sigkilled_holder_releases_lock_within_bounded_wait() {
    let data = DataDir::new();
    let dir = data.path();
    let repo = temp_repo();
    std::fs::create_dir_all(dir).unwrap();
    let suffix = crate::extraction::repo_hash(repo.path()).unwrap();
    let lock_file = dir.join(format!("lock-{suffix}.lock"));
    let status_file = dir.join(format!("status-{suffix}.json"));
    std::fs::write(&lock_file, b"").unwrap();
    std::fs::write(&status_file, b"{}").unwrap();

    // Spawn the current test executable itself as the lock holder: it runs
    // the `lock_holder` test (ignore-marked, so a bare run never picks it
    // up), which acquires the lock via the real `acquire()` path and sleeps
    // until killed. The parent reads "LOCKED", SIGKILLs it, and verifies
    // the OS released the lock. `LIEVO_LOCK_HOLDER_DATA_DIR` points the
    // child at the same isolated dir so it never touches the real data dir.
    let current_exe = std::env::current_exe().expect("current exe path");
    let mut holder = std::process::Command::new(&current_exe)
        .arg("refresh::lock_holder::lock_holder")
        .arg("--exact")
        .arg("--nocapture")
        .env("LIEVO_LOCK_HOLDER_REPO", repo.path().display().to_string())
        .env("LIEVO_LOCK_HOLDER_DATA_DIR", dir.display().to_string())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn lock holder child");
    let holder_pid = holder.id() as i32;

    // Wait until the child prints "LOCKED" (i.e., it holds the lock), then
    // SIGKILL it.
    let mut stdout = holder.stdout.take().unwrap();
    let mut buf = [0u8; 16];
    let mut line = String::new();
    let mut held = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        match std::io::Read::read(&mut stdout, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                line.push_str(&String::from_utf8_lossy(&buf[..n]));
                if line.contains("LOCKED") {
                    held = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(held, "child must take the lock within 5s");
    // SIGKILL the child.
    unsafe {
        libc_kill(holder_pid, 9);
    }
    holder.wait().unwrap();

    // OS released the lock: a bounded-wait acquire must now succeed.
    let probe = std::fs::File::open(&lock_file).unwrap();
    let mut acquired = false;
    for _ in 0..100 {
        if probe.try_lock().is_ok() {
            acquired = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        acquired,
        "lock must be released after the holder is SIGKILLed"
    );
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// Read-during-write (issue #864 decision 8 / binding decision 8): a read
/// issued on a second connection while a refresh write transaction is open
/// must succeed — WAL journal mode + busy_timeout=5000
/// (src/storage/schema/mod.rs) let the serving connection read while the
/// background refresh writes.
#[test]
fn read_during_write_transaction_succeeds() {
    use crate::model::{Entity, EntityTier};
    use crate::storage::Storage;
    use crate::storage::sqlite::SqliteStorage;

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rw.db");

    // Reader connection: opens first, creates project + repo + a file
    // entity, then reads while a writer transaction is open.
    let reader = SqliteStorage::open_at(&db).unwrap();
    let project = reader.create_project("rw-proj", None).unwrap();
    reader
        .add_repo(&project.id, "repo1", "/tmp/rw-repo")
        .unwrap();
    let entity = Entity {
        id: "p:repo1:file:a.rs".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    reader.upsert_entity(&entity).unwrap();

    // Writer thread: open a SECOND connection to the same DB file and run
    // an explicit write transaction (BEGIN IMMEDIATE) on it, hold the
    // transaction briefly, then commit. The reader (main thread) holds the
    // first connection; with WAL journal mode and busy_timeout=5000 the
    // read must succeed even while the writer's transaction is open.
    let writer_db = db.clone();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        use rusqlite::Connection;
        // Open a raw connection and apply the same pragmas the lievo
        // schema sets (busy_timeout=5000, journal_mode=WAL — see
        // src/storage/schema/mod.rs lines 432-433), so the writer
        // connection behaves like the background refresh would.
        let conn = Connection::open(&writer_db).unwrap();
        conn.pragma_update(None, "busy_timeout", "5000")
            .expect("set busy_timeout on writer");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("set WAL mode on writer");
        conn.execute("BEGIN IMMEDIATE", ()).unwrap();
        let _ = conn.execute(
            "INSERT INTO projects (id, name) VALUES (?, ?)",
            rusqlite::params!["writer-proj", "writer-proj"],
        );
        let _ = tx.send(()); // writer's transaction is now open
        std::thread::sleep(Duration::from_millis(300));
        conn.execute("COMMIT", ()).unwrap();
    });

    // Reader waits for the writer to open its transaction, then reads.
    let _ = rx.recv();
    let entities = reader
        .list_entities(&project.id, None)
        .expect("read during an open write transaction must succeed");
    assert_eq!(entities.len(), 1, "reader must see the pre-existing entity");
    let _ = writer.join();
}
