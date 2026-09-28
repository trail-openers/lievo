// Tests for command aliases (q, serve).

mod common;

#[test]
fn test_query_works_with_short_alias() {
    let (stdout, _stderr, status) = common::run_lievo(&["q", "--help"]);

    assert_eq!(status, 0, "lievo q short alias should work");
    assert!(
        stdout.contains("entities") || stdout.contains("Entities"),
        "query help should show entities subcommand"
    );
}

#[test]
fn test_mcp_works_with_serve_alias() {
    let (stdout, _stderr, status) = common::run_lievo(&["serve", "--help"]);

    assert_eq!(status, 0, "lievo serve alias should work");
    assert!(
        stdout.contains("project") || stdout.contains("Project"),
        "mcp/serve help should mention project"
    );
}
