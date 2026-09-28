// End-to-end tests for `lievo doctor` (issue #865). Each test drives the
// real binary as a child process with an isolated environment.

use std::path::PathBuf;
use std::process::Command;

fn lievo_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let debug = exe.parent().and_then(|p| p.parent()).expect("debug dir");
    debug.join("lievo")
}

/// Fresh tempdir with a git repo containing one file + one commit on main.
fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let _ = git_commit(&root, "first");
    (dir, root)
}

/// Create a commit in `root` and return its full hash. If `root` has no
/// `.git` yet, initialize a repo with one file on `main` first.
fn git_commit(root: &std::path::Path, msg: &str) -> String {
    let git = if root.join(".git").exists() {
        git2::Repository::open(root).unwrap()
    } else {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("a.rs"), "fn alpha() {}\n").unwrap();
        let g = git2::Repository::init(root).unwrap();
        g.set_head("refs/heads/main").unwrap();
        g
    };
    let mut index = git.index().unwrap();
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = git.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("lievo-test", "lievo@test").unwrap();
    let head_is_unborn = git
        .head()
        .err()
        .is_some_and(|e| e.code() == git2::ErrorCode::UnbornBranch);
    let parent = if head_is_unborn {
        None
    } else {
        git.head().ok().and_then(|h| h.peel_to_commit().ok())
    };
    let parent_refs: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();
    let oid = git
        .commit(
            Some("refs/heads/main"),
            &sig,
            &sig,
            msg,
            &tree,
            &parent_refs,
        )
        .unwrap();
    oid.to_string()
}

/// One doctor invocation: fresh isolated HOME + LIEVO_DB in a temp dir,
/// cwd = `cwd`. Returns (stdout, stderr, exit_code).
fn run_doctor(cwd: &std::path::Path, extra_env: &[(&str, &str)]) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("lievo-doctor-{}-{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("db");
    let home = base.join("home");
    let _ = std::fs::create_dir_all(&home);

    let mut cmd = Command::new(lievo_bin());
    cmd.arg("doctor")
        .current_dir(cwd)
        .env("HOME", &home)
        .env_remove("LIEVO_PROJECT_DIR")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("LIEVO_NO_REFRESH")
        .env_remove("LIEVO_MCP_TOOLS")
        .env_remove("LIEVO_DB");
    for (k, v) in extra_env {
        if *k == "LIEVO_DB" && v.is_empty() {
            // Empty marker: leave LIEVO_DB unset (fresh-install tests).
            continue;
        }
        cmd.env(*k, v);
    }
    if !extra_env.iter().any(|(k, _)| *k == "LIEVO_DB") {
        cmd.env("LIEVO_DB", &db);
    }
    let out = cmd.output().expect("failed to run lievo doctor");
    let _ = std::fs::remove_dir_all(&base);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(999),
    )
}

/// Run doctor with HOME and LIEVO_DB both under the test's temp base dir,
/// so the child's resolved data dir stays inside the temp dir.
fn run_doctor_with_home(
    cwd: &std::path::Path,
    home: &std::path::Path,
    db: &std::path::Path,
) -> (String, String, i32) {
    let home_str = home.to_string_lossy().into_owned();
    let mut cmd = Command::new(lievo_bin());
    cmd.arg("doctor")
        .current_dir(cwd)
        .env("LIEVO_DB", db)
        .env("HOME", &home_str)
        .env_remove("LIEVO_PROJECT_DIR")
        .env_remove("CLAUDE_PROJECT_DIR");
    let out = cmd.output().expect("failed to run doctor with parent home");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(999),
    )
}

fn json_of(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).expect("doctor --format json must be valid JSON")
}

/// Shared helper: the JSON `ok` field must equal (exit code == 0) in every
/// doctor run that produces JSON (issue #875).
fn assert_ok_matches_exit_code(v: &serde_json::Value, code: i32) {
    let ok = v["ok"].as_bool().expect("ok field must be a bool");
    assert_eq!(
        ok,
        code == 0,
        "JSON ok != (exit code == 0): ok={ok}, exit={code}"
    );
}

