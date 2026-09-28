// RelationshipBuilder - derives structural relationships from grouping + code units.
// No database I/O — pure in-memory graph construction.
//
// The import/call-edge aggregation bookkeeping (edge_weights / evidence_map /
// provenance_map, `emit_imports_edge`, the per-unit edge collection, and the
// final edge-materialisation loop) lives in `relationships_aggregate.rs`;
// `build_inner` itself is implemented there as well. This file keeps the
// public orchestration surface: `build`, `build_with_functions`, and the
// cross-repository builder.

use crate::Result;
use crate::extraction::grouping::GroupingResult;
use crate::model::{CodeUnit, EdgeProvenance, Entity, RelType, Relationship};
use std::path::Path;

use super::import_resolver::UnresolvedCounts;
use super::relationship_helpers::normalize_crate_name;

use std::collections::HashMap;

pub struct RelationshipBuilder;

impl RelationshipBuilder {
    /// Builds all relationships for a single repository.
    ///
    /// Produces four relationship types:
    /// - `contains`: subsystem→module and module→file (from grouping parent_ids)
    /// - `imports`: file→file (resolved from code unit `imports` AND `calls` — at file tier, all cross-file dependencies are "imports")
    /// - `depends_on`: module→module and subsystem→subsystem (aggregated from file-level)
    ///
    /// When function entities are provided, also creates:
    /// - `calls`: function→function (resolved from code unit `calls` when targets are functions)
    /// - `imports`: function→file (resolved from code unit `imports`)
    pub fn build(
        code_units: &[CodeUnit],
        grouping: &GroupingResult,
        project_id: &str,
        repo_name: &str,
        repo_root: &Path,
    ) -> Result<(Vec<Relationship>, UnresolvedCounts)> {
        let ctx = relationships_aggregate::BuildContext {
            code_units,
            grouping,
            project_id,
            repo_name,
            repo_root,
            function_entities: None,
        };
        Self::build_inner(&ctx)
    }

    /// Builds relationships, optionally including function-level edges.
    ///
    /// Returns the unresolved-import side-channel counter (`UnresolvedCounts`,
    /// split internal/external per the #690 amendment) alongside the edges.
    pub(crate) fn build_with_functions(
        code_units: &[CodeUnit],
        grouping: &GroupingResult,
        project_id: &str,
        repo_name: &str,
        repo_root: &Path,
        function_entities: Option<&[Entity]>,
    ) -> Result<(Vec<Relationship>, UnresolvedCounts)> {
        let ctx = relationships_aggregate::BuildContext {
            code_units,
            grouping,
            project_id,
            repo_name,
            repo_root,
            function_entities,
        };
        Self::build_inner(&ctx)
    }
}

#[path = "relationships_aggregate.rs"]
mod relationships_aggregate;

#[path = "relationships_function_edges.rs"]
mod relationships_function_edges;

#[path = "relationships_cross_repo.rs"]
mod relationships_cross_repo;

#[path = "relationships_tests.rs"]
#[cfg(test)]
mod relationships_tests;

#[path = "relationships_tests_skip.rs"]
#[cfg(test)]
mod relationships_tests_skip;

#[path = "js_source_root_resolver_tests.rs"]
#[cfg(test)]
mod js_source_root_resolver_tests;
#[path = "relationships_regression_tests.rs"]
#[cfg(test)]
mod relationships_regression_tests;
