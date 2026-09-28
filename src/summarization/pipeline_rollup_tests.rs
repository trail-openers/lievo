// Rollup batch tests (issue #771): verifies the batch path end-to-end
// using a fake apfel binary that echoes N `#<n>:` lines matching input size.
//
// Measurement corpus: `fixture_corpora_20_modules`
//   - 18 Module entities with 3 summarized children each (packable)
//   - 2 Module entities with zero children (excluded before packing)
//   - 18 packable entities in the File tier
//   - Total: 36 Module-tier entities, 18 of which are packable
//
// Expected: batched path makes ~1 apfel invocation for 18 packable entities
// (they fit in one 8000-char batch); per-entity loop would make 18.
//   → before = 18, after ≈ 1, drop ≈ 94%

use crate::config::SummarizerBackend;
use crate::model::EntityTier;
use crate::summarization::apfel::BackendTransport;
use crate::summarization::pipeline::SummarizationPipeline;
use crate::summarization::pipeline_tests_fixtures::{
    TestStorage, fake_apfel_invocation_log, logging_fake_apfel_script, make_file_entity,
    make_module_entity, read_fake_apfel_invocations, write_fake_apfel,
};
use crate::summarization::summarizer_backend::start_flaky_http_server;
use std::path::PathBuf;
use tempfile::TempDir;

/// Install a fake `apfel` on PATH that echoes N `#<n>:` lines matching the
/// input size and appends one line per invocation to the per-test log file
/// (issue #869: the subprocess count the tests assert on is owned by the
/// test's own TempDir, never a process-global counter).
///
/// The `PATH` env var is process-wide; the caller must hold the crate-wide env
/// lock (issue #863) so concurrent tests cannot interleave their PATH
/// mutations.
fn install_n_echo() -> (String, PathBuf, TempDir, PathBuf) {
    // A `TempDir` rather than a fixed path in `std::env::temp_dir()` (issue
    // #863): two parallel tests or two concurrent runs can no longer collide
    // on the same fake `apfel` binary path.
    let dir_tmp = TempDir::new().unwrap();
    let bin = dir_tmp.path().join("apfel");
    let log = fake_apfel_invocation_log(dir_tmp.path());
    let body = "input=$(cat)\nn=$(printf '%s' \"$input\" | grep -c -- '--- Entry: #' || true)\n[ -z \"$n\" ] && n=0\npython3 -c \"import sys,json;n=int(sys.argv[1]);print(json.dumps({'content':'\\\\n'.join(f'#{i}: summary_for_{i}' for i in range(n))}))\" $n";
    write_fake_apfel(&bin, &logging_fake_apfel_script(&log, body));
    let orig = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{}", dir_tmp.path().display(), orig)) };
    (orig, dir_tmp.path().to_path_buf(), dir_tmp, log)
}

