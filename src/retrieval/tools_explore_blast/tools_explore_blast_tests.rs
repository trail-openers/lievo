// Issue #759: blast_radius is INCOMING-only, includes Imports as well as
// Calls, transitive to BLAST_RADIUS_MAX_HOPS, cycle-safe, and normalized to
// the owning file entity. Wired via `#[path]` from tools_explore_blast.rs.
//
// Issue #767: hop-0 direct-dependent count and lean completeness hint.

use std::sync::Mutex;

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tools_explore_blast::call_paths_and_blast_radius;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use serde_json::Value;

// Storage adapter: delegates the 4 methods called by
// call_paths_and_blast_radius to a real SqliteStorage; all others unimplemented!().
pub(crate) struct SqliteBox(pub(crate) Mutex<SqliteStorage>);

impl SqliteBox {
    pub(crate) fn new(storage: SqliteStorage) -> Self {
        Self(Mutex::new(storage))
    }
}

impl Storage for SqliteBox {
    fn get_entity(&self, id: &str) -> crate::Result<Option<Entity>> {
        self.0.lock().unwrap().get_entity(id)
    }
    fn entities_by_parent(&self, parent_id: &str) -> crate::Result<Vec<Entity>> {
        self.0.lock().unwrap().entities_by_parent(parent_id)
    }
    fn relationships_from(&self, source_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        self.0.lock().unwrap().relationships_from(source_id)
    }
    fn relationships_to(&self, target_id: &str) -> crate::Result<Vec<(Relationship, Entity)>> {
        self.0.lock().unwrap().relationships_to(target_id)
    }
    // All other methods — never called in these tests.
    fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<crate::model::Project> {
        unimplemented!()
    }
    fn get_project(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn add_output_dir(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_output_dirs(&self, _: &str) -> crate::Result<Vec<String>> {
        unimplemented!()
    }
    fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<crate::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _: &str) -> crate::Result<Option<crate::model::Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _: &str) -> crate::Result<Vec<crate::model::Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn record_unresolved_counts(&self, _: &str, _: u64, _: u64) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn upsert_entity(&self, _: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn search_entities_by_name(
        &self,
        _: &str,
        _: &[&str],
        _: usize,
        _: Option<&str>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_repo(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn get_unresolved_counts(&self, _: &str) -> Option<(u64, u64)> {
        unimplemented!()
    }
    fn entity_by_path(&self, _: &str, _: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entity_ids_for_paths(
        &self,
        _: &str,
        _: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _: &Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_all_relationships(&self, _: &str) -> crate::Result<Vec<Relationship>> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _: &crate::model::Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> crate::Result<Vec<crate::model::Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(&self, _: &str, _: &str) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _: &crate::model::AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_all_file_hashes(
        &self,
        _: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn delete_file_hash(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _: &[&Entity],
        _: &[Relationship],
        _: &crate::model::AnalysisRun,
        _: &str,
        _: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _: &str,
        _: &str,
        _: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn count_missing_summaries(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
}
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn setup() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let project = storage
        .create_project(&format!("blast-{unique}"), None)
        .unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

pub(crate) fn file_entity(
    id: &str,
    project_id: &str,
    repo_id: &str,
    name: &str,
    path: &str,
    language: &str,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some(language.to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

pub(crate) fn func_entity(
    id: &str,
    project_id: &str,
    repo_id: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::Function,
        parent_id: parent_id.map(|s| s.to_string()),
        name: name.to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

pub(crate) fn dep_edge(storage: &SqliteStorage, source: &str, target: &str, rel_type: RelType) {
    storage
        .upsert_relationship(&Relationship {
            source_id: source.to_string(),
            target_id: target.to_string(),
            rel_type,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();
}

fn entry_named<'a>(blast: &'a [Value], name: &str) -> Option<&'a Value> {
    blast.iter().find(|e| e["name"] == name)
}

#[test]
fn imported_js_component_reports_all_importers_as_blast_radius() {
    // The exact BannerSection regression: a JS component that is imported N
    // times but never called. Before the fix it returned zero entries.
    let (storage, project_id, repo_id) = setup();
    let banner = file_entity(
        "p:repo1:file:src/banner.tsx",
        &project_id,
        &repo_id,
        "banner",
        "src/banner.tsx",
        "TypeScript",
    );
    storage.upsert_entity(&banner).unwrap();
    for importer in [
        "src/home.tsx",
        "src/store.tsx",
        "src/cart.tsx",
        "src/checkout.tsx",
    ] {
        let importer_id = format!("p:repo1:file:{importer}");
        let ent = file_entity(
            &importer_id,
            &project_id,
            &repo_id,
            importer
                .rsplit('/')
                .next()
                .unwrap()
                .strip_suffix(".tsx")
                .unwrap(),
            importer,
            "TypeScript",
        );
        storage.upsert_entity(&ent).unwrap();
        dep_edge(&storage, &importer_id, &banner.id, RelType::Imports);
    }

    let boxed = SqliteBox::new(storage);
    let (call_paths, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &banner);

    // Imported but never called: call_paths is empty.
    assert!(
        call_paths.is_empty(),
        "imported component has no call edges: {call_paths:?}"
    );
    // All four importers appear, with the Imports relation type.
    assert_eq!(blast.len(), 4, "expected 4 importers, got: {blast:?}");
    for name in ["home", "store", "cart", "checkout"] {
        let entry = entry_named(&blast, name)
            .unwrap_or_else(|| panic!("importer '{name}' missing from blast_radius: {blast:?}"));
        assert_eq!(
            entry["rel_types"],
            Value::from(vec![Value::from("imports")])
        );
    }
}

#[test]
fn outgoing_dependencies_never_appear_in_blast_radius() {
    // caller.rs (target) has function `fn_caller` which CALLS fn_other in
    // other.rs. other.rs is a dependency, not a dependent — it must NOT be
    // in caller's blast_radius.
    let (storage, project_id, repo_id) = setup();
    let caller = file_entity(
        "p:repo1:file:caller.rs",
        &project_id,
        &repo_id,
        "caller",
        "caller.rs",
        "Rust",
    );
    let other = file_entity(
        "p:repo1:file:other.rs",
        &project_id,
        &repo_id,
        "other",
        "other.rs",
        "Rust",
    );
    let fn_caller = func_entity(
        "fn-caller",
        &project_id,
        &repo_id,
        "fn_caller",
        Some("p:repo1:file:caller.rs"),
    );
    let fn_other = func_entity(
        "fn-other",
        &project_id,
        &repo_id,
        "fn_other",
        Some("p:repo1:file:other.rs"),
    );
    for e in [&caller, &other, &fn_caller, &fn_other] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-caller", "fn-other", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (call_paths, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &caller);

    assert!(
        blast.is_empty(),
        "outgoing call must not appear in blast_radius: {blast:?}"
    );
    // call_paths still carries the outgoing edge (reference shape unchanged).
    let out = call_paths.iter().any(|cp| {
        cp["direction"] == "out" && cp["name"] == "fn_caller" && cp["calls"] == "fn_other"
    });
    assert!(
        out,
        "call_paths must still carry the outgoing edge: {call_paths:?}"
    );
}

#[test]
fn every_blast_entry_carries_direction_in_and_relation_type() {
    let (storage, project_id, repo_id) = setup();
    let target = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let importer = file_entity(
        "p:repo1:file:i.js",
        &project_id,
        &repo_id,
        "i",
        "i.js",
        "JavaScript",
    );
    let fn_caller = func_entity(
        "fn-i",
        &project_id,
        &repo_id,
        "fn_i",
        Some("p:repo1:file:i.js"),
    );
    let fn_target = func_entity(
        "fn-t",
        &project_id,
        &repo_id,
        "fn_t",
        Some("p:repo1:file:t.js"),
    );
    for e in [&target, &importer, &fn_caller, &fn_target] {
        storage.upsert_entity(e).unwrap();
    }
    // Same file reached through two edge types: import + call.
    dep_edge(
        &storage,
        "p:repo1:file:i.js",
        "p:repo1:file:t.js",
        RelType::Imports,
    );
    dep_edge(&storage, "fn-i", "fn-t", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);

    assert_eq!(
        blast.len(),
        1,
        "imported+called file normalizes to ONE entry: {blast:?}"
    );
    let entry = &blast[0];
    assert_eq!(entry["direction"], "in");
    assert_eq!(entry["tier"], "file");
    let types = entry["rel_types"].as_array().unwrap();
    // Issue #834: the calls edge still normalizes the two edges into ONE
    // entry (the merge site still dedupes on file id), but only the non-call
    // rel type (imports) is LABELLED — "calls" is dropped from the output.
    assert_eq!(
        types.len(),
        1,
        "one entry, labelled with the non-call rel type only: {entry:?}"
    );
    let has_imports = types.iter().any(|t| t == "imports");
    assert!(has_imports, "the import edge must be labelled: {entry:?}");
    assert!(
        !types.iter().any(|t| t == "calls"),
        "calls must not be labelled (issue #834): {entry:?}"
    );
}

/// (case name, file entities as (id, name, path), edges as (src, tgt),
/// target id, expected blast names in BFS order)
type BlastCase = (
    &'static str,
    &'static [(&'static str, &'static str, &'static str)],
    &'static [(&'static str, &'static str)],
    &'static str,
    &'static [&'static str],
);

#[test]
fn simple_blast_radius_invariants_two_hop_cycle_self_bidirectional() {
    // Consolidated: four small invariants that share the same shape (file
    // entities + Imports edges + a target). Run as a table to keep the file
    // under the 800-line test budget (issue #834 added a sibling emit test
    // file for the new invariants).
    let cases: Vec<BlastCase> = vec![
        (
            "two-hop reverse closure returns direct and indirect dependents (BFS order mid before top)",
            &[
                ("p:repo1:file:bottom.js", "bottom", "bottom.js"),
                ("p:repo1:file:mid.js", "mid", "mid.js"),
                ("p:repo1:file:top.js", "top", "top.js"),
            ],
            &[
                ("p:repo1:file:mid.js", "p:repo1:file:bottom.js"),
                ("p:repo1:file:top.js", "p:repo1:file:mid.js"),
            ],
            "p:repo1:file:bottom.js",
            &["mid", "top"],
        ),
        (
            "cyclic import graph terminates and does not duplicate",
            &[
                ("p:repo1:file:a.js", "a", "a.js"),
                ("p:repo1:file:b.js", "b", "b.js"),
            ],
            &[
                ("p:repo1:file:a.js", "p:repo1:file:b.js"),
                ("p:repo1:file:b.js", "p:repo1:file:a.js"),
            ],
            "p:repo1:file:a.js",
            &["b"],
        ),
        (
            "self-edge does not appear in own blast radius",
            &[("p:repo1:file:self.js", "self", "self.js")],
            &[("p:repo1:file:self.js", "p:repo1:file:self.js")],
            "p:repo1:file:self.js",
            &[],
        ),
        (
            "bidirectional edges emit a single dependent entry",
            &[
                ("p:repo1:file:a.js", "a", "a.js"),
                ("p:repo1:file:b.js", "b", "b.js"),
            ],
            &[
                ("p:repo1:file:a.js", "p:repo1:file:b.js"),
                ("p:repo1:file:b.js", "p:repo1:file:a.js"),
            ],
            "p:repo1:file:a.js",
            &["b"],
        ),
    ];
    for (case_name, files, edges, target_id, expected) in cases {
        let (storage, project_id, repo_id) = setup();
        for (id, name, path) in files {
            let ent = file_entity(id, &project_id, &repo_id, name, path, "JavaScript");
            storage.upsert_entity(&ent).unwrap();
        }
        for (src, tgt) in edges {
            dep_edge(&storage, src, tgt, RelType::Imports);
        }
        let target = storage.get_entity(target_id).unwrap().unwrap();
        let boxed = SqliteBox::new(storage);
        let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &target);
        let names: Vec<&str> = blast.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(
            &names[..],
            expected,
            "case '{case_name}': blast entries must equal expected (BFS order)"
        );
        // The bidirectional/cycle/self cases collapse to a single entry whose
        // `direction` the consolidated shape used to pin explicitly — keep the
        // pin here: a blast entry is an INCOMING dependent, never "out".
        for e in &blast {
            assert_eq!(e["direction"], "in", "case '{case_name}'");
        }
    }
}

#[test]
fn hop1_file_importer_with_hop2_child_function_collapses_to_one_entry() {
    // The operator's collision case: file F is a hop-1 importer of target T.
    // F has a child function g, and g CALLS a function h in T. At hop 2 the
    // traversal walks into F (the file) AND into g (the function in F). Both
    // normalize to F — a single entry, not two.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.js",
        &project_id,
        &repo_id,
        "t",
        "t.js",
        "JavaScript",
    );
    let f = file_entity(
        "p:repo1:file:f.js",
        &project_id,
        &repo_id,
        "f",
        "f.js",
        "JavaScript",
    );
    let g = func_entity(
        "fn-g",
        &project_id,
        &repo_id,
        "g",
        Some("p:repo1:file:f.js"),
    );
    let h = func_entity(
        "fn-h",
        &project_id,
        &repo_id,
        "h",
        Some("p:repo1:file:t.js"),
    );
    for e in [&t, &f, &g, &h] {
        storage.upsert_entity(e).unwrap();
    }
    // f imports t (file->file, hop 1 from t's perspective: f is t's dependent).
    dep_edge(
        &storage,
        "p:repo1:file:f.js",
        "p:repo1:file:t.js",
        RelType::Imports,
    );
    // g (in f) calls h (in t) — a second incoming edge to t, from a function
    // inside f.
    dep_edge(&storage, "fn-g", "fn-h", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);

    // f.js imports t (file->file, hop 1) AND fn-g (in f.js) calls fn-h (in
    // t). The calls edge still drives traversal (g enters the frontier), but
    // it is no longer LABELLED on the entry (issue #834: calls rel_types are
    // dropped). So f.js appears once, labelled with "imports" only — the
    // call-derived reachability is preserved, the "calls" label is gone.
    assert_eq!(
        blast.len(),
        1,
        "f must appear once (file importer + function caller in f): {blast:?}"
    );
    let entry = &blast[0];
    assert_eq!(entry["name"], "f");
    let types = entry["rel_types"].as_array().unwrap();
    // Only the non-call rel type is labelled on the merged entry.
    assert_eq!(
        types.len(),
        1,
        "only non-call rel type labelled (calls dropped): {entry:?}"
    );
    assert!(
        types.iter().any(|t| t == "imports"),
        "imports must be listed: {entry:?}"
    );
    assert!(
        !types.iter().any(|t| t == "calls"),
        "calls must NOT be listed (issue #834): {entry:?}"
    );
}

#[test]
fn sibling_function_edge_within_target_file_does_not_appear() {
    // Target file T has function t1; another function t2 in the SAME file
    // calls t1. t2 is a sibling of the target, not a dependent. It must not
    // appear in T's blast radius.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.rs",
        &project_id,
        &repo_id,
        "t",
        "t.rs",
        "Rust",
    );
    let t1 = func_entity(
        "fn-t1",
        &project_id,
        &repo_id,
        "t1",
        Some("p:repo1:file:t.rs"),
    );
    let t2 = func_entity(
        "fn-t2",
        &project_id,
        &repo_id,
        "t2",
        Some("p:repo1:file:t.rs"),
    );
    for e in [&t, &t1, &t2] {
        storage.upsert_entity(e).unwrap();
    }
    dep_edge(&storage, "fn-t2", "fn-t1", RelType::Calls);

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);
    assert!(
        blast.is_empty(),
        "sibling function within target file must not appear: {blast:?}"
    );
}

#[test]
fn contains_edges_are_excluded_from_blast_radius() {
    // A Contains edge pointing at the target is structural, not a
    // dependency — it must not produce a blast_radius entry.
    let (storage, project_id, repo_id) = setup();
    let t = file_entity(
        "p:repo1:file:t.rs",
        &project_id,
        &repo_id,
        "t",
        "t.rs",
        "Rust",
    );
    let parent = Entity {
        id: "p:repo1:mod:parent".to_string(),
        project_id: project_id.clone(),
        repo_id: Some(repo_id.clone()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "parent".to_string(),
        path: Some("parent".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&t).unwrap();
    storage.upsert_entity(&parent).unwrap();
    dep_edge(
        &storage,
        "p:repo1:mod:parent",
        "p:repo1:file:t.rs",
        RelType::Contains,
    );

    let boxed = SqliteBox::new(storage);
    let (_, blast, _, _, _) = call_paths_and_blast_radius(&boxed, &t);
    assert!(
        blast.is_empty(),
        "Contains edges are structural, not dependencies: {blast:?}"
    );
}
