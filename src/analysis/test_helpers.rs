#[cfg(test)]
pub(crate) use helpers::*;

#[cfg(test)]
mod helpers {
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Create a git repo with an initial commit containing `files`.
    pub(crate) fn make_repo_with_commit(files: &[(&str, &str)]) -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@t.com").unwrap();

        for (name, content) in files {
            let full_path = dir.path().join(name);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full_path, content).unwrap();
        }

        let mut index = repo.index().unwrap();
        for (name, _) in files {
            index.add_path(Path::new(name)).unwrap();
        }
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let commit_id = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        (dir, commit_id.to_string())
    }

    /// Add a second commit to an existing repo.
    pub(crate) fn add_commit(repo_path: &Path, files: &[(&str, &str)]) -> String {
        let repo = git2::Repository::open(repo_path).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@t.com").unwrap();

        for (name, content) in files {
            let full_path = repo_path.join(name);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full_path, content).unwrap();
        }

        let mut index = repo.index().unwrap();
        for (name, _) in files {
            index.add_path(Path::new(name)).unwrap();
        }
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let commit_id = repo
            .commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
            .unwrap();

        commit_id.to_string()
    }
}
