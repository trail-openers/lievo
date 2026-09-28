// #[path] fixture child for issue #732: physical location of the module
// declared from `src/steps.rs` via `#[path = "steps/cleanup.rs"] mod cleanup;`.
// The `crate::steps::cleanup::cleanup_label` specifier in src/main.rs names
// this file's symbol; the literal module mapping (`src/steps/cleanup.rs`)
// lands here, so the independent resolver and the production extractor
// agree on the same physical file.
pub fn cleanup_label() -> &'static str {
    "from cleanup module"
}