/// RAII guard that restores PATH and releases the `TempDir` (which removes
/// the directory on drop — the held binding is what keeps the unique path
/// alive for the whole test, issue #863).
struct G {
    orig: String,
    tmp: PathBuf,
    _dir: TempDir,
}
impl Drop for G {
    fn drop(&mut self) {
        unsafe {
            std::env::set_var("PATH", self.orig.clone());
        }
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

/// A loopback-only apfel-shaped transport pointing at a fake HTTP server
/// (issue #783: the transport is explicit, not ambient).
fn fake_apfel_transport(url: &str) -> BackendTransport {
    BackendTransport {
        url: url.to_string(),
        backend: SummarizerBackend::Apfel,
        model_name: None,
    }
}

/// Measurement test (issue #771): named fixture corpus `fixture_corpus_22_modules`
/// with 20 packable Module entities (3 summarized children each) + 2 zero-child
/// entities. Asserts the batched path makes fewer invocations than the per-entity
/// loop would (20).
#[test]
fn test_rollup_measurement_fixture_corpus() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    // Build the fixture corpus: 20 modules with 3 summarized children each
    // (packable) + 2 zero-child modules (excluded before packing).
    // 20 packable × ~200-char prompt > 8000-char budget → 2 batches.
    let mut storage = TestStorage::new();
    let mut module_entities = Vec::new();
    for i in 0..20 {
        let m = make_module_entity(&format!("m{i}"), &format!("mod_{i}"));
        module_entities.push(m.clone());
        for j in 0..3 {
            let mut c = make_file_entity(&format!("f{i}_{j}"), &format!("src/m{i}.rs"));
            c.summary = Some(format!("child summary {i}_{j}"));
            storage.add_children(&m.id, vec![c]);
        }
    }
    for i in 20..22 {
        let m = make_module_entity(&format!("m{i}"), &format!("mod_{i}"));
        module_entities.push(m.clone());
        // No children → excluded before packing.
    }
    storage.add_repo_tier_entities("test-repo", EntityTier::Module, module_entities);

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::Module,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    let invocations = read_fake_apfel_invocations(&log);
    let count = outcome.updated;

    // 20 packable entities, each with a ~200-char prompt → 2 batches
    // (the 8000-char budget fits ~10-12 per batch). The batched path makes
    // 2 invocations; the per-entity loop would have made 20.
    assert_eq!(
        count, 20,
        "expected 20 summarized entities (20 packable, 2 excluded)"
    );
    // Issue #793: the 2 zero-child modules are not silent — they are counted
    // in the no_children bucket of the first-class RollupOutcome.
    assert_eq!(outcome.skipped.no_children, 2);
    assert_eq!(outcome.skipped.total(), 2);
    assert!(
        invocations < 20,
        "batched path made {invocations} invocations for 20 entities; per-entity loop would make 20"
    );
    eprintln!(
        "[measurement] fixture_corpus_22_modules: {invocations} apfel invocation(s) for 20 packable entities (per-entity baseline: 20)"
    );
}

// ── Issue #790: offset-advance regression tests ────────────────────────────

/// 15 modules with 3 summarized children each, each child ~300 chars, so
/// every module's prompt is ~1.1 KB and the 15 packable entities pack into
/// 3 batches (7, 7, 1) under the 8000-char budget.
fn build_15_module_storage() -> TestStorage {
    let mut storage = TestStorage::new();
    let mut module_entities = Vec::new();
    for i in 0..15 {
        let m = make_module_entity(&format!("m{i}"), &format!("mod_{i}"));
        module_entities.push(m.clone());
        for j in 0..3 {
            let mut c = make_file_entity(&format!("f{i}_{j}"), &format!("src/m{i}_{j}.rs"));
            c.summary = Some(format!(
                "child {i}_{j} distinct content: {}",
                "x".repeat(300)
            ));
            storage.add_children(&m.id, vec![c]);
        }
    }
    storage.add_repo_tier_entities("test-repo", EntityTier::Module, module_entities);
    storage
}

/// Issue #790: a generic rollup batch FAILURE (not overflow) followed by
/// further successful batches must not corrupt the offset. The broken code
/// advanced `offset` twice for the failed batch, so subsequent batches'
/// summaries landed on the wrong entities.
///
/// With 15 entities (~1.1KB prompts each) the packer produces 3 batches
/// (7, 7, 1). The first batch (7 entities) fails deterministically; the
/// remaining 8 entities are summarized across 2 successful batches.
/// Every upserted entity must carry a valid summary.
///
/// `transport` is `None` for the subprocess leg (fake apfel on PATH) and
/// `Some(flaky)` for the HTTP leg (issue #783: explicit, not ambient).
fn run_15_module_generic_failure_case(transport: Option<&BackendTransport>) {
    let storage = build_15_module_storage();

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::Module,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        transport,
    )
    .unwrap();
    let count = outcome.updated;

    let upserted = storage.upserted.borrow();
    assert_eq!(
        count,
        upserted.len(),
        "rollup_to_tier reported {count} updates but {} upserts were recorded",
        upserted.len()
    );
    // The first batch failed (generic error → all-None results), so fewer
    // than 15 entities were upserted. The rest (from subsequent batches)
    // succeeded. The first batch's entities were not upserted.
    assert!(
        upserted.len() < 15,
        "expected fewer than 15 upserts (first batch failed); got {}",
        upserted.len()
    );
    assert!(
        !upserted.is_empty(),
        "expected at least one upsert from subsequent batches; got 0"
    );