/// Register a repo via the admin command and index it (core index; the
/// sample fixture is tiny so the full refresh completes in seconds).
fn register_and_index(db: &std::path::Path, root: &std::path::Path, _home: &str) {
    // HOME = the parent of the DB path (the test's temp base dir), so the
    // child's lievo_data_dir() is the test's data dir.
    let home = db.parent().unwrap().to_string_lossy().into_owned();
    let bin = lievo_bin();
    let out = Command::new(&bin)
        .args(["admin", "add-repo"])
        .arg(root)
        .arg("docproj")
        .env("LIEVO_DB", db)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "add-repo failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = Command::new(&bin)
        .args(["refresh", "docproj", "--no-summarize"])
        .env("LIEVO_DB", db)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "refresh failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// -- registered + indexed + current ------------------------------------

#[test]
fn registered_and_current_exits_zero() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-cur-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("cur.db");
    let _ = std::fs::remove_file(&db);
    let home = base.join("home");
    let _ = std::fs::create_dir_all(&home);
    let home_str = home.to_string_lossy().into_owned();
    register_and_index(&db, &root, &home_str);
    let (stdout, _stderr, code) = run_doctor_with_home(&root, &home, &db);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 0, "current repo must exit 0: {stdout}");
    assert!(stdout.contains("registered: true"), "{stdout}");
    assert!(stdout.contains("index: current"), "{stdout}");
    assert!(
        !stdout.contains("problems:"),
        "no problems expected: {stdout}"
    );
}

// -- git repo not registered -----------------------------------------

#[test]
fn unregistered_repo_exits_zero_with_fix() {
    let (_dir, root) = git_repo();
    let (stdout, _stderr, code) = run_doctor(&root, &[]);
    assert_eq!(code, 0, "unregistered repo must exit 0: {stdout}");
    assert!(stdout.contains("registered: false"), "{stdout}");
    assert!(stdout.contains("not registered"), "{stdout}");
    assert!(
        stdout.contains("`lievo mcp` will register and index it automatically on first use"),
        "{stdout}"
    );
}

// -- outside a git repo -----------------------------------------------

#[test]
fn outside_git_repo_exits_one_and_omits_repo_lines() {
    let plain = tempfile::TempDir::new().unwrap();
    let (stdout, _stderr, code) = run_doctor(plain.path(), &[]);
    assert_eq!(code, 1, "outside a git repo must exit 1: {stdout}");
    assert!(stdout.contains("not in a git repository"), "{stdout}");
    assert!(!stdout.contains("registered:"), "{stdout}");
    assert!(!stdout.contains("index: never_indexed"), "{stdout}");
    assert!(!stdout.contains("index: current"), "{stdout}");
    assert!(!stdout.contains("index: stale"), "{stdout}");
    assert!(
        stdout.contains("open your agent in a git repository, or set LIEVO_PROJECT_DIR to one"),
        "{stdout}"
    );
}

// -- LIEVO_PROJECT_DIR -----------------------------------------------

#[test]
fn lievo_project_dir_names_the_source() {
    let (_dir, root) = git_repo();
    let other = tempfile::TempDir::new().unwrap();
    let (stdout, _stderr, code) = run_doctor(
        other.path(),
        &[("LIEVO_PROJECT_DIR", root.to_str().unwrap())],
    );
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("(source: LIEVO_PROJECT_DIR)"), "{stdout}");
}

// -- stale index ---------------------------------------------------------

#[test]
fn stale_index_is_a_warning_not_a_failure() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-stale-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("stale.db");
    let _ = std::fs::remove_file(&db);
    let home = base.join("home");
    let _ = std::fs::create_dir_all(&home);
    let home_str = home.to_string_lossy().into_owned();
    register_and_index(&db, &root, &home_str);
    // Make the index stale: add a second commit after indexing.
    std::fs::write(root.join("b.rs"), "fn beta() {}\n").unwrap();
    let _second = git_commit(&root, "second");
    let (stdout, _stderr, code) = run_doctor_with_home(&root, &home, &db);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 0, "stale index must not fail: {stdout}");
    assert!(stdout.contains("index: stale"), "{stdout}");
    assert!(
        stdout.contains(
            "start your agent (or `lievo mcp`) here; indexing runs automatically in the background"
        ),
        "{stdout}"
    );
}

// -- LIEVO_NO_REFRESH fix text -------------------------------------------

#[test]
fn no_refresh_changes_fix_text() {
    let (_dir, root) = git_repo();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_NO_REFRESH", "1")]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains(
            "automatic indexing is disabled by LIEVO_NO_REFRESH; run `lievo refresh`, or unset it"
        ),
        "{stdout}"
    );
    assert!(stdout.contains("env LIEVO_NO_REFRESH=1"), "{stdout}");
}

// -- indexing in progress -----------------------------------------------

