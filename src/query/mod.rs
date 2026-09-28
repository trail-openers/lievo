// Query API - entity lookup, dependency traversal, intelligence queries
// Includes: entity queries, dependency queries

pub mod dependency;
pub mod entity_queries;
pub mod project_boundary;
pub use project_boundary::same_project;