    // Every upserted entity must have a valid summary.
    for (up_idx, entity) in upserted.iter().enumerate() {
        assert!(
            entity.summary.as_deref().is_some(),
            "upsert {up_idx} (entity {} / name {}) has no summary",
            entity.id,
            entity.name
        );
        let summary = entity.summary.as_deref().unwrap();
        assert!(
            summary.starts_with("summary_for_"),
            "upsert {up_idx} (entity {} / name {}) has invalid summary: {:?}",
            entity.id,
            entity.name,
            summary
        );
    }
}

/// Generic batch failure via the flaky HTTP server: the first connection
/// (batch 0) fails with 500, subsequent connections succeed. The transport
/// is the explicit flaky-server value (issue #783) — `Connection: close` on
/// every response guarantees a fresh connection per request, so
/// `flaky_first=1` reliably fails exactly one batch.
#[test]
fn test_rollup_generic_batch_failure_http() {
    let addr = start_flaky_http_server(
        1,
        500,
        "{\"error\":\"boom\"}",
        200,
        // Fixed response for all successful batches. The OpenAI chat-completions
        // wrapper is required by `send_chat_completions_for`. `parse_batch_response`
        // extracts #0–#6 (7 entries) from a 7-entry batch, or #0 from a singleton.
        // Extra indices beyond the batch size are silently skipped.
        "{\"choices\":[{\"message\":{\"content\":\"#0: summary_for_0\\n#1: summary_for_1\\n#2: summary_for_2\\n#3: summary_for_3\\n#4: summary_for_4\\n#5: summary_for_5\\n#6: summary_for_6\",\"role\":\"assistant\"}}]}",
    )
    .expect("bind flaky server");
    let url = format!("http://{addr}");
    let transport = fake_apfel_transport(&url);

    // The HTTP path makes no apfel subprocess invocations (the flaky server
    // answers over HTTP), so the offset fix is proven by the upsert
    // assertions inside the helper, not by a subprocess invocation count.
    run_15_module_generic_failure_case(Some(&transport));
}

/// Generic batch failure via the fake-apfel subprocess: the first invocation
/// exits 1 (a generic, non-overflow failure) and the following invocations
/// succeed, echoing `#<i>: summary_for_<i>` lines for each batch.
#[test]
fn test_rollup_generic_batch_failure_subprocess() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let dir_tmp = TempDir::new().unwrap();
    let bin = dir_tmp.path().join("apfel");
    let log = fake_apfel_invocation_log(dir_tmp.path());
    let marker = dir_tmp.path().join(".invoked");
    let marker_str = marker.display().to_string();
    // The fake apfel script fails on the first invocation (marker file absent)
    // and echoes batch-local `#<i>: summary_for_<i>` lines on subsequent
    // calls. It also appends to the per-test invocation log (issue #869).
    let body = format!(
        "if [ ! -f \"{marker_str}\" ]; then echo 'forced failure' >&2; touch \"{marker_str}\"; exit 1; fi\ninput=$(cat)\nn=$(printf '%s' \"$input\" | grep -c -- '--- Entry: #' || true)\n[ -z \"$n\" ] && n=0\npython3 -c \"import sys,json;n=int(sys.argv[1]);print(json.dumps({{'content':'\\\\n'.join(f'#{{i}}: summary_for_{{i}}' for i in range(n))}}))\" $n"
    );
    write_fake_apfel(&bin, &logging_fake_apfel_script(&log, &body));
    let orig = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{}", dir_tmp.path().display(), orig)) };
    let _g = G {
        orig,
        tmp: dir_tmp.path().to_path_buf(),
        _dir: dir_tmp,
    };

    run_15_module_generic_failure_case(None);

    // The first batch failed, the remaining 8 entities were summarized across
    // 2 batches, and the fake apfel was invoked for exactly the 3 batches
    // (one per batch — the subprocess path, issue #869).
    let invocations = read_fake_apfel_invocations(&log);
    assert_eq!(
        invocations, 3,
        "expected 3 fake apfel invocations (one per batch); got {invocations}"
    );
}