#[test]
fn indexing_in_progress_reported() {
    use lievo::refresh::lock;
    let (_dir, root) = git_repo();
    // The child resolves its lievo data dir from its own HOME (`<HOME>/.lievo`
    // per `lievo_data_dir`). Acquire the per-repo index lock in THAT temporary
    // data dir (never the real ~/.lievo) and give the child a HOME whose
    // `.lievo` is the same dir, so parent and child see the same lock/status
    // files. The base dir doubles as the child's HOME, so its data dir is
    // `base/.lievo`.
    let base = std::env::temp_dir().join(format!("lievo-doctor-lock-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let data_dir = base.join(".lievo");
    let holder = lock::acquire(&root, lock::IndexTrigger::FirstIndex, &data_dir)
        .expect("must acquire the index lock");
    let db = base.join("lock.db");
    let base_str = base.to_string_lossy().into_owned();
    let mut cmd = Command::new(lievo_bin());
    cmd.arg("doctor")
        .current_dir(&root)
        .env("LIEVO_DB", &db)
        .env("HOME", &base_str)
        .env_remove("LIEVO_PROJECT_DIR")
        .env_remove("CLAUDE_PROJECT_DIR");
    let out = cmd.output().expect("failed to run doctor");
    drop(holder);
    let _ = std::fs::remove_dir_all(&base);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(999);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("indexing in progress: true"),
        "must report a live index holder: {stdout}"
    );
}

// -- data dir not writable ----------------------------------------------

/// The data directory is "not writable" when the probe cannot create a file
/// inside it. The test uses a nonexistent parent directory: the missing
/// directory is reported as a "database cannot be opened" failure (issue
/// #875 D1) — never a double "not writable" report.
#[test]
fn non_writable_data_dir_exits_one() {
    let (_dir, root) = git_repo();
    // Point LIEVO_DB at a file inside a directory that does not exist.
    let db = std::env::temp_dir()
        .join(format!("lievo-doctor-nw-missing-{}", std::process::id()))
        .join("db");
    let db_str = db.to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_DB", &db_str)]);
    assert_eq!(code, 1, "missing LIEVO_DB parent must exit 1: {stdout}");
    // The doctor report must be printed (it is NOT empty anymore) and the
    // problem line is the db-open failure with a cause-specific fix.
    assert!(
        !stdout.is_empty(),
        "doctor report must still be printed when the DB cannot be opened"
    );
    assert!(
        stdout.contains("database cannot be opened"),
        "must name the db-open failure: {stdout}"
    );
    assert!(
        stdout.contains("create the directory or correct LIEVO_DB"),
        "missing parent dir needs its specific fix: {stdout}"
    );
    assert!(
        !stdout.contains("data directory not writable"),
        "must not double-report a missing dir as not writable: {stdout}"
    );
}

/// A LIEVO_DB pointing at a path whose parent exists but is a regular file:
/// doctor reports the db-open failure with the not-a-directory fix, exit 1.
#[test]
fn lievo_db_parent_is_a_file_reports_not_a_directory() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-fileparent-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let blocker = base.join("blocker");
    std::fs::write(&blocker, b"i am a file").unwrap();
    let db = blocker.join("db");
    let db_str = db.to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_DB", &db_str)]);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 1, "LIEVO_DB with a file parent must exit 1: {stdout}");
    assert!(
        stdout.contains("database cannot be opened"),
        "must name the db-open failure: {stdout}"
    );
    assert!(
        stdout
            .contains("is not a directory; point LIEVO_DB at a file inside an existing directory"),
        "a file parent needs its specific fix: {stdout}"
    );
    assert!(
        !stdout.contains("parent directory is missing"),
        "a file parent must not be reported as missing: {stdout}"
    );
}

// -- JSON shape + nulls -----------------------------------------------

