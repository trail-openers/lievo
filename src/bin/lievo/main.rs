use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use lievo::output::{OutputFormat, format_error_envelope};
use lievo::storage::sqlite::SqliteStorage;
use tracing_subscriber::EnvFilter;

mod commands;

/// Lievo — Codebase knowledge platform
#[derive(Parser)]
#[command(name = "lievo")]
#[command(about = "Structural analysis and knowledge platform for codebases")]
#[command(
    long_about = "Daily workflows:\n  refresh  query  mcp\n\nAdmin workflows:\n  lievo admin <subcommand>\n\nRun 'lievo admin --help' for project and repository management."
)]
#[command(version)]
struct Cli {
    /// Output format: human or json (default: human)
    #[arg(long, global = true)]
    format: Option<OutputFormat>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Project and repository management commands
    #[command(visible_alias = "adm")]
    Admin {
        #[command(subcommand)]
        subcommand: AdminCommands,
    },

    /// Keep codebase analysis up to date
    Refresh {
        /// Project name to refresh (defaults to the only project if exactly one exists)
        #[arg(value_name = "PROJECT")]
        project: Option<String>,
        /// Force full refresh (ignore incremental)
        #[arg(long)]
        full: bool,
        /// Force refresh even if commit hash is unchanged (e.g. after upgrading lievo or changing config)
        #[arg(long, short = 'f')]
        force: bool,
        /// Skip on-device summarization even if apfel is available
        #[arg(long)]
        no_summarize: bool,
        /// Include gitignored files in refresh
        #[arg(long)]
        no_ignore: bool,
    },

    /// Query codebase structure, dependencies, and metrics
    #[command(visible_alias = "q")]
    Query {
        #[command(subcommand)]
        subcommand: QueryCommands,
    },

    /// Start MCP server exposing lievo tools to agents
    #[command(visible_alias = "serve")]
    Mcp {
        /// Project name to serve (optional — when omitted, resolves the repo
        /// the server is launched in: LIEVO_PROJECT_DIR > CLAUDE_PROJECT_DIR > cwd)
        #[arg(value_name = "PROJECT")]
        project: Option<String>,
    },

    /// Diagnose whether lievo will work here: repo resolution, registration,
    /// index state, background indexing, and data location (read-only)
    Doctor {
        /// Path to resolve the repository from (defaults to the current directory)
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Re-summarize entities with cached descriptions
    #[command(visible_alias = "sum")]
    Summarize {
        /// Project name to summarize (defaults to the only project if exactly one exists)
        #[arg(value_name = "PROJECT")]
        project: Option<String>,
        /// File path to re-summarize (re-summarizes only this file and its ancestors)
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
    },
}

#[derive(Subcommand)]
enum AdminCommands {
    /// Create a new project
    CreateProject {
        /// Name for the project
        name: String,
    },

    /// List all projects
    ListProjects,

