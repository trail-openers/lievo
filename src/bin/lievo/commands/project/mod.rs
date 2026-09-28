// Project and repository management command handlers.

mod info_ops;
mod project_ops;
mod query_ops;
mod repo_ops;
mod selfcheck_edge_split;
mod selfcheck_false_zero;
mod selfcheck_include_d;
mod selfcheck_metrics;
mod selfcheck_ops;
mod selfcheck_super_probe;
mod selfcheck_super_probe_sites;

pub use info_ops::info;
pub use project_ops::{coverage, create_project, delete_project, list_projects};
pub use query_ops::status;
pub use repo_ops::{add_repo, delete_repo, link_repo, list_repos};
pub use selfcheck_ops::{SelfcheckArgs, selfcheck};

/// Escape backslashes and double-quotes so string values are safe in hand-rolled JSON.
pub fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod project_tests;
