//! Shared helper for exercising the CLI binary's command-handler modules.
//!
//! The `lievo` binary is a directory-style binary (`src/bin/lievo/`) with
//! binary-local `commands` modules. The `status` handler's JSON output
//! formatter (`format_status_json`) is exercised without process stdout
//! capture by compiling `query_ops.rs` directly into this test crate via
//! `#[path]` — no re-exports are added to the public library API
//! (see issue #631).
//!
//! `query_ops.rs` (compiled into this test crate via `#[path]`) declares
//! an inline `#[cfg(test)] mod tests` that references
//! `super::{status, format_status_json, StatusStats}` — the re-exports
//! below make that resolve when the file compiles under this test crate.
//! (`query_ops.rs` handles its own `json_escape` via a `#[cfg]` split.)
#[path = "../src/bin/lievo/commands/project/query_ops.rs"]
mod query_ops;

pub use query_ops::{StatusStats, format_status_json, status};
