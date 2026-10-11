//! The filesystem primitives of a relocation: renames that are recorded so they
//! can be undone, and the undo itself. Nothing here knows about rows or locks.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A rename, kept so it can be undone if a later step fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// A rename that a rollback could not undo, with the error that stopped it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedUndo {
    pub rename: Rename,
    pub error: String,
}

/// Whether `path` exists (without following a symlink).
pub(super) fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Rename `from` to `to` when `from` exists. A missing source is not an error:
/// a repo that was never indexed has no index directory or status file. An
/// existing destination is an error, never overwritten: it may be stale state
/// that must not be destroyed silently.
pub(super) fn move_if_present(
    done: &mut Vec<Rename>,
    from: PathBuf,
    to: PathBuf,
) -> io::Result<()> {
    if !exists(&from)? {
        return Ok(());
    }
    if exists(&to)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination '{}' already exists", to.display()),
        ));
    }
    fs::rename(&from, &to)?;
    done.push(Rename { from, to });
    Ok(())
}

/// Undo completed renames, newest first. Returns the renames that could not be
/// undone, each with the error from its undo; each stays at `to`.
pub(super) fn roll_back(done: &[Rename]) -> Vec<FailedUndo> {
    done.iter()
        .rev()
        .filter_map(|r| {
            fs::rename(&r.to, &r.from).err().map(|e| FailedUndo {
                rename: r.clone(),
                error: e.to_string(),
            })
        })
        .collect()
}

// Test seam: runs once between the index-directory rename and the status
// rename, so a test can force a failed rollback. Never set in production.
#[cfg(test)]
thread_local! {
    pub(crate) static BETWEEN_RENAMES: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(super) fn between_renames() {
    if let Some(hook) = BETWEEN_RENAMES.with(|h| h.borrow_mut().take()) {
        hook();
    }
}

#[cfg(not(test))]
pub(super) fn between_renames() {}
