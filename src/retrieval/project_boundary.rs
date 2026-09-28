//! Project-boundary helpers for relationship traversals (issue #764).
//!
//! The `relationships` table has no project_id column: `relationships_from`
//! and `relationships_to` resolve purely by globally-unique entity id, so
//! any traversal that walks edges by id can silently cross project
//! boundaries when multiple projects share one database. Every traversal
//! must filter its endpoints against its own project at EVERY hop — this
//! module is the single shared mechanism for that filter, mirroring the
//! project check the #759 blast_radius fix introduced locally.
//!
//! The endpoint entity row carries its `project_id` explicitly — trust the
//! row, not the id (an id embeds a project prefix by convention, but the
//! row is the source of truth). An endpoint that lacks a stored entity row
//! is treated as same-project: the traversal already skips endpoints whose
//! entity row does not exist (e.g. blast_radius drops them via
//! `owning_file`), and a missing row must never masquerade as a
//! cross-project intrusion.

/// True when `entity_project_id` belongs to `project_id`.
pub fn same_project(project_id: &str, entity_project_id: &str) -> bool {
    entity_project_id == project_id
}
