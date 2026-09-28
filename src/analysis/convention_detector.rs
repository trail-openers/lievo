// ConventionDetector — analyzes codebase patterns to detect project conventions.
// Detects: naming style, test file patterns, module organization.

use crate::error::Result;
use crate::model::Convention;
use crate::model::Entity;
use crate::storage::Storage;
use chrono::Utc;
use std::collections::HashMap;

pub struct ConventionDetector<'a> {
    storage: &'a dyn Storage,
}

impl<'a> ConventionDetector<'a> {
    pub fn new(storage: &'a dyn Storage) -> Self {
        Self { storage }
    }

    /// Detect and persist conventions for the given project.
    pub fn detect(&self, project_id: &str) -> Result<()> {
        let entities = self.storage.list_entities(project_id, None)?;

        // Detect naming convention
        if let Some(conv) = self.detect_naming_convention(&entities, project_id) {
            self.storage.upsert_convention(&conv)?;
        }

        // Detect test file pattern
        if let Some(conv) = self.detect_test_pattern(&entities, project_id) {
            self.storage.upsert_convention(&conv)?;
        }

        // Detect module organization
        if let Some(conv) = self.detect_module_organization(&entities, project_id) {
            self.storage.upsert_convention(&conv)?;
        }

        Ok(())
    }

    /// Detect naming convention (snake_case vs camelCase).
    fn detect_naming_convention(
        &self,
        entities: &[Entity],
        project_id: &str,
    ) -> Option<Convention> {
        if entities.is_empty() {
            return None;
        }

        let mut snake_case_count = 0;
        let mut camel_case_count = 0;
        let mut total = 0;
        let mut snake_example: Option<String> = None;
        let mut camel_example: Option<String> = None;

        for entity in entities {
            if entity.tier == crate::model::EntityTier::Function
                || entity.tier == crate::model::EntityTier::Module
            {
                total += 1;
                if is_snake_case(&entity.name) {
                    snake_case_count += 1;
                    if snake_example.is_none() {
                        snake_example = Some(entity.name.clone());
                    }
                } else if is_camel_case(&entity.name) {
                    camel_case_count += 1;
                    if camel_example.is_none() {
                        camel_example = Some(entity.name.clone());
                    }
                }
            }
        }

        if total == 0 {
            return None;
        }

        let snake_ratio = snake_case_count as f64 / total as f64;
        let camel_ratio = camel_case_count as f64 / total as f64;

        let (title, description, confidence, example_code) = if snake_ratio >= 0.7 {
            (
                "snake_case naming".to_string(),
                Some(format!(
                    "Project uses snake_case for {}% of functions/modules ({}/{} total)",
                    (snake_ratio * 100.0) as i32,
                    snake_case_count,
                    total
                )),
                snake_ratio,
                snake_example,
            )
        } else if camel_ratio >= 0.7 {
            (
                "camelCase naming".to_string(),
                Some(format!(
                    "Project uses camelCase for {}% of functions/modules ({}/{} total)",
                    (camel_ratio * 100.0) as i32,
                    camel_case_count,
                    total
                )),
                camel_ratio,
                camel_example,
            )
        } else {
            return None; // No clear dominant pattern
        };

        Some(Convention {
            id: format!("conv-{}-naming", project_id),
            project_id: project_id.to_string(),
            category: "naming".to_string(),
            title,
            description,
            example_code,
            confidence,
            entity_ids_json: None,
            detected_at: Utc::now().to_rfc3339(),
            still_valid: true,
        })
    }

