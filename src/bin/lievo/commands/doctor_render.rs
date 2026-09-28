// Rendering for the `lievo doctor` report (issue #875): the `DoctorReport`
// data block, the human layout, and the JSON document.
//
// This file is a `#[path]` sibling of `doctor.rs`: the module lives at the
// same crate path as `doctor.rs`'s own items, so `use super::*` resolves to
// the doctor command's scope. The diagnosis itself stays in `doctor.rs`;
// nothing in here inspects state, it only prints.

use std::io::Write;
use std::path::PathBuf;

use super::*;
use lievo::mcp::repo_resolution::RepoRootSource;
use serde_json::json;

/// All data needed to render the doctor report (human or JSON).
pub struct DoctorReport {
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
    pub db_writable: bool,
    pub repo_root: Option<PathBuf>,
    pub source_label: Option<RepoRootSource>,
    pub registered: bool,
    pub project_name: Option<String>,
    pub index_state: Option<String>,
    pub entity_count: Option<u64>,
    pub last_analyzed_commit: Option<String>,
    pub head_commit: Option<String>,
    pub indexing_in_progress: bool,
    pub indexing_elapsed: Option<u64>,
    pub env: Vec<(&'static str, String)>,
    pub vector_index_present: bool,
    pub embedding_model_present: bool,
    pub problems: Vec<Problem>,
}

/// Print the human-readable report (stdout, one line per check).
pub fn print_human(report: &DoctorReport) {
    let mut out: Vec<String> = Vec::new();
    out.push(format!("lievo {}", env!("CARGO_PKG_VERSION")));
    out.push(format!(
        "data dir: {} (writable: {})",
        report.data_dir.to_string_lossy(),
        report.db_writable
    ));
    out.push(format!("db: {}", report.db_path.to_string_lossy()));

    match &report.repo_root {
        Some(root) => out.push(format!(
            "repo: {} (source: {})",
            root.to_string_lossy(),
            report.source_label.map(|s| s.as_label()).unwrap_or("?")
        )),
        None => out.push("resolution: not in a git repository".to_string()),
    }

    // Outside a git repo: registration and index lines are omitted
    // (issue decision 4).
    if report.repo_root.is_some() {
        out.push(format!(
            "registered: {}{}",
            report.registered,
            report
                .project_name
                .as_ref()
                .map(|p| format!(" (project: {p}"))
                .unwrap_or_default()
        ));
        out.push(format!(
            "index: {}{}",
            report.index_state.as_deref().unwrap_or("unknown"),
            report
                .entity_count
                .map(|c| format!(" (entities: {c}"))
                .unwrap_or_default()
        ));
        if let Some(stored) = &report.last_analyzed_commit {
            out.push(format!("last indexed commit: {stored}"));
        }
        if let Some(head) = &report.head_commit {
            out.push(format!("HEAD: {head}"));
        }
        out.push(format!(
            "indexing in progress: {}{}",
            report.indexing_in_progress,
            report
                .indexing_elapsed
                .map(|s| format!(" ({s}s)"))
                .unwrap_or_default()
        ));
    }

    if report.env.is_empty() {
        out.push("env: (none set)".to_string());
    } else {
        for (name, value) in &report.env {
            out.push(format!("env {name}={value}"));
        }
    }

    out.push(format!(
        "vector index: {} (not required for lievo_explore)",
        report.vector_index_present
    ));
    out.push(format!(
        "embedding model: {} (not required for lievo_explore)",
        report.embedding_model_present
    ));

    if !report.problems.is_empty() {
        out.push("problems:".to_string());
        for p in &report.problems {
            out.push(format!("  - {} — fix: {}", p.what, p.fix));
        }
    }

    let mut w = std::io::stdout().lock();
    for line in &out {
        let _ = writeln!(w, "{line}");
    }
}

/// Build the pretty-printed JSON document for the report.
pub fn build_json(report: &DoctorReport, ok: bool) -> String {
    let env_json: serde_json::Map<String, serde_json::Value> = report
        .env
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    let out = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "data_dir": report.data_dir.to_string_lossy(),
        "db_path": report.db_path.to_string_lossy(),
        "db_writable": report.db_writable,
        "repo": report.repo_root.as_ref().map(|p| p.to_string_lossy().to_string()),
        "repo_source": report.source_label.map(|s| s.as_label().to_string()),
        "registered": report.registered,
        "project": report.project_name,
        "index_state": report.index_state,
        "entity_count": report.entity_count,
        "last_indexed_commit": report.last_analyzed_commit,
        "head_commit": report.head_commit,
        "indexing_in_progress": report.indexing_in_progress,
        "indexing_elapsed_secs": report.indexing_elapsed,
        "env": env_json,
        "vector_index_present": report.vector_index_present,
        "embedding_model_present": report.embedding_model_present,
        "problems": report
            .problems
            .iter()
            .map(|p| json!({ "what": p.what, "fix": p.fix }))
            .collect::<Vec<_>>(),
        "ok": ok,
    });
    serde_json::to_string_pretty(&out).unwrap_or_default()
}
