// Tests for admin command grouping.
// Verifies that admin commands are only available under 'lievo admin <subcommand>'
// and that legacy top-level aliases have been removed.
//
// NOTE: Clap outputs help messages to stdout, not stderr. All help assertions
// check stdout. Error messages go to stderr.

pub mod common;

// ---------------------------------------------------------------------------
// Admin command group tests
// ---------------------------------------------------------------------------

#[test]
fn test_admin_help_shows_management_subcommands() {
    let (stdout, _stderr, status) = common::run_lievo(&["admin", "--help"]);

    assert_eq!(status, 0, "lievo admin --help should succeed");
    assert!(
        stdout.contains("create-project") || stdout.contains("CreateProject"),
        "admin help should show create-project command"
    );
    assert!(
        stdout.contains("list-projects") || stdout.contains("ListProjects"),
        "admin help should show list-projects command"
    );
    assert!(
        stdout.contains("delete-project") || stdout.contains("DeleteProject"),
        "admin help should show delete-project command"
    );
    assert!(
        stdout.contains("add-repo") || stdout.contains("AddRepo"),
        "admin help should show add-repo command"
    );
    assert!(
        stdout.contains("list-repos") || stdout.contains("ListRepos"),
        "admin help should show list-repos command"
    );
    assert!(
        stdout.contains("delete-repo") || stdout.contains("DeleteRepo"),
        "admin help should show delete-repo command"
    );
    assert!(
        stdout.contains("link-repo") || stdout.contains("LinkRepo"),
        "admin help should show link-repo command"
    );
    assert!(
        stdout.contains("info") || stdout.contains("Info"),
        "admin help should show info command"
    );
    assert!(
        stdout.contains("status") || stdout.contains("Status"),
        "admin help should show status command"
    );
    assert!(
        stdout.contains("coverage"),
        "admin help should list the coverage subcommand"
    );
}

#[test]
fn test_admin_coverage_help_lists_options() {
    let (stdout, _stderr, status) = common::run_lievo(&["admin", "coverage", "--help"]);

    assert_eq!(status, 0, "lievo admin coverage --help should succeed");
    assert!(
        stdout.contains("project") || stdout.contains("Project"),
        "coverage help should show the PROJECT option, got: {stdout}"
    );
    assert!(
        stdout.contains("--gate"),
        "coverage help should show the --gate flag, got: {stdout}"
    );
    assert!(
        stdout.contains("--fan-out-threshold"),
        "coverage help should show --fan-out-threshold, got: {stdout}"
    );
}

#[test]
fn test_admin_alias_works() {
    let (stdout, _stderr, status) = common::run_lievo(&["admin", "--help"]);

    assert_eq!(status, 0, "lievo admin --help should succeed");
    assert!(!stdout.is_empty(), "admin help should produce output");
}

#[test]
fn test_admin_status_subcommand() {
    let (stdout, _stderr, status) = common::run_lievo(&["admin", "status", "--help"]);

    assert_eq!(status, 0, "lievo admin status --help should succeed");
    assert!(
        stdout.contains("project") || stdout.contains("Project"),
        "admin status help should mention project"
    );
}

// ---------------------------------------------------------------------------
// Main help shows daily commands and admin group
// ---------------------------------------------------------------------------

#[test]
fn test_main_help_separates_daily_and_admin_workflows() {
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
}

#[test]
fn test_main_help_shows_admin_and_daily_sections() {
    let (stdout, _stderr, status) = common::run_lievo(&["--help"]);

    assert_eq!(status, 0, "lievo --help should succeed");

    assert!(
        stdout.contains("admin") || stdout.contains("Admin"),
        "main help should show admin command"
    );
    assert!(
        stdout.contains("query") || stdout.contains("Query"),
        "main help should show query command"
    );
    assert!(
        stdout.contains("mcp") || stdout.contains("Mcp"),
        "main help should show mcp command"
    );
}

#[test]
fn test_main_help_shows_daily_workflow_commands() {
    let (stdout, _stderr, status) = common::run_lievo(&["--help"]);

    assert_eq!(status, 0, "lievo --help should succeed");

    let daily_commands = ["query", "mcp", "refresh"];

    for cmd_name in daily_commands {
        assert!(
            stdout.contains(cmd_name),
            "main help should show {} command",
            cmd_name
        );
    }
}

// ---------------------------------------------------------------------------
// Legacy aliases are removed
// ---------------------------------------------------------------------------

#[test]
fn test_legacy_aliases_not_available_at_top_level() {
    let legacy_commands = [
        "create-project",
        "list-projects",
        "delete-project",
        "add-repo",
        "link-repo",
        "list-repos",
        "delete-repo",
        "info",
        "status",
    ];

    for cmd in legacy_commands {
        let (_stdout, _stderr, status) = common::run_lievo(&[cmd, "--help"]);
        assert_ne!(
            status, 0,
            "legacy command '{}' should not be available at top level",
            cmd
        );
    }
}
