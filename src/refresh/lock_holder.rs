// Lock-holder: acquires the per-repo index lock via the real `acquire()`
// path and sleeps, so the parent can SIGKILL it and verify the OS releases
// the lock (issue #864, replacing the python3 flock holder).

/// Lock holder: run with `LIEVO_LOCK_HOLDER_REPO` (a repo path) set.
/// `LIEVO_LOCK_HOLDER_DATA_DIR` (optional, issue #864 follow-up) redirects
/// the lock/status files to a unique temp dir so the child never touches the
/// real lievo data dir. Acquires the real index lock, prints "LOCKED" once
/// it holds it, then sleeps until killed.
#[test]
fn lock_holder() {
    use crate::refresh::IndexTrigger;
    use crate::refresh::lock::acquire;
    use std::io::Write;
    use std::time::Duration;

    let repo = match std::env::var("LIEVO_LOCK_HOLDER_REPO") {
        Ok(r) => r,
        Err(_) => {
            eprintln!(
                "lock_holder: LIEVO_LOCK_HOLDER_REPO not set — skipping (test holder, \
                 invoked as a child by the SIGKILL recovery test)"
            );
            return;
        }
    };
    let repo_path = std::path::Path::new(&repo).to_path_buf();
    let data_dir = match std::env::var("LIEVO_LOCK_HOLDER_DATA_DIR") {
        Ok(d) => Some(std::path::PathBuf::from(d)),
        Err(_) => crate::extraction::lievo_data_dir().ok(),
    };
    let holder = match data_dir.as_deref() {
        Some(dir) => match acquire(&repo_path, IndexTrigger::FirstIndex, dir) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("lock_holder: failed to acquire lock: {e}");
                return;
            }
        },
        None => {
            eprintln!("lock_holder: no data dir available");
            return;
        }
    };
    let _ = writeln!(std::io::stdout(), "LOCKED");
    let _ = std::io::stdout().flush();
    drop(holder);
    // Sleep long enough that the parent definitely SIGKILLs us first.
    std::thread::sleep(Duration::from_secs(60));
}