/// Issue #790: a single over-budget entry takes the overflow arm and is
/// counted in `skipped_oversized` without shifting any subsequent batch. The
/// broken generic arm is adjacent to the overflow arm — pinning this path so
/// it cannot silently regress.
#[test]
fn test_rollup_overflow_arm_does_not_shift_subsequent_batches() {
    // Serializes the fake apfel subprocess against every other apfel-interacting
    // test (issue #869): without the guard, a concurrent test's fake apfel
    // invocation would inflate the per-test invocation log.
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    // 1 over-budget module (entity 0) + 6 normal modules (entities 1–6).
    let mut storage = TestStorage::new();
    let mut module_entities = Vec::new();

    let m0 = make_module_entity("m0", "mod_0");
    module_entities.push(m0.clone());
    let mut c0 = make_file_entity("f0_0", "src/m0.rs");
    c0.summary = Some("x".repeat(8500)); // child summary > 8000 → prompt over budget
    storage.add_children(&m0.id, vec![c0]);

    for i in 1..=6 {
        let m = make_module_entity(&format!("m{i}"), &format!("mod_{i}"));
        module_entities.push(m.clone());
        for j in 0..3 {
            let mut c = make_file_entity(&format!("f{i}_{j}"), &format!("src/m{i}_{j}.rs"));
            c.summary = Some(format!("child {i}_{j} content: {}", "y".repeat(60)));
            storage.add_children(&m.id, vec![c]);
        }
    }
    storage.add_repo_tier_entities("test-repo", EntityTier::Module, module_entities);

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::Module,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    let invocations = read_fake_apfel_invocations(&log);
    let count = outcome.updated;

    // Entity 0 is oversized and skipped (overflow arm, no apfel invocation);
    // the 6 normal modules pack into 1 batch → 1 successful invocation.
    assert_eq!(
        invocations, 1,
        "expected 1 apfel invocation; got {invocations}"
    );
    assert_eq!(count, 6, "expected 6 summarized entities; got {count}");

    // The upserted entities are the 6 normal modules (m1–m6), in order.
    // Their summaries come from the fake apfel's n-echo: #0→summary_for_0,
    // #1→summary_for_1, etc. — matching the batch-local positions 0–5.

    let upserted = storage.upserted.borrow();
    assert_eq!(
        upserted.len(),
        6,
        "expected 6 upserts; got {}",
        upserted.len()
    );
    for (up_idx, entity) in upserted.iter().enumerate() {
        let expected = format!("summary_for_{up_idx}");
        assert_eq!(
            entity.summary.as_deref(),
            Some(expected.as_str()),
            "upsert {up_idx} (entity {} / name {}) got {:?}, expected {:?} — overflow arm shifted the offset",
            entity.id,
            entity.name,
            entity.summary,
            expected
        );
    }
}

/// Issue #790 control: the all-success path. All 15 modules are packable and
/// every batch succeeds. Every entity must receive a valid summary — no offset
/// drift on the clean path.
#[test]
fn test_rollup_all_success_control() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    let storage = build_15_module_storage();

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::Module,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    let invocations = read_fake_apfel_invocations(&log);
    let count = outcome.updated;

    assert!(
        invocations > 0,
        "expected at least 1 apfel invocation; got {invocations}"
    );
    assert_eq!(count, 15, "expected 15 summarized entities; got {count}");

    let upserted = storage.upserted.borrow();
    assert_eq!(
        upserted.len(),
        15,
        "expected 15 upserts; got {}",
        upserted.len()
    );
    for (up_idx, entity) in upserted.iter().enumerate() {
        assert!(
            entity.summary.as_deref().is_some(),
            "upsert {up_idx} (entity {} / name {}) has no summary",
            entity.id,
            entity.name
        );
        let summary = entity.summary.as_deref().unwrap();
        assert!(
            summary.starts_with("summary_for_"),
            "upsert {up_idx} (entity {} / name {}) has invalid summary: {:?}",
            entity.id,
            entity.name,
            summary
        );
    }
}

// ── Issue #827: File-tier source-text fallback ────────────────────────────

