use std::path::PathBuf;
use std::process::Command;

pub fn run_lievo(args: &[&str]) -> (String, String, i32) {
    let bin_path = if PathBuf::from("./target/debug/lievo").exists() {
        "./target/debug/lievo".to_string()
    } else {
        "cargo".to_string()
    };

    let result = if bin_path == "cargo" {
        Command::new("cargo")
            .args(["run", "--bin", "lievo", "--"])
            .args(args)
            .output()
            .expect("failed to run lievo")
    } else {
        Command::new(&bin_path)
            .args(args)
            .output()
            .expect("failed to run lievo")
    };

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
    let status = result.status.code().unwrap_or(1);

    (stdout, stderr, status)
}

/// Locate the pre-built lievo binary (built by `cargo test` before the
/// integration tests execute).
pub fn lievo_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    // <workspace>/target/debug/deps/<test>-<hash>
    //   -> parent: deps, parent: debug
    let debug = exe.parent().and_then(|p| p.parent()).expect("debug dir");
    debug.join("lievo")
}

/// Recursively copy `src` into `dst`.
fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) {
    for entry in std::fs::read_dir(src).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("fixture entry type").is_dir() {
            let _ = std::fs::create_dir_all(&target);
            copy_recursive(&entry.path(), &target);
        } else {
            let _ = std::fs::copy(entry.path(), &target);
        }
    }
}

/// Copy tests/fixtures/sample_repo into a fresh temp dir, git-init it with a
/// single commit on `refs/heads/main`, and return the PathBuf (the parent
/// TempDir is intentionally leaked so the repo outlives the call — the
/// caller owns cleanup). The in-tree copy has no .git; analysis and
/// `add-repo` need a real committed working tree.
///
/// Uses a unique path per invocation (pid + atomic counter) so parallel
/// tests in the same binary never collide.
pub fn prepare_fixture_repo() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let src = PathBuf::from("tests/fixtures/sample_repo");
    let dst = std::env::temp_dir().join(format!("lievo-fixture-{pid}-{n}"));
    let _ = std::fs::remove_dir_all(&dst);
    copy_recursive(&src, &dst);
    let git = git2::Repository::init(&dst).expect("git init on fixture copy");
    // `git2::Repository::init` may default HEAD to `refs/heads/master`
    // regardless of init.defaultBranch; pin it to main before the commit
    // (issue #715).
    git.set_head("refs/heads/main")
        .expect("point HEAD at refs/heads/main before the fixture commit");
    let mut index = git.index().expect("open git index");
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .expect("stage fixture files");
    index.write().expect("write git index");
    let tree_oid = index.write_tree().expect("write tree");
    let tree = git.find_tree(tree_oid).expect("find tree");
    let sig = git2::Signature::now("lievo-test", "lievo@test").expect("signature");
    git.commit(Some("refs/heads/main"), &sig, &sig, "fixture", &tree, &[])
        .expect("commit fixture");
    dst
}