#[test]
fn json_shape_preserves_nulls_and_fields() {
    let (_dir, root) = git_repo();
    let (stdout, _stderr, code) = run_doctor_json(&root, &[]);
    assert_eq!(code, 0, "{stdout}");
    let v = json_of(&stdout);
    assert_ok_matches_exit_code(&v, code);
    // Exact top-level field set (issue decision 1).
    let expected = [
        "version",
        "data_dir",
        "db_path",
        "db_writable",
        "repo",
        "repo_source",
        "registered",
        "project",
        "index_state",
        "entity_count",
        "last_indexed_commit",
        "head_commit",
        "indexing_in_progress",
        "indexing_elapsed_secs",
        "env",
        "vector_index_present",
        "embedding_model_present",
        "problems",
        "ok",
    ];
    for key in expected {
        assert!(v.get(key).is_some(), "missing field {key}: {stdout}");
    }
    assert_eq!(v["version"].as_str().unwrap(), env!("CARGO_PKG_VERSION"));
    assert_eq!(v["repo_source"].as_str().unwrap(), "cwd");
    assert!(!v["registered"].as_bool().unwrap());
    assert_eq!(
        v["project"].as_null(),
        Some(()),
        "project null when unregistered"
    );
    assert_eq!(v["index_state"].as_str().unwrap(), "never_indexed");
    assert_eq!(v["entity_count"], serde_json::Value::Null);
    assert_eq!(v["last_indexed_commit"], serde_json::Value::Null);
    assert!(v["head_commit"].as_str().is_some());
    assert!(!v["indexing_in_progress"].as_bool().unwrap());
    assert_eq!(v["indexing_elapsed_secs"], serde_json::Value::Null);
    let env_obj = v["env"].as_object().unwrap();
    assert_eq!(
        env_obj.len(),
        1,
        "only LIEVO_DB is set in the child: {stdout}"
    );
    assert!(env_obj.contains_key("LIEVO_DB"));
    assert!(!v["vector_index_present"].as_bool().unwrap());
    assert!(v["ok"].as_bool().unwrap(), "warnings do not fail");
    // Three warning problems: the fresh-install "database does not exist
    // yet" line (the helper's LIEVO_DB file is absent) plus unregistered and
    // never-indexed.
    let problems = v["problems"].as_array().unwrap();
    assert_eq!(
        problems.len(),
        3,
        "db-absent + unregistered + never-indexed: {stdout}"
    );
    for p in problems {
        assert!(p.get("what").is_some() && p.get("fix").is_some());
    }
}

#[test]
fn json_outside_git_repo_has_nulls() {
    let plain = tempfile::TempDir::new().unwrap();
    let plain_str = plain.path().to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor_json(plain.path(), &[]);
    assert_eq!(code, 1, "{stdout}");
    let v = json_of(&stdout);
    assert_ok_matches_exit_code(&v, code);
    assert_eq!(v["repo"], serde_json::Value::Null);
    assert_eq!(v["repo_source"], serde_json::Value::Null);
    assert!(!v["registered"].as_bool().unwrap());
    assert_eq!(v["project"], serde_json::Value::Null);
    assert_eq!(v["index_state"], serde_json::Value::Null);
    assert_eq!(v["entity_count"], serde_json::Value::Null);
    assert_eq!(v["last_indexed_commit"], serde_json::Value::Null);
    assert_eq!(v["head_commit"], serde_json::Value::Null);
    assert!(!v["ok"].as_bool().unwrap());
    // Two problems: the fresh-install "database does not exist yet" line
    // (the helper's LIEVO_DB file is absent) plus the not-in-git-repo line.
    let problems = v["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 2, "db-absent + not-in-git: {stdout}");
    let what = problems[1]["what"].as_str().unwrap();
    assert!(
        what.contains("is not inside a git repository"),
        "the checked directory must be named in the problem line: {stdout}"
    );
    // The checked directory may be canonicalised (e.g. /var → /private/var
    // on macOS), so assert on the last path component rather than the full
    // string.
    let checked_last = std::path::Path::new(&plain_str)
        .components()
        .next_back()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");
    assert!(
        what.starts_with(&plain_str) || what.contains(checked_last),
        "the problem line must name the checked directory: {what} (expected prefix {plain_str})"
    );
}

fn run_doctor_json(cwd: &std::path::Path, extra_env: &[(&str, &str)]) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("lievo-doctor-json-{}-{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("db");
    let home = base.join("home");
    let _ = std::fs::create_dir_all(&home);
    let mut cmd = Command::new(lievo_bin());
    cmd.arg("doctor")
        .arg("--format")
        .arg("json")
        .current_dir(cwd)
        .env("LIEVO_DB", &db)
        .env("HOME", &home)
        .env_remove("LIEVO_PROJECT_DIR")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("LIEVO_NO_REFRESH")
        .env_remove("LIEVO_MCP_TOOLS");
    // An empty value means "leave this variable unset", as in `run_doctor`.
    for (k, v) in extra_env {
        if v.is_empty() {
            cmd.env_remove(k);
        } else {
            cmd.env(k, v);
        }
    }
    let out = cmd.output().expect("failed to run doctor --format json");
    let _ = std::fs::remove_dir_all(&base);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(999),
    )
}

