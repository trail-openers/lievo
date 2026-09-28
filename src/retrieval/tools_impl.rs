// Retrieval tool implementations — see submodules tools_search, tools_entity, tools_relationship, tools_doc, tools_function.
//
// Tool impls are split across:
//   tools_search      — SearchEntitiesTool
//   tools_entity      — GetEntityTool, ListSubsystemsTool
//   tools_relationship — ListRelationshipsTool
//   tools_doc         — GetConventionsTool, GetInsightsTool, GetExecutionFlowsTool, ListProjectDocsTool, ReadProjectDocTool
//   tools_function    — GetFunctionTool
//   tools_query       — GetImpactTool, GetHotspotsTool

pub mod tools_directory;
pub mod tools_doc;
pub mod tools_entity;
pub mod tools_function;
pub mod tools_query;
pub mod tools_relationship;
pub mod tools_search;

#[cfg(test)]
#[path = "tools_tests_helpers.rs"]
mod tools_tests_helpers;

#[cfg(test)]
#[path = "tools_tests_search.rs"]
mod tests_search;

#[cfg(test)]
#[path = "tools_tests_get_entity_basic.rs"]
mod tests_get_entity_basic;

#[cfg(test)]
#[path = "tools_tests_get_entity_module.rs"]
mod tests_get_entity_module;

#[cfg(test)]
#[path = "tools_tests_integration.rs"]
mod tests_integration;

#[cfg(test)]
#[path = "tools_read_file_tests.rs"]
mod read_file_tests;

#[cfg(test)]
#[path = "tools_list_dir_tests.rs"]
mod list_dir_tests;