    /// Delete a project and all its data
    DeleteProject {
        /// Name of the project to delete
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Register a repository and add it to a project
    AddRepo {
        /// Path to the local git repository
        path: PathBuf,
        /// Project name to add the repo to (defaults to directory name)
        #[arg(value_name = "PROJECT")]
        project: Option<String>,
    },

    /// Link an existing repository to a project
    LinkRepo {
        /// Project name
        project: String,
        /// Repository ID
        repo_id: String,
    },

    /// List repositories, optionally filtered by project
    ListRepos {
        /// Filter by project name
        #[arg(value_name = "PROJECT")]
        project: Option<String>,
    },

    /// Delete a repository and all its analysis data from a project
    DeleteRepo {
        /// Repository name to delete
        name: String,
        /// Project name the repo belongs to (required for safety)
        project: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Show analysis status for repositories
    Status {
        /// Show status for a specific project only
        project: Option<String>,
    },

    /// Show database information and statistics
    Info,

    /// Show per-language extraction coverage metrics (issue #679)
    Coverage {
        /// Show coverage for a specific project only
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        /// Measure a pinned repository path directly (CI gate target; bypasses project lookup)
        #[arg(long, value_name = "REPO_PATH")]
        repo: Option<std::path::PathBuf>,
        /// Fail (exit 1) when the fan-out or single-character callee gate is violated
        #[arg(long)]
        gate: bool,
        /// Fan-out threshold: max entities per bare function name (default 10)
        #[arg(long, default_value = "10")]
        fan_out_threshold: usize,
    },

    /// Pinned-repo quality gate: edge correctness, false-0-callers, retrieval
    /// probes, payload-bytes floor (issue #715)
    Selfcheck {
        #[command(flatten)]
        args: commands::project::SelfcheckArgs,
    },
}

#[derive(Subcommand)]
enum QueryCommands {
    /// Search entities by keyword
    Entities {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
        /// Search query
        query: String,
        /// Use semantic (embedding-based) search instead of keyword matching.
        /// Requires a vector index built by `lievo refresh`; falls back to
        /// keyword matching with a warning if no index is available.
        #[arg(long)]
        semantic: bool,
    },

    /// Show full entity details
    Entity {
        /// Filter by project name (optional — entity IDs are project-scoped)
        #[arg(long)]
        project: Option<String>,
        /// Entity ID
        entity_id: String,
    },

    /// Show dependencies and dependents for an entity
    Relationships {
        /// Filter by project name (optional — entity IDs are project-scoped)
        #[arg(long)]
        project: Option<String>,
        /// Entity ID
        entity_id: String,
    },

    /// List child entities for a module or subsystem
    Children {
        /// Filter by project name (optional — entity IDs are project-scoped)
        #[arg(long)]
        project: Option<String>,
        /// Parent entity ID
        entity_id: String,
    },

    /// Show execution flows
    Flows {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
    },

    /// List discovered documentation files
    Docs {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
    },

    /// List subsystems with metrics
    Subsystems {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
    },

    /// List modules in a subsystem
    Modules {
        /// Subsystem entity ID
        subsystem_id: String,
    },

    /// List files in a module
    Files {
        /// Module entity ID
        module_id: String,
    },

    /// Show what an entity depends on
    Deps {
        /// Entity ID
        entity_id: String,
    },

    /// Show what depends on an entity
    Dependents {
        /// Entity ID
        entity_id: String,
    },

    /// Impact analysis for changed files (repo-relative paths)
    Impact {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
        /// One or more repo-relative file paths
        #[arg(required = true)]
        files: Vec<String>,
    },

    /// Show complexity/coupling hotspots
    Hotspots {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
        /// Number of hotspots to show (default 10)
        #[arg(long, default_value = "10")]
        limit: usize,
    },

    /// List detected coding conventions
    Conventions {
        /// Filter by project name (defaults to the only project if exactly one exists)
        #[arg(long)]
        project: Option<String>,
        /// Filter by category: naming, error_handling, testing, architecture, documentation
        #[arg(long)]
        category: Option<String>,
    },
}

fn main() {
    // Initialize tracing with env-filter, defaulting to lievo=warn
    // Users can override with RUST_LOG environment variable
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("lievo=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();

    let cli = Cli::parse();

    // `doctor` opens the database itself (issue #875 D1): it must still run
    // and diagnose when the database cannot be opened, so dispatch it before
    // the global storage open. Every other command opens storage exactly as
    // before.
    if let Commands::Doctor { path } = &cli.command {
        let path_str = path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let fmt = cli.format.unwrap_or_default();
        match commands::doctor::doctor(path_str.as_deref(), fmt) {
            Ok(code) => process::exit(code),
            Err(e) => {
                eprintln!("{}", format_error_envelope(&e));
                process::exit(1);
            }
        }
    }

    let storage = match SqliteStorage::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", format_error_envelope(&e));
            process::exit(1);
        }
    };

    // Default format: human (OutputFormat::default); scripts/agents opt into JSON with --format json
    let fmt = cli.format.unwrap_or_default();

    let result = match cli.command {
        Commands::Admin { subcommand } => execute_admin(&storage, subcommand, fmt),
        Commands::Refresh {
            project,
            full,
            force,
            no_summarize,
            no_ignore,
        } => commands::refresh::refresh(
            &storage,
            project.as_deref(),
            full,
            force,
            no_summarize,
            no_ignore,
            fmt,
        ),
        Commands::Mcp { project } => commands::serve::serve(project.as_deref()),
        // `doctor` is dispatched before the storage open (see above); this
        // arm is unreachable.
        Commands::Doctor { .. } => unreachable!("doctor is dispatched before the storage open"),
        Commands::Query { subcommand } => execute_query(&storage, subcommand, fmt),
        Commands::Summarize { project, file } => {
            commands::summarize::summarize(&storage, project.as_deref(), file.as_deref(), fmt)
        }
    };

    if let Err(e) = result {
        // All errors go to stderr as structured JSON for machine-readability
        eprintln!("{}", format_error_envelope(&e));
        process::exit(1);
    }
}

/// Returns the platform-appropriate default database path (~/.lievo/lievo.db).
fn default_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".lievo")
        .join("lievo.db")
}