/// A File-tier entity with no children and non-empty source on disk is
/// summarized via the source-text fallback and re-bucketed into `updated`
/// (not `no_children`). The source read goes through the injectable
/// `read_source` seam, mockable via `TestStorage::add_repo` + a temp-dir
/// `local_path` (no `unimplemented!()` panic path).
#[test]
fn test_file_childless_non_empty_source_gets_summary() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    // Temp dir with a real source file; the repo mock points `local_path`
    // here so `read_source` can read it via the injectable seam.
    let src_dir = std::env::temp_dir().join(format!(
        "lievo-827-src-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("childless.rs"), "fn hello() -> i32 { 42 }\n").unwrap();

    let mut storage = TestStorage::new();
    storage.add_repo("test-repo", src_dir.to_str().unwrap());
    let entity = make_file_entity("f-childless", "childless.rs");
    storage.add_repo_tier_entities("test-repo", EntityTier::File, vec![entity.clone()]);

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::File,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    let invocations = read_fake_apfel_invocations(&log);

    // The entity is re-bucketed into `updated`, not `no_children`.
    assert_eq!(
        outcome.updated, 1,
        "expected 1 fallback summary; got {}",
        outcome.updated
    );
    assert_eq!(
        outcome.skipped.no_children, 0,
        "childless file with source must not be in no_children"
    );
    assert_eq!(outcome.skipped.total(), 0, "no skips expected");
    // The fallback makes exactly one summarize_code invocation.
    assert_eq!(
        invocations, 1,
        "expected 1 summarize_code invocation; got {invocations}"
    );

    // The upserted entity carries a summary and a summary_commit.
    let upserted = storage.upserted.borrow();
    assert_eq!(
        upserted.len(),
        1,
        "expected 1 upsert; got {}",
        upserted.len()
    );
    let up = &upserted[0];
    assert_eq!(up.id, "f-childless");
    assert!(up.summary.is_some(), "fallback entity must have a summary");
    assert!(
        up.summary_commit.is_some(),
        "fallback entity must have a summary_commit"
    );

    let _ = std::fs::remove_dir_all(&src_dir);
}

/// A File-tier entity with no children and empty/missing source stays in
/// `no_children` (the seam returns `Ok(None)` uniformly for empty/missing/
/// unreadable source). Two cases: file exists but empty; file absent entirely.
#[test]
fn test_file_childless_empty_or_missing_source_stays_no_children() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, _log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    // Case A: file exists but is empty → Ok(None) → no_children.
    let src_dir_a = std::env::temp_dir().join(format!(
        "lievo-827-empty-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&src_dir_a).unwrap();
    std::fs::write(src_dir_a.join("empty.rs"), "").unwrap();
    let mut storage_a = TestStorage::new();
    storage_a.add_repo("test-repo", src_dir_a.to_str().unwrap());
    let entity_a = make_file_entity("f-empty", "empty.rs");
    storage_a.add_repo_tier_entities("test-repo", EntityTier::File, vec![entity_a]);

    let outcome_a = SummarizationPipeline::rollup_to_tier(
        &storage_a,
        "test-repo",
        EntityTier::File,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    assert_eq!(outcome_a.updated, 0, "empty source must not be summarized");
    assert_eq!(
        outcome_a.skipped.no_children, 1,
        "empty source must be in no_children"
    );
    assert_eq!(
        storage_a.upserted.borrow().len(),
        0,
        "empty source must not be upserted"
    );
    let _ = std::fs::remove_dir_all(&src_dir_a);

    // Case B: file does not exist → Ok(None) → no_children.
    let src_dir_b = std::env::temp_dir().join(format!(
        "lievo-827-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&src_dir_b).unwrap();
    let mut storage_b = TestStorage::new();
    storage_b.add_repo("test-repo", src_dir_b.to_str().unwrap());
    let entity_b = make_file_entity("f-missing", "absent.rs");
    storage_b.add_repo_tier_entities("test-repo", EntityTier::File, vec![entity_b]);

    let outcome_b = SummarizationPipeline::rollup_to_tier(
        &storage_b,
        "test-repo",
        EntityTier::File,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();
    assert_eq!(
        outcome_b.updated, 0,
        "missing source must not be summarized"
    );
    assert_eq!(
        outcome_b.skipped.no_children, 1,
        "missing source must be in no_children"
    );
    let _ = std::fs::remove_dir_all(&src_dir_b);
}

/// A File-tier entity with no children and source exceeding the budget is
/// truncated at the `read_source` seam (char-boundary `take(budget)`), so
/// the fallback prompt never exceeds the budget and the overflow arm is
/// never hit. The entity is summarized and re-bucketed into `updated`.
#[test]
fn test_file_childless_over_budget_source_gets_truncated_summary() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    let src_dir = std::env::temp_dir().join(format!(
        "lievo-827-oversized-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&src_dir).unwrap();
    // Source is well over the 100-char budget used for this test.
    let big_source = "x".repeat(500);
    std::fs::write(src_dir.join("big.rs"), &big_source).unwrap();

    let mut storage = TestStorage::new();
    storage.add_repo("test-repo", src_dir.to_str().unwrap());
    let entity = make_file_entity("f-big", "big.rs");
    storage.add_repo_tier_entities("test-repo", EntityTier::File, vec![entity.clone()]);

    // Use a small budget (100) to force truncation. The resolved budget for
    // `summarize_code` with `transport=None` is APFEL_INPUT_CHAR_BUDGET
    // (8000), so the truncation at the seam is what matters — the seam caps
    // at `input_char_budget` (100 here), keeping the prompt well under 8000.
    let budget = 100;

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::File,
        budget,
        None,
    )
    .unwrap();
    let invocations = read_fake_apfel_invocations(&log);

    // The entity is summarized (truncated source fits the budget); no
    // overflow arm is hit, no skip.
    assert_eq!(
        outcome.updated, 1,
        "expected 1 truncated fallback summary; got {}",
        outcome.updated
    );
    assert_eq!(
        outcome.skipped.no_children, 0,
        "truncated source must not be in no_children"
    );
    assert_eq!(outcome.skipped.total(), 0, "no skips expected");
    assert_eq!(
        invocations, 1,
        "expected 1 summarize_code invocation; got {invocations}"
    );

    // The upserted entity carries a summary.
    let upserted = storage.upserted.borrow();
    assert_eq!(
        upserted.len(),
        1,
        "expected 1 upsert; got {}",
        upserted.len()
    );
    assert!(upserted[0].summary.is_some());

    let _ = std::fs::remove_dir_all(&src_dir);
}

/// A File-tier entity whose path matches `is_test_file_path` (policy skip,
/// #531) is NOT source-fallback summarized — policy takes precedence over
/// the fallback. The entity lands in the `policy` bucket, not `updated`.
#[test]
fn test_file_childless_test_path_policy_skips_fallback() {
    let _env_guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let (orig, tmp, dir, _log) = install_n_echo();
    let _g = G {
        orig,
        tmp,
        _dir: dir,
    };

    // Real readable source on disk — the fallback would fire if policy did
    // not take precedence. A `TempDir` (unique path, removed on drop — issue
    // #863) rather than a fixed path in `std::env::temp_dir()`.
    let src_dir = TempDir::new().unwrap();
    std::fs::write(
        src_dir.path().join("test_helper.rs"),
        "fn helper() -> i32 { 1 }\n",
    )
    .unwrap();

    let mut storage = TestStorage::new();
    storage.add_repo("test-repo", src_dir.path().to_str().unwrap());
    // `test_helper.rs` matches `is_test_file_path` (starts with "test_").
    let entity = make_file_entity("f-test", "test_helper.rs");
    storage.add_repo_tier_entities("test-repo", EntityTier::File, vec![entity.clone()]);

    let outcome = SummarizationPipeline::rollup_to_tier(
        &storage,
        "test-repo",
        EntityTier::File,
        crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .unwrap();

    // Policy skip: the entity is in the `policy` bucket, not `updated`.
    assert_eq!(
        outcome.updated, 0,
        "policy-skipped file must not be summarized"
    );
    assert_eq!(
        outcome.skipped.policy, 1,
        "test file must be in the policy bucket"
    );
    assert_eq!(
        outcome.skipped.no_children, 0,
        "policy skip must not be in no_children"
    );
    assert_eq!(
        storage.upserted.borrow().len(),
        0,
        "policy-skipped file must not be upserted"
    );
}
