// Static framework lookup table and dependency-matching helper.
// These data-only helpers are separated from the framework detection logic
// to keep `framework.rs` under the 500-line file limit.

use super::framework::FrameworkProfile;

/// Build the static lookup table: `(ecosystem, dep_substring)` → `FrameworkProfile`.
///
/// Uses `contains` matching on lowercased dep names, so "django==4.2" matches "django".
/// Returns the first match from the table.
pub(crate) fn framework_lookup_table() -> Vec<(&'static str, &'static str, FrameworkProfile)> {
    vec![
        (
            "ruby",
            "rails",
            FrameworkProfile {
                name: "Rails".to_string(),
                module_depth: 2,
                key_files: vec![
                    "config/routes.rb".to_string(),
                    "db/schema.rb".to_string(),
                    "Gemfile".to_string(),
                    "config/application.rb".to_string(),
                ],
                subsystems: vec![
                    ("app".to_string(), "app".to_string()),
                    ("spec".to_string(), "testing".to_string()),
                    ("test".to_string(), "testing".to_string()),
                    (".".to_string(), "infrastructure".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "python",
            "django",
            FrameworkProfile {
                name: "Django".to_string(),
                module_depth: 2,
                key_files: vec!["manage.py".to_string(), "requirements.txt".to_string()],
                subsystems: vec![],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: true,
            },
        ),
        (
            "python",
            "fastapi",
            FrameworkProfile {
                name: "FastAPI".to_string(),
                module_depth: 2,
                key_files: vec!["requirements.txt".to_string()],
                subsystems: vec![
                    ("routers".to_string(), "routing".to_string()),
                    ("services".to_string(), "services".to_string()),
                    ("models".to_string(), "models".to_string()),
                    ("schemas".to_string(), "schemas".to_string()),
                    ("api".to_string(), "api".to_string()),
                    ("db".to_string(), "db".to_string()),
                    ("core".to_string(), "core".to_string()),
                    ("tests".to_string(), "testing".to_string()),
                    ("test".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "python",
            "flask",
            FrameworkProfile {
                name: "Flask".to_string(),
                module_depth: 1,
                key_files: vec!["requirements.txt".to_string()],
                subsystems: vec![
                    ("blueprints".to_string(), "blueprints".to_string()),
                    ("models".to_string(), "models".to_string()),
                    ("templates".to_string(), "templates".to_string()),
                    ("static".to_string(), "static".to_string()),
                    ("services".to_string(), "services".to_string()),
                    ("tests".to_string(), "testing".to_string()),
                    ("test".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "javascript",
            "next",
            FrameworkProfile {
                name: "Next.js".to_string(),
                module_depth: 2,
                key_files: vec![
                    "package.json".to_string(),
                    "next.config.js".to_string(),
                    "next.config.mjs".to_string(),
                ],
                subsystems: vec![],
                doc_template: "frontend_app".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "javascript",
            "@nestjs/core",
            FrameworkProfile {
                name: "NestJS".to_string(),
                module_depth: 3,
                key_files: vec!["package.json".to_string(), "nest-cli.json".to_string()],
                subsystems: vec![
                    ("src".to_string(), "app".to_string()),
                    ("test".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "javascript",
            "nuxt",
            FrameworkProfile {
                name: "Nuxt.js".to_string(),
                module_depth: 2,
                key_files: vec!["package.json".to_string(), "nuxt.config.ts".to_string()],
                subsystems: vec![],
                doc_template: "frontend_app".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "php",
            "laravel/framework",
            FrameworkProfile {
                name: "Laravel".to_string(),
                module_depth: 2,
                key_files: vec![
                    "composer.json".to_string(),
                    "routes/web.php".to_string(),
                    "routes/api.php".to_string(),
                ],
                subsystems: vec![
                    ("app".to_string(), "app".to_string()),
                    ("routes".to_string(), "routing".to_string()),
                    ("database".to_string(), "database".to_string()),
                    ("tests".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "php",
            "symfony/framework-bundle",
            FrameworkProfile {
                name: "Symfony".to_string(),
                module_depth: 2,
                key_files: vec![
                    "composer.json".to_string(),
                    "config/services.yaml".to_string(),
                ],
                subsystems: vec![
                    ("src".to_string(), "app".to_string()),
                    ("config".to_string(), "config".to_string()),
                    ("tests".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "java",
            "spring-boot",
            FrameworkProfile {
                name: "Spring Boot".to_string(),
                module_depth: 2,
                key_files: vec![
                    "application.properties".to_string(),
                    "application.yml".to_string(),
                ],
                subsystems: vec![
                    ("src/main/java".to_string(), "app".to_string()),
                    ("src/test".to_string(), "testing".to_string()),
                ],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "go",
            "github.com/gin-gonic/gin",
            FrameworkProfile {
                name: "Gin".to_string(),
                module_depth: 1,
                key_files: vec!["go.mod".to_string(), "main.go".to_string()],
                subsystems: vec![],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: true,
            },
        ),
        (
            "go",
            "github.com/labstack/echo",
            FrameworkProfile {
                name: "Echo".to_string(),
                module_depth: 1,
                key_files: vec!["go.mod".to_string()],
                subsystems: vec![],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: true,
            },
        ),
        (
            "elixir",
            "phoenix",
            FrameworkProfile {
                name: "Phoenix".to_string(),
                module_depth: 2,
                key_files: vec!["mix.exs".to_string(), "config/config.exs".to_string()],
                subsystems: vec![],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
        (
            "rust",
            "actix-web",
            FrameworkProfile {
                name: "Actix Web".to_string(),
                module_depth: 1,
                key_files: vec!["Cargo.toml".to_string()],
                subsystems: vec![],
                doc_template: "mvc_backend".to_string(),
                has_dynamic_subsystem_detection: false,
            },
        ),
    ]
}

/// Returns `true` when `dep` matches `needle` for the given ecosystem.
///
/// Ecosystems that use exact package names (JS/TS, PHP, Elixir, Rust) require
/// an exact lowercase match. Ecosystems where dep entries appear with version
/// suffixes or full module paths (Ruby, Python, Go, Java) use substring matching
/// so that e.g. "django==4.2" matches "django" and "spring-boot-starter-web"
/// matches "spring-boot".
pub(crate) fn dep_matches(ecosystem: &str, dep: &str, needle: &str) -> bool {
    let dep_lower = dep.to_lowercase();
    match ecosystem {
        "javascript" | "php" | "elixir" | "rust" => dep_lower == needle,
        _ => dep_lower.contains(needle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dep_matches_exact_ecosystems() {
        assert!(dep_matches("javascript", "@nestjs/core", "@nestjs/core"));
        assert!(!dep_matches(
            "javascript",
            "@nestjs/core-extra",
            "@nestjs/core"
        ));
        assert!(dep_matches("php", "laravel/framework", "laravel/framework"));
        assert!(!dep_matches(
            "php",
            "laravel/framework-extra",
            "laravel/framework"
        ));
        assert!(dep_matches("elixir", "phoenix", "phoenix"));
        assert!(!dep_matches("elixir", "phoenix_live_view", "phoenix"));
        assert!(dep_matches("rust", "actix-web", "actix-web"));
        assert!(!dep_matches("rust", "actix-web-extra", "actix-web"));
    }

    #[test]
    fn test_dep_matches_substring_ecosystems() {
        assert!(dep_matches("python", "django==4.2", "django"));
        assert!(dep_matches("python", "Django==4.2", "django"));
        assert!(dep_matches("ruby", "rails", "rails"));
        assert!(dep_matches(
            "go",
            "github.com/gin-gonic/gin",
            "github.com/gin-gonic/gin"
        ));
        assert!(dep_matches(
            "java",
            "spring-boot-starter-web",
            "spring-boot"
        ));
    }

    #[test]
    fn test_framework_lookup_table_has_entries() {
        let table = framework_lookup_table();
        assert!(!table.is_empty());
        // Spot-check a few entries
        assert!(
            table
                .iter()
                .any(|(eco, dep, _)| *eco == "ruby" && *dep == "rails")
        );
        assert!(
            table
                .iter()
                .any(|(eco, dep, _)| *eco == "python" && *dep == "django")
        );
        assert!(
            table
                .iter()
                .any(|(eco, dep, _)| *eco == "go" && *dep == "github.com/gin-gonic/gin")
        );
    }

    #[test]
    fn test_fastapi_profile_has_subsystems() {
        let table = framework_lookup_table();
        let fastapi_profile = table
            .iter()
            .find(|(eco, dep, _)| *eco == "python" && *dep == "fastapi")
            .map(|(_, _, profile)| profile);

        assert!(
            fastapi_profile.is_some(),
            "FastAPI profile should exist in lookup table"
        );

        let profile = fastapi_profile.unwrap();
        assert!(
            !profile.subsystems.is_empty(),
            "FastAPI profile should have non-empty subsystems list"
        );

        // Verify expected subsystems are present
        let subsystem_paths: Vec<&str> = profile
            .subsystems
            .iter()
            .map(|(path, _)| path.as_str())
            .collect();
        assert!(subsystem_paths.contains(&"routers"));
        assert!(subsystem_paths.contains(&"services"));
        assert!(subsystem_paths.contains(&"models"));
        assert!(subsystem_paths.contains(&"schemas"));
        assert!(subsystem_paths.contains(&"api"));
        assert!(subsystem_paths.contains(&"db"));
        assert!(subsystem_paths.contains(&"core"));
        assert!(subsystem_paths.contains(&"tests"));
        assert!(subsystem_paths.contains(&"test"));
    }

    #[test]
    fn test_flask_profile_has_subsystems() {
        let table = framework_lookup_table();
        let flask_profile = table
            .iter()
            .find(|(eco, dep, _)| *eco == "python" && *dep == "flask")
            .map(|(_, _, profile)| profile);

        assert!(
            flask_profile.is_some(),
            "Flask profile should exist in lookup table"
        );

        let profile = flask_profile.unwrap();
        assert!(
            !profile.subsystems.is_empty(),
            "Flask profile should have non-empty subsystems list"
        );

        // Verify expected subsystems are present
        let subsystem_paths: Vec<&str> = profile
            .subsystems
            .iter()
            .map(|(path, _)| path.as_str())
            .collect();
        assert!(subsystem_paths.contains(&"blueprints"));
        assert!(subsystem_paths.contains(&"models"));
        assert!(subsystem_paths.contains(&"templates"));
        assert!(subsystem_paths.contains(&"static"));
        assert!(subsystem_paths.contains(&"services"));
        assert!(subsystem_paths.contains(&"tests"));
        assert!(subsystem_paths.contains(&"test"));
    }
}