    /// Detect test file organization pattern.
    fn detect_test_pattern(&self, entities: &[Entity], project_id: &str) -> Option<Convention> {
        if entities.is_empty() {
            return None;
        }

        let mut in_tests_dir = 0;
        let mut in_test_dir = 0;
        let mut prefix_test = 0;
        let mut suffix_test = 0;
        let mut suffix_tests = 0;

        for entity in entities {
            if entity.tier == crate::model::EntityTier::File
                && let Some(path) = &entity.path
            {
                // Test files are classified by HIGHEST-PRIORITY pattern only (exclusive match):
                // tests/ dir > test/ dir > _test.rs > _tests.rs > test_ prefix.
                // A file matching multiple patterns counts only under its highest-priority category.
                if path.starts_with("tests/") || path.contains("/tests/") {
                    in_tests_dir += 1;
                } else if path.starts_with("test/") || path.contains("/test/") {
                    in_test_dir += 1;
                } else if path.ends_with("_test.rs") {
                    suffix_test += 1;
                } else if path.ends_with("_tests.rs") {
                    suffix_tests += 1;
                } else if (path.contains("/test_") || path.starts_with("test_"))
                    && path.ends_with(".rs")
                {
                    prefix_test += 1;
                }
            }
        }

        let total_test_files =
            in_tests_dir + in_test_dir + prefix_test + suffix_test + suffix_tests;
        if total_test_files < 2 {
            return None; // Not enough test files to establish a pattern
        }

        // Determine dominant pattern
        let mut counts = HashMap::new();
        counts.insert("tests/ directory", in_tests_dir);
        counts.insert("test/ directory", in_test_dir);
        counts.insert("test_ prefix", prefix_test);
        counts.insert("_test.rs suffix", suffix_test);
        counts.insert("_tests.rs suffix", suffix_tests);

        let dominant_pattern = counts
            .iter()
            .max_by_key(|&(_, count)| count)
            .and_then(|(name, count)| if *count > 0 { Some(*name) } else { None })?;

        // SAFETY: key was just obtained from this map
        let confidence = *counts.get(dominant_pattern).unwrap() as f64 / total_test_files as f64;

        let description = format!(
            "Project organizes tests using {} pattern ({}/{} test files)",
            dominant_pattern,
            // SAFETY: key was just obtained from this map
            counts.get(dominant_pattern).unwrap(),
            total_test_files
        );

        let example_code = match dominant_pattern {
            "tests/ directory" => Some("tests/integration_test.rs".to_string()),
            "test/ directory" => Some("test/unit_test.rs".to_string()),
            "test_ prefix" => Some("test_math.rs".to_string()),
            "_test.rs suffix" => Some("module_test.rs".to_string()),
            "_tests.rs suffix" => Some("module_tests.rs".to_string()),
            _ => None,
        };

        Some(Convention {
            id: format!("conv-{}-testing", project_id),
            project_id: project_id.to_string(),
            category: "testing".to_string(),
            title: format!("{} pattern", dominant_pattern),
            description: Some(description),
            example_code,
            confidence,
            entity_ids_json: None,
            detected_at: Utc::now().to_rfc3339(),
            still_valid: true,
        })
    }

    /// Detect module organization pattern (flat vs nested/feature-based).
    fn detect_module_organization(
        &self,
        entities: &[Entity],
        project_id: &str,
    ) -> Option<Convention> {
        if entities.is_empty() {
            return None;
        }

        let mut depths = Vec::new();

        for entity in entities {
            if entity.tier == crate::model::EntityTier::File
                && let Some(path) = &entity.path
            {
                let depth = path.matches('/').count();
                depths.push(depth);
            }
        }

        if depths.is_empty() {
            return None;
        }

        let avg_depth = depths.iter().sum::<usize>() as f64 / depths.len() as f64;
        let max_depth = *depths.iter().max().unwrap_or(&0);

        let (title, description, confidence) = if avg_depth <= 2.0 {
            let confidence = if avg_depth <= 1.5 { 0.95 } else { 0.8 };
            (
                "flat structure".to_string(),
                Some(format!(
                    "Project uses flat directory structure (avg depth: {:.1}, max: {})",
                    avg_depth, max_depth
                )),
                confidence,
            )
        } else if avg_depth >= 3.0 {
            let confidence = if avg_depth >= 4.0 { 0.9 } else { 0.75 };
            (
                "nested/feature-based structure".to_string(),
                Some(format!(
                    "Project uses nested directory structure (avg depth: {:.1}, max: {})",
                    avg_depth, max_depth
                )),
                confidence,
            )
        } else {
            // Mixed structure - no clear pattern
            return None;
        };

        let example_code = Some(if title.contains("flat") {
            "src/module1.rs\nsrc/module2.rs".to_string()
        } else {
            "src/features/auth/login.rs\nsrc/features/auth/register.rs".to_string()
        });

        Some(Convention {
            id: format!("conv-{}-structure", project_id),
            project_id: project_id.to_string(),
            category: "structure".to_string(),
            title,
            description,
            example_code,
            confidence,
            entity_ids_json: None,
            detected_at: Utc::now().to_rfc3339(),
            still_valid: true,
        })
    }
}

/// Check if a name follows snake_case convention.
fn is_snake_case(name: &str) -> bool {
    name.contains('_') && !name.chars().any(|c| c.is_uppercase())
}

/// Check if a name follows camelCase convention.
fn is_camel_case(name: &str) -> bool {
    // camelCase: starts with lowercase, contains uppercase
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() || !chars[0].is_lowercase() {
        return false;
    }
    chars.iter().any(|c| c.is_uppercase())
}

#[cfg(test)]
#[path = "convention_detector_tests/mod.rs"]
mod convention_detector_tests;
