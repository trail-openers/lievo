use super::{clear_file_summaries, summarize};
use lievo::config::RepoConfig;
use lievo::summarization::pipeline::summarize_decision;

/// User-facing reason from the decision the gate itself produces (issue #786).
fn disabled_reason(cfg: &RepoConfig, apfel_available: bool) -> String {
    summarize_decision(false, cfg, apfel_available)
        .disabled_reason(cfg)
        .expect("test setup: decision must be disabled")
}
use lievo::model::{Entity, EntityTier};
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

fn storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().unwrap()
}

fn create_test_entity(
    id: &str,
    name: &str,
    tier: EntityTier,
    parent_id: Option<String>,
    path: Option<String>,
    project_id: &str,
    repo_id: &str,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier,
        parent_id,
        name: name.to_string(),
        path,
        language: Some("rust".to_string()),
        summary: Some("old summary".to_string()),
        summary_commit: Some("old-commit".to_string()),
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_summarize_unknown_project_returns_error() {
    let s = storage();
    let result = summarize(&s, Some("no-such-project"), None, OutputFormat::Human);
    assert!(result.is_err());
}

#[test]
fn test_disabled_reason_names_configured_backend_not_apfel() {
    use lievo::config::SummarizerBackend;

    let cfg = RepoConfig {
        summarizer_backend: Some(SummarizerBackend::LlamaServer.as_str().to_string()),
        ..Default::default()
    };
    // No endpoint configured and summarize unset -> endpoint is the reason.
    let reason = disabled_reason(&cfg, false);
    assert!(
        reason.contains("llama-server"),
        "reason should name llama-server: {reason}!"
    );
    assert!(
        !reason.contains("apfel"),
        "reason must not name apfel: {reason}!"
    );

    // Apfel unavailable case names apfel only for the apfel backend.
    let default_cfg = RepoConfig::default();
    let apfel_reason = disabled_reason(&default_cfg, false);
    assert!(
        apfel_reason.contains("apfel"),
        "apfel default should name apfel: {apfel_reason}!"
    );

    // Previously-wrong case: apfel backend + endpoint configured + apfel
    // binary absent. On that path the rule says the apfel binary (the
    // transport for apfel) is the missing piece — not "no endpoint",
    // which would be the non-apfel variant. The reason must name apfel.
    let apfel_endpoint_cfg = RepoConfig {
        summarizer_backend: Some(SummarizerBackend::Apfel.as_str().to_string()),
        apfel_endpoint: Some("http://127.0.0.1:9005".to_string()),
        ..Default::default()
    };
    let apfel_ep_reason = disabled_reason(&apfel_endpoint_cfg, false);
    assert!(
        apfel_ep_reason.contains("apfel"),
        "reason should name apfel: {apfel_ep_reason}!"
    );
    assert!(
        !apfel_ep_reason.contains("no endpoint configured"),
        "apfel + endpoint set must not say 'no endpoint': {apfel_ep_reason}!"
    );

    // Explicit disable names the configured backend too.
    let mut off_cfg = cfg.clone();
    off_cfg.summarize = Some(false);
    let off_reason = disabled_reason(&off_cfg, false);
    assert!(
        off_reason.contains("llama-server"),
        "off reason should name llama-server: {off_reason}!"
    );
    assert!(
        off_reason.contains("disabled in the repository config"),
        "off reason: {off_reason}!"
    );
}

#[test]
fn test_clear_file_summaries_without_entity_returns_error() {
    let s = storage();
    let proj = s.create_project("testproj", None).unwrap();
    s.add_repo(&proj.id, "testrepo", "/tmp/test").unwrap();

    let result = clear_file_summaries(&s, &proj.id, "nonexistent/file.rs");
    assert!(result.is_err());
}

#[test]
fn test_clear_file_summaries_clears_file_and_ancestors() {
    let s = storage();
    let proj = s.create_project("testproj", None).unwrap();
    let repo = s.add_repo(&proj.id, "testrepo", "/tmp/test").unwrap();

    // Create hierarchy: subsystem -> module -> file
    let subsystem = create_test_entity(
        "subsystem1",
        "subsystem",
        EntityTier::Subsystem,
        None,
        Some("src".to_string()),
        &proj.id,
        &repo.id,
    );
    let module = create_test_entity(
        "module1",
        "module",
        EntityTier::Module,
        Some(subsystem.id.clone()),
        Some("src/mod".to_string()),
        &proj.id,
        &repo.id,
    );
    let file = create_test_entity(
        "file1",
        "file.rs",
        EntityTier::File,
        Some(module.id.clone()),
        Some("src/mod/file.rs".to_string()),
        &proj.id,
        &repo.id,
    );

    s.upsert_entity(&subsystem).unwrap();
    s.upsert_entity(&module).unwrap();
    s.upsert_entity(&file).unwrap();

    // Clear file summaries
    clear_file_summaries(&s, &proj.id, "src/mod/file.rs").unwrap();

    // Verify file summary AND summary_commit cleared
    let file_updated = s.get_entity(&file.id).unwrap().unwrap();
    assert!(file_updated.summary.is_none());
    assert!(file_updated.summary_commit.is_none());

    // Verify module summary AND summary_commit cleared
    let module_updated = s.get_entity(&module.id).unwrap().unwrap();
    assert!(module_updated.summary.is_none());
    assert!(module_updated.summary_commit.is_none());

    // Verify subsystem summary AND summary_commit cleared
    let subsystem_updated = s.get_entity(&subsystem.id).unwrap().unwrap();
    assert!(subsystem_updated.summary.is_none());
    assert!(subsystem_updated.summary_commit.is_none());

    // Guard for the naive-COALESCE trap: after a deliberate clear, a subsequent
    // structural re-persist (incoming summary = None) must keep the cleared state
    // NULL — not resurrect the previously-stored summary. Without the dedicated
    // clear path, upsert_entity with a NULL summary would leave the old text in place
    // (making the clear a no-op), or a blanket COALESCE that resurrects on re-persist.
    let mut stale_structural = file_updated.clone();
    stale_structural.summary = None; // "no summary to offer" (structural pass)
    stale_structural.summary_commit = None;
    s.upsert_entity(&stale_structural).unwrap();

    let file_after_structural = s.get_entity(&file.id).unwrap().unwrap();
    assert!(
        file_after_structural.summary.is_none(),
        "a cleared summary must not be resurrected by a subsequent structural upsert"
    );
    assert!(file_after_structural.summary_commit.is_none());
}

#[test]
fn test_clear_file_summaries_non_file_entity_returns_descriptive_error() {
    let s = storage();
    let proj = s.create_project("testproj", None).unwrap();
    let repo = s.add_repo(&proj.id, "testrepo", "/tmp/test").unwrap();

    // Create a module entity (not a file)
    let module = create_test_entity(
        "module1",
        "module",
        EntityTier::Module,
        None,
        Some("src/mod".to_string()),
        &proj.id,
        &repo.id,
    );
    s.upsert_entity(&module).unwrap();

    // Try to clear summaries for a module path
    let result = clear_file_summaries(&s, &proj.id, "src/mod");
    assert!(result.is_err());

    // Error should show actual entity tier (Module)
    let err = result.unwrap_err();
    let err_msg = format!("{:?}", err);
    assert!(err_msg.contains("Module"));
    assert!(err_msg.contains("not a File"));
}
