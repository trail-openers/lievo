// Lievo — Codebase knowledge bank
//
// This library provides structural analysis, dependency graphs, and
// structured knowledge access for codebases across multiple repositories.

/// Analysis pipeline: relationships, insights, metrics, incremental updates, and the coverage gate.
pub mod analysis;
/// Configuration parser for optional per-repository .lievo/config.yaml overrides.
pub mod config;
#[cfg(test)]
mod config_tests;
/// Error types: the LievoError enum, machine-readable ErrorCode, and the crate Result alias.
pub mod error;
/// Repository identity: normalized origin-remote identity keys and the `identity:` override (issue #28).
pub mod identity;
pub use error::{LievoError, Result};
/// Top-level Lievo facade: composes the SQLite storage into a single thread-safe entry point.
pub mod api;
/// Extraction pipeline: tree-sitter based code parsing, entity grouping, and metric computation.
pub mod extraction;
/// MCP (Model Context Protocol) server exposing lievo tools, prompts, and tool interception.
pub mod mcp;
/// Core data models: Project, Repository, Entity, Relationship, Insight, and AnalysisRun types.
pub mod model;
/// Output formatting for query results: human and JSON/NDJSON formatters per entity type.
pub mod output;
/// Project resolution shared by the MCP server and CLI: optional name lookup with auto-selection.
pub mod project_resolution;
/// Query API: entity lookup, dependency traversal, and impact analysis.
pub mod query;
/// Shared repository registration entry point (issue #29): one `register` used by both MCP and CLI.
pub mod registration;

/// Single process-wide lock for every in-crate test that mutates or reads
/// environment variables (issue #863). `std::env::set_var` is process-global
/// (and `unsafe` in the 2024 edition): a `set_var` in one thread racing a
/// `getenv` of ANY variable in another thread can corrupt the process
/// environment at the libc level, producing intermittent CI failures. Every
/// env-mutating lib test must hold this one lock — per-module locks do not
/// serialize against each other.
#[cfg(test)]
mod test_env_support {
    /// Lock the crate-wide env-var mutex, recovering from a poisoned mutex
    /// (a previous test panicked while holding it) so one failure does not
    /// cascade to every other env-mutating test.
    pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Isolate `HOME` + `LIEVO_DB` in temp dirs for the test's duration
    /// (issue #872: shared by the refresh env-based tests). Holds the
    /// crate-wide env lock; restores nothing (env mutation only lives as
    /// long as the guard).
    pub struct EnvGuard {
        pub _lock: std::sync::MutexGuard<'static, ()>,
        pub _home: tempfile::TempDir,
    }

    impl EnvGuard {
        pub fn new(db: &std::path::Path) -> Self {
            let _lock = env_lock();
            let _home = tempfile::tempdir().unwrap();
            unsafe {
                std::env::set_var("HOME", _home.path());
                std::env::set_var("LIEVO_DB", db);
            }
            Self { _lock, _home }
        }
    }
}

/// Retrieval layer: semantic search, graph queries, centrality, and agent-facing tool implementations.
pub mod retrieval;
/// Storage layer: the Storage trait and its SQLite implementation, schema, and queries.
pub mod storage;
/// Summarization: on-device code summarization via the apfel backend.
pub mod summarization;
pub use api::Lievo;
/// Auto-refresh: staleness detection and cheap-layer refresh of analyzed data.
pub mod refresh;
pub(crate) mod refresh_resume;
