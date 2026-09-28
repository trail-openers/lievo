//! Response-assembly helpers for the files-mode batch path (extracted from
//! `tools_explore_files.rs` to keep it under the 500-line source budget,
//! issue #848). Owns the size-probe shape, the final response assembly,
//! and the continuation instruction.

use serde_json::{Value, json};

use crate::retrieval::explore_common::continuation_pointer;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;

/// Fit-probe response for the packing and depth-shed loops: the full
/// response shape (symbols, not_shown_files, returned/total, completeness)
/// with the current `chosen` prefix plus candidate, so the serialized size
/// measured in the loop is a faithful bound on the final response (the final
/// continuation pointer, when it fires, only adds a few hundred chars the
/// cap's `cap_response` path then bounds).
pub(crate) fn files_probe(
    symbols: &[Value],
    not_shown_files: &[String],
    total: usize,
    omitted_files: usize,
) -> Value {
    json!({
        "symbols": symbols,
        "not_shown_files": not_shown_files,
        "returned": symbols.len(),
        "total": total,
        "completeness": {
            "complete": not_shown_files.is_empty(),
            "omitted_files": omitted_files,
            "omitted_edges": 0,
        },
    })
}

/// Context for assembling the files-mode response.
pub(crate) struct FilesResponseCtx<'a> {
    pub(crate) symbols: &'a [Value],
    pub(crate) not_shown_files: &'a [String],
    pub(crate) not_found: &'a [String],
    pub(crate) total: usize,
    pub(crate) include_source: bool,
    pub(crate) complete: bool,
    pub(crate) omitted_files: usize,
}

/// Assemble the files-mode response: the symbols array, the `not_shown_files`
/// array naming every budget-dropped path, the `not_found` array (separate
/// set for unresolvable paths), returned/total counts, the structured
/// `completeness` object (bundle shape: {complete, omitted_files,
/// omitted_edges}), and a continuation pointer naming the exact remainder
/// when the batch is partial.
pub(crate) fn files_response(ctx: &FilesResponseCtx) -> Value {
    let returned = ctx.symbols.len();
    let mut response = json!({
        "symbols": ctx.symbols,
        "not_shown_files": ctx.not_shown_files,
        "returned": returned,
        "total": ctx.total,
        "completeness": {
            "complete": ctx.complete,
            "omitted_files": ctx.omitted_files,
            "omitted_edges": 0,
        },
    });
    if !ctx.not_found.is_empty() {
        response["not_found"] = json!(ctx.not_found);
    }
    if !ctx.not_shown_files.is_empty() {
        // The remainder is surfaced via the EXISTING continuation-pointer
        // mechanism (returned/total/next): `next` names a follow-up
        // lievo_explore call carrying the exact remaining paths in order.
        let next = files_next_instruction(ctx.not_shown_files, ctx.include_source);
        response["continuation"] = json!(continuation_pointer(returned, ctx.total, &next));
    }
    response
}

/// Compose the `next` instruction for a partial batch: a follow-up
/// `lievo_explore` call carrying exactly the remaining paths, in order.
fn files_next_instruction(remainder: &[String], include_source: bool) -> String {
    let list = remainder
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("lievo_explore(files=[{list}], include_source={include_source})")
}

/// A single symbol fits when the full single-file response shape (symbols +
/// not_shown_files + completeness + framing) serializes under the cap.
pub(crate) fn single_fits(sym: &Value) -> bool {
    let probe = files_probe(std::slice::from_ref(sym), &[], 1, 0);
    probe.to_string().chars().count() <= MAX_EXPLORE_OUTPUT_CHARS
}
