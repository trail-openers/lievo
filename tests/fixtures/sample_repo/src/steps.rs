// #[path] fixture module for issue #732: declares the `cleanup` module with
// `#[path = "steps/cleanup.rs"]`. The target `steps/cleanup.rs` resolves to
// `src/steps/cleanup.rs` relative to this file's directory — the same as the
// literal module mapping, so the `#[path]` attribute is exercised (parsed by
// the selfcheck's alias-map scanner) without a physical/logical divergence
// (which the production extractor does not yet handle — see issue #732).
//
// The `crate::steps::cleanup::cleanup_label` specifier in src/main.rs must
// resolve to `src/steps/cleanup.rs` via the literal path mapping, and the
// `cleanup_label()` call below must record an import edge to the same file.
#[path = "steps/cleanup.rs"]
mod cleanup;

pub fn run_steps() -> &'static str {
    cleanup_label()
}