// -- doctor must not mutate state ----------------------------------------

#[test]
fn doctor_does_not_mutate_the_database() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-mut-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("mut.db");
    let _ = std::fs::remove_file(&db);
    let home = base.join("home");
    let _ = std::fs::create_dir_all(&home);
    let home_str = home.to_string_lossy().into_owned();
    register_and_index(&db, &root, &home_str);
    let before_size = std::fs::metadata(&db).unwrap().len();
    let before_mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    let _ = run_doctor(&root, &[]);
    let after_size = std::fs::metadata(&db).unwrap().len();
    let after_mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(before_size, after_size, "doctor must not grow the database");
    assert_eq!(
        before_mtime, after_mtime,
        "doctor must not touch the database (mtime unchanged)"
    );
}

// -- database cannot be opened (issue #875 D1) -------------------------

/// A corrupt-but-present DB file: doctor reports "database cannot be
/// opened" with a cause-specific fix, exit 1.
#[test]
fn corrupt_db_file_reports_cannot_open() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-corrupt-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("corrupt.db");
    std::fs::write(&db, b"this is not a sqlite database").unwrap();
    let db_str = db.to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_DB", &db_str)]);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 1, "corrupt DB must exit 1: {stdout}");
    assert!(
        !stdout.is_empty(),
        "doctor report must still be printed for a corrupt DB"
    );
    assert!(
        stdout.contains("database cannot be opened"),
        "must name the db-open failure: {stdout}"
    );
    assert!(
        stdout.contains("move it aside"),
        "corrupt file fix must say to move it aside: {stdout}"
    );
    // A corrupt file is not a "not writable" report.
    assert!(
        !stdout.contains("data directory not writable"),
        "corrupt DB is a db-open failure, not a writability failure: {stdout}"
    );
}

/// JSON variant: db-open failure produces ok=false matching exit 1, and
/// registration/index fields are null.
#[test]
fn corrupt_db_json_ok_matches_exit_code() {
    let (_dir, root) = git_repo();
    let base =
        std::env::temp_dir().join(format!("lievo-doctor-corrupt-json-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("corrupt.db");
    std::fs::write(&db, b"not a sqlite database at all").unwrap();
    let db_str = db.to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor_json(&root, &[("LIEVO_DB", &db_str)]);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 1, "corrupt DB must exit 1: {stdout}");
    let v = json_of(&stdout);
    assert_ok_matches_exit_code(&v, code);
    assert!(
        !v["ok"].as_bool().unwrap(),
        "db-open failure must set ok=false"
    );
    assert!(!v["registered"].as_bool().unwrap());
    assert_eq!(v["entity_count"], serde_json::Value::Null);
    let problems = v["problems"].as_array().unwrap();
    assert!(
        problems.iter().any(|p| p["what"]
            .as_str()
            .unwrap()
            .starts_with("database cannot be opened")),
        "db-open problem must be present: {stdout}"
    );
}

// -- fresh install: no DB yet, LIEVO_DB unset ---------------------------

/// LIEVO_DB unset and the default DB absent (fresh install): NOT a failure
/// — doctor exits 0, reports registered false, and creates nothing.
#[test]
fn fresh_install_exits_zero_and_creates_nothing() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-fresh-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_DB", "")]);
    let _ = std::fs::remove_dir_all(&base);
    assert_eq!(code, 0, "fresh install (no DB yet) must exit 0: {stdout}");
    assert!(
        !stdout.contains("database cannot be opened"),
        "no db-open failure expected: {stdout}"
    );
    assert!(stdout.contains("registered: false"), "{stdout}");
    assert!(
        !base.join("home/.lievo").exists(),
        "doctor must not create the data dir or DB on a fresh install"
    );
}

