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

/// Recursively copy `src` into `dst`, propagating all I/O errors.
fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copy tests/fixtures/sample_repo into a fresh temp dir, git-init it with a
/// single commit on `refs/heads/main`, and return the `TempDir`. The caller
/// owns the `TempDir` and must keep it alive for the test's duration (use
/// `.path()` to get the fixture path); the fixture is deleted when the
/// `TempDir` is dropped. The in-tree copy has no .git; analysis and
/// `add-repo` need a real committed working tree.
pub fn prepare_fixture_repo() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let src = PathBuf::from("tests/fixtures/sample_repo");
    let tmp = tempfile::TempDir::new().map_err(|e| format!("create temp dir for fixture: {e}"))?;
    let dst = tmp.path().to_path_buf();
    copy_recursive(&src, &dst).map_err(|e| format!("copy fixture {src:?} -> {dst:?}: {e}"))?;
    let git =
        git2::Repository::init(&dst).map_err(|e| format!("git init on fixture {dst:?}: {e}"))?;
    // `git2::Repository::init` may default HEAD to `refs/heads/master`
    // regardless of init.defaultBranch; pin it to main before the commit
    // (issue #715).
    git.set_head("refs/heads/main")
        .map_err(|e| format!("point HEAD at refs/heads/main on {dst:?}: {e}"))?;
    let mut index = git
        .index()
        .map_err(|e| format!("open git index on {dst:?}: {e}"))?;
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("stage fixture files in {dst:?}: {e}"))?;
    index
        .write()
        .map_err(|e| format!("write git index in {dst:?}: {e}"))?;
    let tree_oid = index
        .write_tree()
        .map_err(|e| format!("write tree in {dst:?}: {e}"))?;
    let tree = git
        .find_tree(tree_oid)
        .map_err(|e| format!("find tree in {dst:?}: {e}"))?;
    let sig =
        git2::Signature::now("lievo-test", "lievo@test").map_err(|e| format!("signature: {e}"))?;
    git.commit(Some("refs/heads/main"), &sig, &sig, "fixture", &tree, &[])
        .map_err(|e| format!("commit fixture in {dst:?}: {e}"))?;
    Ok(tmp)
}
