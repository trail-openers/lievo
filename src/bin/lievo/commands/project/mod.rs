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

/// Escape backslashes, double-quotes and control characters so string values
/// are safe in hand-rolled JSON and do not write raw ESC/LF bytes to stdout.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod project_tests;
