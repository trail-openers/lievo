// Tests for ReadProjectDocTool in tools_doc.rs

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ReadProjectDocTool;

use super::MAX_DOC_SIZE;
use std::fs;
use std::io::Write;

#[test]
fn test_read_project_doc_rejects_files_that_grew_too_large() {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!(
        "lievo-read-project-doc-{}-{}.md",
        std::process::id(),
        nanos
    ));

    fs::write(&path, "small doc").unwrap();

    let docs = Arc::new(vec![(path.to_string_lossy().to_string(), 1)]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let oversized = "a".repeat((MAX_DOC_SIZE as usize) + 1);
    fs::write(&path, oversized).unwrap();

    let result = tool.call(serde_json::json!({"path": path.to_string_lossy()}));
    let err = result.unwrap_err().to_string();
    assert!(err.contains("is too large"));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_reports_canonicalization_failure() {
    let path = std::env::temp_dir().join("lievo-read-project-doc-missing.md");
    let docs = Arc::new(vec![(path.to_string_lossy().to_string(), 1)]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let result = tool.call(serde_json::json!({"path": path.to_string_lossy()}));
    let err = result.unwrap_err().to_string();
    // When path does not exist, canonicalization fails with a path validation error
    assert!(err.contains("path validation error"));
}

#[test]
fn test_read_project_doc_format_raw_returns_verbatim_content() {
    let mut path = std::env::temp_dir();
    path.push("lievo-read-project-doc-raw-test.md");

    let content = "# Title\n\nThis is markdown.\n| Column1 | Column2 |\n|---------|---------|\n| Value1  | Value2  |";

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();

    let docs = Arc::new(vec![(
        path.to_string_lossy().to_string(),
        content.len() as u64,
    )]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let result = tool
        .call(serde_json::json!({"path": path.to_string_lossy(), "format": "raw"}))
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(json["file"], path.to_string_lossy().to_string());
    assert!(json.get("content").is_some());
    assert_eq!(json["content"], content);

    // Markdown table should appear in raw output
    assert!(json["content"].as_str().unwrap().contains("| Column1 |"));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_format_structured_returns_extracted_content() {
    let mut path = std::env::temp_dir();
    path.push("lievo-read-project-doc-structured-test.md");

    let content =
        "# Main Heading\n\nThis is a design decision.\n\nSubsystems are major components.";

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();

    let docs = Arc::new(vec![(
        path.to_string_lossy().to_string(),
        content.len() as u64,
    )]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let result = tool
        .call(serde_json::json!({"path": path.to_string_lossy(), "format": "structured"}))
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(json["file"], path.to_string_lossy().to_string());
    assert!(json.get("headings").is_some());
    assert!(json.get("decisions").is_some());
    assert!(json.get("terminology").is_some());
    assert!(json.get("config_values").is_some());

    let headings = json["headings"].as_array().unwrap();
    assert!(!headings.is_empty());
    assert!(headings.iter().any(|t| t == "Main Heading"));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_default_format_returns_raw() {
    let mut path = std::env::temp_dir();
    path.push("lievo-read-project-doc-default-test.md");

    let content = "# Default test\n\nContent here.";

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();

    let docs = Arc::new(vec![(
        path.to_string_lossy().to_string(),
        content.len() as u64,
    )]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    // No format parameter - should default to raw
    let result = tool
        .call(serde_json::json!({"path": path.to_string_lossy()}))
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(json["file"], path.to_string_lossy().to_string());
    assert!(json.get("content").is_some());
    assert_eq!(json["content"], content);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_invalid_format_returns_error() {
    let mut path = std::env::temp_dir();
    path.push("lievo-read-project-doc-invalid-format-test.md");

    let content = "# Test\n\nContent.";

    let mut file = fs::File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();

    let docs = Arc::new(vec![(
        path.to_string_lossy().to_string(),
        content.len() as u64,
    )]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let result =
        tool.call(serde_json::json!({"path": path.to_string_lossy(), "format": "invalid"}));
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid format"));
    assert!(err.contains("invalid"));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_raw_mode_enforces_size_limit() {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!(
        "lievo-raw-size-test-{}-{}.md",
        std::process::id(),
        nanos
    ));

    fs::write(&path, "small doc").unwrap();

    let docs = Arc::new(vec![(path.to_string_lossy().to_string(), 1)]);
    let project_root = path
        .parent()
        .expect("temp file path should have a parent directory")
        .to_path_buf();
    let tool = ReadProjectDocTool { docs, project_root };

    let oversized = "a".repeat((MAX_DOC_SIZE as usize) + 1);
    fs::write(&path, oversized).unwrap();

    // raw mode should also reject oversized files
    let result = tool.call(serde_json::json!({"path": path.to_string_lossy(), "format": "raw"}));
    let err = result.unwrap_err().to_string();
    assert!(err.contains("is too large"));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_read_project_doc_rejects_path_traversal() {
    let temp_dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    // Create two sibling subdirectories: one as project_root, one for an outside file
    let project_root = temp_dir.join(format!("lievo_test_project_root_{}", nanos));
    let outside_dir = temp_dir.join(format!("lievo_test_outside_{}", nanos));

    fs::create_dir_all(&project_root).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    let outside_path = outside_dir.join("outside.md");
    fs::write(&outside_path, "test content").unwrap();
    let path_str = outside_path.to_string_lossy().to_string();

    let docs = Arc::new(vec![(path_str.clone(), 1)]);
    let tool = ReadProjectDocTool {
        docs,
        project_root: project_root.clone(),
    };

    let result = tool.call(serde_json::json!({"path": path_str}));
    let err = result.unwrap_err().to_string();
    // Canonicalization succeeds, but path is not within project root
    assert!(
        err.contains("path is outside the project root"),
        "expected error about path outside root, got: {}",
        err
    );

    // Cleanup both temp directories (uniquely named, so swallowing errors is acceptable)
    let _ = fs::remove_dir_all(&project_root);
    let _ = fs::remove_dir_all(&outside_dir);
}
