use lievo::Result;

pub fn serve(project_name: Option<&str>) -> Result<()> {
    log_resolution();
    lievo::mcp::server::serve(project_name)
}

/// Mirror `resolve_project_root`'s stderr diagnostics (issue #863): which
/// source the repo resolved from, or that the directory is not inside a git
/// repository. Kept here (binary-local) so `lievo doctor`'s stdout stays
/// clean and the lib function stays silent.
fn log_resolution() {
    use lievo::mcp::repo_resolution::resolve_project_root;
    match resolve_project_root() {
        Ok(resolved) => eprintln!(
            "lievo mcp: resolved repository root {} from {}",
            resolved.root.display(),
            resolved.source.as_label()
        ),
        Err(e) => eprintln!(
            "lievo mcp: {} path {} is not inside a git repository",
            e.source.as_label(),
            e.checked.display()
        ),
    }
}
