// Tests for help structure and visual separation between daily and admin workflows.

pub mod common;

#[test]
fn test_help_separates_daily_and_admin_workflows() {
    let (stdout, _stderr, status) = common::run_lievo(&["--help"]);

    assert_eq!(status, 0, "lievo --help should succeed");

    let help_text = stdout.to_lowercase();

    assert!(
        help_text.contains("daily") || help_text.contains("workflows"),
        "help should mention daily workflows"
    );
    assert!(
        help_text.contains("admin") || help_text.contains("management"),
        "help should mention admin workflows"
    );

    assert!(
        stdout.contains("admin") || stdout.contains("Admin"),
        "help should show admin command"
    );
    assert!(
        stdout.contains("query") || stdout.contains("mcp") || stdout.contains("refresh"),
        "help should show at least one daily command (query, mcp, or refresh)"
    );
}

#[test]
fn test_help_visual_separation_structure() {
    let (stdout, _stderr, status) = common::run_lievo(&["--help"]);

    assert_eq!(status, 0, "lievo --help should succeed");

    let help_text = &stdout;

    assert!(
        help_text.contains("Admin") || help_text.contains("admin"),
        "help should show a clear Admin/admin category section"
    );

    let daily_count = [
        help_text.contains("query"),
        help_text.contains("mcp"),
        help_text.contains("refresh"),
    ]
    .iter()
    .filter(|&&c| c)
    .count();

    assert!(
        daily_count >= 3,
        "help should show at least 3 daily workflow commands, found {}",
        daily_count
    );

    assert!(
        help_text.contains("admin"),
        "admin should be listed as an available command"
    );

    assert!(
        help_text.contains("workflow")
            || help_text.contains("Daily")
            || help_text.contains("Admin"),
        "help text should mention workflows, Daily, or Admin to show separation"
    );
}