/// Execute admin subcommands
fn execute_admin(
    storage: &SqliteStorage,
    command: AdminCommands,
    fmt: OutputFormat,
) -> Result<(), lievo::LievoError> {
    match command {
        AdminCommands::CreateProject { name } => commands::project::create_project(storage, &name),
        AdminCommands::ListProjects => commands::project::list_projects(storage),
        AdminCommands::DeleteProject { name, force } => {
            commands::project::delete_project(storage, &name, force)
        }
        AdminCommands::DeleteRepo {
            name,
            project,
            force,
        } => commands::project::delete_repo(storage, &name, &project, force),
        AdminCommands::AddRepo { path, project } => {
            commands::project::add_repo(storage, &path, project.as_deref())
        }
        AdminCommands::LinkRepo { project, repo_id } => {
            commands::project::link_repo(storage, &project, &repo_id, fmt)
        }
        AdminCommands::ListRepos { project } => {
            commands::project::list_repos(storage, project.as_deref())
        }
        AdminCommands::Status { project } => {
            commands::project::status(storage, project.as_deref(), fmt)
        }
        AdminCommands::Info => {
            let db_path = default_db_path();
            commands::project::info(storage, &db_path, fmt)
        }
        AdminCommands::Coverage {
            project,
            repo,
            gate,
            fan_out_threshold,
        } => commands::project::coverage(
            storage,
            project.as_deref(),
            repo.as_deref(),
            gate,
            fan_out_threshold,
            fmt,
        ),
        AdminCommands::Selfcheck { args } => commands::project::selfcheck(storage, args, fmt),
    }
}

/// Execute query subcommands
fn execute_query(
    storage: &SqliteStorage,
    subcommand: QueryCommands,
    fmt: OutputFormat,
) -> Result<(), lievo::LievoError> {
    match subcommand {
        QueryCommands::Entities {
            project,
            query,
            semantic,
        } => commands::query_entity::entities(storage, project.as_deref(), &query, semantic, fmt),
        QueryCommands::Entity {
            project: _,
            entity_id,
        } => commands::query_entity::entity(storage, &entity_id, fmt),
        QueryCommands::Relationships {
            project: _,
            entity_id,
        } => commands::query_entity::relationships(storage, &entity_id, fmt),
        QueryCommands::Children {
            project: _,
            entity_id,
        } => commands::query_entity::children(storage, &entity_id, fmt),
        QueryCommands::Flows { project } => {
            commands::query_discovery::flows(storage, project.as_deref(), fmt)
        }
        QueryCommands::Docs { project } => {
            commands::query_discovery::docs(storage, project.as_deref(), fmt)
        }
        QueryCommands::Subsystems { project } => {
            commands::query::subsystems(storage, project.as_deref(), fmt)
        }
        QueryCommands::Modules { subsystem_id } => {
            commands::query::modules(storage, &subsystem_id, fmt)
        }
        QueryCommands::Files { module_id } => commands::query::files(storage, &module_id, fmt),
        QueryCommands::Deps { entity_id } => commands::query::deps(storage, &entity_id, fmt),
        QueryCommands::Dependents { entity_id } => {
            commands::query::dependents(storage, &entity_id, fmt)
        }
        QueryCommands::Impact { project, files } => {
            commands::query::impact(storage, project.as_deref(), &files, fmt)
        }
        QueryCommands::Hotspots { project, limit } => {
            commands::query::hotspots(storage, project.as_deref(), limit, fmt)
        }
        QueryCommands::Conventions { project, category } => {
            commands::query::conventions(storage, project.as_deref(), category.as_deref(), fmt)
        }
    }
}
