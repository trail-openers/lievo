//! Project-boundary helper for query-layer relationship traversals (issue #764).
//!
//! The `relationships` table has no project_id column:
//! `relationships_from` and `relationships_to` resolve purely by globally-
//! unique entity id, so any query that walks edges by id can silently cross
//! project boundaries when multiple projects share one database. Every query
//! must filter its endpoints against the queried entity's own project.
//!
//! This re-exports the retrieval-layer implementation so the query layer can
//! share the same single mechanism (mirrors how the blast_radius fix from
//! #759 was generalized into `retrieval::project_boundary`).
pub use crate::retrieval::project_boundary::same_project;