/// LIEVO_DB set to a non-existent file inside an existing directory:
/// doctor exits 0, ok is true, the report names the path, nothing created.
#[test]
fn lievo_db_file_absent_in_existing_dir_is_not_a_failure() {
    let (_dir, root) = git_repo();
    let base = std::env::temp_dir().join(format!("lievo-doctor-absent-env-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&base);
    let db = base.join("absent.db");
    let db_str = db.to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_DB", &db_str)]);
    assert_eq!(
        code, 0,
        "absent LIEVO_DB in an existing dir must exit 0: {stdout}"
    );
    assert!(stdout.contains("registered: false"), "{stdout}");
    assert!(
        stdout.contains(&format!("{db_str} does not exist yet")),
        "must name the path and say it does not exist yet: {stdout}"
    );
    assert!(
        stdout.contains("will be created on first use"),
        "must say the database will be created on first use: {stdout}"
    );
    assert!(
        !db.exists(),
        "doctor must not create the LIEVO_DB database file"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// -- LIEVO_NO_REFRESH gate (issue #875) ----------------------------------

/// LIEVO_NO_REFRESH=0 does not disable automatic indexing (the gate
/// matches only the exact value "1").
#[test]
fn no_refresh_zero_does_not_disable() {
    let (_dir, root) = git_repo();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_NO_REFRESH", "0")]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        !stdout.contains("automatic indexing is disabled by LIEVO_NO_REFRESH"),
        "LIEVO_NO_REFRESH=0 must not claim automatic indexing is disabled: {stdout}"
    );
    assert!(
        stdout.contains("indexing runs automatically in the background"),
        "the default automatic-indexing fix must still be offered: {stdout}"
    );
}

/// LIEVO_NO_REFRESH=1 (the exact gate value) does disable it.
#[test]
fn no_refresh_one_disables() {
    let (_dir, root) = git_repo();
    let (stdout, _stderr, code) = run_doctor(&root, &[("LIEVO_NO_REFRESH", "1")]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains(
            "automatic indexing is disabled by LIEVO_NO_REFRESH; run `lievo refresh`, or unset it"
        ),
        "{stdout}"
    );
}

// -- outside a git repo: the checked directory is named ------------------

/// A directory that exists but is not inside a git repository: the problem
/// line names the checked directory.
#[test]
fn outside_git_repo_names_the_checked_directory() {
    let plain = tempfile::TempDir::new().unwrap();
    let plain_str = plain.path().to_string_lossy().into_owned();
    let (stdout, _stderr, code) = run_doctor(plain.path(), &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains(&format!("{plain_str} is not inside a git repository")),
        "the checked directory must be named: {stdout}"
    );
}

/// A LIEVO_PROJECT_DIR that does not exist: the problem line names the
/// checked directory and says it does not exist.
#[test]
fn outside_git_repo_missing_dir_says_missing() {
    let missing = std::env::temp_dir()
        .join(format!("lievo-doctor-missing-dir-{}", std::process::id()))
        .join("does-not-exist");
    let missing_str = missing.to_string_lossy().into_owned();
    let outside = tempfile::TempDir::new().unwrap();
    let (stdout, _stderr, code) =
        run_doctor(outside.path(), &[("LIEVO_PROJECT_DIR", &missing_str)]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stdout.contains(&format!("{missing_str} does not exist")),
        "the missing checked directory must be named: {stdout}"
    );
    assert!(
        !stdout.contains(&format!("{missing_str} is not inside a git repository")),
        "a missing dir must not be reported as merely not-in-git: {stdout}"
    );
}

// -- unborn HEAD -----------------------------------------------------------

/// A git repository with no commits yet (unborn HEAD) gets the documented
/// "unborn" index state.
#[test]
fn unborn_head_reports_unborn_state() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let g = git2::Repository::init(&root).unwrap();
    g.set_head("refs/heads/main").unwrap();
    let (stdout, _stderr, code) = run_doctor(&root, &[]);
    assert_eq!(code, 0, "unborn HEAD must not fail: {stdout}");
    assert!(
        stdout.contains("index: unborn"),
        "index_state must be unborn: {stdout}"
    );
    assert!(
        stdout.contains("repo has no commits yet"),
        "the human report must say the repo has no commits yet: {stdout}"
    );
    assert!(
        !stdout.contains("HEAD:"),
        "no HEAD line for an unborn branch: {stdout}"
    );
}

/// JSON variant: index_state == "unborn", head_commit null, ok == exit 0.
#[test]
fn unborn_head_json_index_state_is_unborn() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let g = git2::Repository::init(&root).unwrap();
    g.set_head("refs/heads/main").unwrap();
    let (stdout, _stderr, code) = run_doctor_json(&root, &[]);
    assert_eq!(code, 0, "unborn HEAD must not fail: {stdout}");
    let v = json_of(&stdout);
    assert_ok_matches_exit_code(&v, code);
    assert_eq!(v["index_state"].as_str().unwrap(), "unborn");
    assert_eq!(v["head_commit"], serde_json::Value::Null);
    assert!(v["ok"].as_bool().unwrap());
}
