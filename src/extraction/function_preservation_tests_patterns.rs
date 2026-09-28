use crate::extraction::function_preservation::extract_cfg_test_functions;

// Tests for issue #538: Enhanced cfg(test) pattern detection

#[test]
fn test_issue_538_cfg_test_with_intermediate_attribute() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
#[allow(dead_code)]
fn helper() {
    // Test helper with intermediate attribute
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 1);
    assert!(cfg_test_fns.contains("helper"));
}

#[test]
fn test_issue_538_cfg_test_with_restricted_visibility() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
pub(crate) fn helper() {
    // Test helper with crate-level visibility
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 1);
    assert!(cfg_test_fns.contains("helper"));
}

#[test]
fn test_issue_538_cfg_test_with_unsafe_fn() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
unsafe fn helper() {
    // Unsafe test helper
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 1);
    assert!(cfg_test_fns.contains("helper"));
}

#[test]
fn test_issue_538_cfg_test_mod_level_pattern() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
pub fn production_function() -> i32 {
    42
}

#[cfg(test)]
mod tests {
    fn helper() {
        // Test helper inside cfg(test) module
    }
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 1);
    assert!(cfg_test_fns.contains("helper"));
    assert!(!cfg_test_fns.contains("production_function"));
}

#[test]
fn test_issue_538_cfg_test_mod_multiple_functions() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
mod tests {
    fn test_helper() {
        // First test helper
    }

    fn setup() {
        // Setup function
    }

    fn teardown() {
        // Teardown function
    }
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 3);
    assert!(cfg_test_fns.contains("test_helper"));
    assert!(cfg_test_fns.contains("setup"));
    assert!(cfg_test_fns.contains("teardown"));
}

#[test]
fn test_issue_538_cfg_test_mod_direct_and_indirect_patterns() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
fn direct_helper() {
    // Directly annotated
}

#[cfg(test)]
mod tests {
    fn mod_helper() {
        // Inside cfg(test) module
    }
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 2);
    assert!(cfg_test_fns.contains("direct_helper"));
    assert!(cfg_test_fns.contains("mod_helper"));
}

#[test]
fn test_issue_538_cfg_test_all_variations_together() {
    let temp = tempfile::tempdir().unwrap();
    let file_path = temp.path().join("test.rs");

    let source = r#"
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn complex_helper() {
    // Mix of attributes
}

#[cfg(test)]
unsafe fn unsafe_helper() {
    // Unsafe function
}

#[cfg(test)]
pub async fn async_helper() {
    // Public async function
}

#[cfg(test)]
mod tests {
    fn mod_helper1() {}
    fn mod_helper2() {}
}
"#;

    std::fs::write(&file_path, source).unwrap();

    let cfg_test_fns = extract_cfg_test_functions(file_path.to_str().unwrap(), temp.path());

    assert_eq!(cfg_test_fns.len(), 5);
    assert!(cfg_test_fns.contains("complex_helper"));
    assert!(cfg_test_fns.contains("unsafe_helper"));
    assert!(cfg_test_fns.contains("async_helper"));
    assert!(cfg_test_fns.contains("mod_helper1"));
    assert!(cfg_test_fns.contains("mod_helper2"));
}
