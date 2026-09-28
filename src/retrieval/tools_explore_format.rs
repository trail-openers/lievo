//! Shared helpers for `lievo_explore` source formatting (issue #864:
//! extracted from `tools_explore.rs` to keep it under the 500-line budget).

/// Body-size threshold: a symbol whose source body is strictly smaller than
/// this is inlined in the tier-1 map instead of summarized (Complexity Trap).
/// Boundaries are exclusive: a body of exactly this size gets a summary.
pub const SMALL_BODY_THRESHOLD_CHARS: usize = 500;

/// Format a source body as verbatim line-numbered text (Read-tool shape).
pub fn line_numbered_source(source: &str) -> String {
    source
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, line)| format!("{}\t{}", i + 1, line))
        .collect()
}

/// True when the source body is small enough to inline in the tier-1 map
/// instead of summarized (`None` body returns false).
pub fn is_small_body(body: Option<&str>) -> bool {
    body.map(|b| b.chars().count() < SMALL_BODY_THRESHOLD_CHARS)
        .unwrap_or(false)
}
