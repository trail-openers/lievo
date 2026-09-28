use lievo::extraction::framework::read_manifest;
use std::fs;
use tempfile::TempDir;

// ── Gemfile ───────────────────────────────────────────────────────────────

#[test]
fn test_gemfile_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("Gemfile"),
        "source 'https://rubygems.org'\ngem 'rails', '~> 7.0'\ngem \"pg\"\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "ruby");
    assert!(deps.contains(&"rails".to_string()));
    assert!(deps.contains(&"pg".to_string()));
}

#[test]
fn test_gemfile_missing_returns_none() {
    let tmp = TempDir::new().unwrap();
    let result = read_manifest(tmp.path()).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_gemfile_empty_content_returns_empty_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Gemfile"), "# just a comment\n").unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "ruby");
    assert!(deps.is_empty());
}

#[test]
fn test_gemfile_binary_returns_empty_not_error() {
    let tmp = TempDir::new().unwrap();
    // Write bytes that are not valid UTF-8
    fs::write(tmp.path().join("Gemfile"), b"\xff\xfe binary \x00 data").unwrap();
    // Should return None (file treated as unreadable) — not an error
    let result = read_manifest(tmp.path());
    assert!(result.is_ok());
    // Binary file treated as if manifest missing
    let _ = result.unwrap(); // either None or empty deps — no panic/err
}

#[test]
fn test_gemfile_large_file_returns_empty_gracefully() {
    let tmp = TempDir::new().unwrap();
    // Write a file larger than 4 MB
    let large = "gem 'x'\n".repeat(600_000); // ~4.8 MB
    fs::write(tmp.path().join("Gemfile"), large).unwrap();
    let result = read_manifest(tmp.path()).unwrap();
    // Large file: treated as if manifest absent
    assert!(result.is_none());
}

// ── requirements.txt ─────────────────────────────────────────────────────

#[test]
fn test_requirements_txt_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("requirements.txt"),
        "requests==2.28.0\nflask>=2.0\nnumpy~=1.24\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "python");
    assert!(deps.contains(&"requests".to_string()));
    assert!(deps.contains(&"flask".to_string()));
    assert!(deps.contains(&"numpy".to_string()));
}

#[test]
fn test_requirements_txt_missing() {
    let tmp = TempDir::new().unwrap();
    assert!(read_manifest(tmp.path()).unwrap().is_none());
}

// ── pyproject.toml ────────────────────────────────────────────────────────

#[test]
fn test_pyproject_toml_pep508_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pyproject.toml"),
        "[project]\ndependencies = [\"requests>=2.0\", \"flask\"]\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "python");
    assert!(deps.contains(&"requests".to_string()));
    assert!(deps.contains(&"flask".to_string()));
}

#[test]
fn test_pyproject_toml_poetry_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pyproject.toml"),
        "[tool.poetry.dependencies]\npython = \"^3.11\"\nfastapi = \"*\"\nhttpx = \"*\"\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "python");
    assert!(deps.contains(&"fastapi".to_string()));
    assert!(deps.contains(&"httpx".to_string()));
    assert!(!deps.contains(&"python".to_string()));
}

#[test]
fn test_pyproject_toml_malformed_returns_empty() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("pyproject.toml"), "[[[[invalid toml").unwrap();
    // Malformed pyproject.toml returns empty deps, so requirements.txt is
    // consulted. With no requirements.txt either, result is None.
    let result = read_manifest(tmp.path()).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_pyproject_toml_empty_deps_falls_through_to_requirements_txt() {
    let tmp = TempDir::new().unwrap();
    // pyproject.toml with no dependency sections — build-only project
    fs::write(
        tmp.path().join("pyproject.toml"),
        "[build-system]\nrequires = [\"setuptools\"]\n",
    )
    .unwrap();
    fs::write(tmp.path().join("requirements.txt"), "django==4.2\n").unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "python");
    // requirements.txt must have been consulted since pyproject had no deps
    assert!(deps.contains(&"django".to_string()));
}

// ── package.json ──────────────────────────────────────────────────────────

#[test]
fn test_package_json_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("package.json"),
        r#"{"dependencies":{"react":"^18.0"},"devDependencies":{"typescript":"^5.0"}}"#,
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "javascript");
    assert!(deps.contains(&"react".to_string()));
    assert!(deps.contains(&"typescript".to_string()));
}

#[test]
fn test_package_json_malformed_returns_empty() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("package.json"), "{bad json").unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "javascript");
    assert!(deps.is_empty());
}

// ── composer.json ─────────────────────────────────────────────────────────

#[test]
fn test_composer_json_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("composer.json"),
        r#"{"require":{"laravel/framework":"^10.0","php":">=8.1"}}"#,
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "php");
    assert!(deps.contains(&"laravel/framework".to_string()));
    assert!(deps.contains(&"php".to_string()));
}

#[test]
fn test_composer_json_missing() {
    let tmp = TempDir::new().unwrap();
    assert!(read_manifest(tmp.path()).unwrap().is_none());
}

// ── go.mod ────────────────────────────────────────────────────────────────

#[test]
fn test_go_mod_file_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("go.mod"),
        "module example.com/app\n\nrequire (\n    github.com/gorilla/mux v1.8.0\n)\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "go");
    assert!(deps.contains(&"github.com/gorilla/mux".to_string()));
}

#[test]
fn test_go_mod_require_no_space_before_paren() {
    let tmp = TempDir::new().unwrap();
    // `require(` with no space — must still be detected
    fs::write(
        tmp.path().join("go.mod"),
        "module example.com/app\n\nrequire(\n    github.com/pkg/errors v0.9.1\n)\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "go");
    assert!(deps.contains(&"github.com/pkg/errors".to_string()));
}

// ── pom.xml ───────────────────────────────────────────────────────────────

#[test]
fn test_pom_xml_extracts_artifact_ids_only_in_dependency_blocks() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pom.xml"),
        "<project>\
         <artifactId>myapp</artifactId>\
         <dependencies>\
           <dependency><artifactId>spring-core</artifactId></dependency>\
           <dependency><artifactId>junit</artifactId></dependency>\
         </dependencies>\
         </project>",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "java");
    // Only deps inside <dependency> blocks
    assert!(deps.contains(&"spring-core".to_string()));
    assert!(deps.contains(&"junit".to_string()));
    // Project's own artifactId must NOT be included
    assert!(!deps.contains(&"myapp".to_string()));
}

// ── build.gradle ──────────────────────────────────────────────────────────

#[test]
fn test_build_gradle_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("build.gradle"),
        "dependencies {\n    implementation 'com.google.guava:guava:31.0-jre'\n    implementation 'org.springframework:spring-core:5.3'\n}\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "java");
    assert!(deps.contains(&"guava".to_string()));
    assert!(deps.contains(&"spring-core".to_string()));
}

#[test]
fn test_build_gradle_kts_preferred_over_plain() {
    let tmp = TempDir::new().unwrap();
    // Plain file has "gson", KTS file has "jackson-databind" — they are distinct
    // so we can assert the KTS-specific dep is present and the plain one absent.
    fs::write(
        tmp.path().join("build.gradle"),
        "implementation 'com.google.code.gson:gson:2.10'\n",
    )
    .unwrap();
    fs::write(
        tmp.path().join("build.gradle.kts"),
        "implementation(\"com.fasterxml.jackson.core:jackson-databind:2.15\")\n",
    )
    .unwrap();
    let (_, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    // KTS file must win
    assert!(
        deps.contains(&"jackson-databind".to_string()),
        "KTS-specific dep must be present"
    );
    assert!(
        !deps.contains(&"gson".to_string()),
        "plain gradle dep must not appear when KTS file exists"
    );
}

// ── mix.exs ───────────────────────────────────────────────────────────────

#[test]
fn test_mix_exs_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("mix.exs"),
        "defp deps do\n  [{:phoenix, \"~> 1.7\"}, {:ecto, \"~> 3.0\"}]\nend\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "elixir");
    assert!(deps.contains(&"phoenix".to_string()));
    assert!(deps.contains(&"ecto".to_string()));
}

#[test]
fn test_mix_exs_no_false_positives_from_global_atoms() {
    let tmp = TempDir::new().unwrap();
    // Common Elixir atoms appear OUTSIDE deps/do block — must not be included
    fs::write(
        tmp.path().join("mix.exs"),
        "defmodule MyApp.MixProject do\n\
         use Mix.Project\n\
         def project do\n\
           [app: :my_app, version: \"0.1.0\"]\n\
         end\n\
         def application do\n\
           [extra_applications: [:logger, :runtime_tools], mod: {:my_app, []}]\n\
         end\n\
         defp deps do\n\
           [{:phoenix, \"~> 1.7\"}]\n\
         end\n\
         end\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "elixir");
    assert!(deps.contains(&"phoenix".to_string()));
    // These atoms appear outside `defp deps` and must NOT show up
    assert!(!deps.contains(&"logger".to_string()));
    assert!(!deps.contains(&"runtime_tools".to_string()));
    assert!(!deps.contains(&"my_app".to_string()));
}

// ── *.csproj ──────────────────────────────────────────────────────────────

#[test]
fn test_csproj_extracts_deps() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("MyApp.csproj"),
        r#"<Project><ItemGroup><PackageReference Include="Newtonsoft.Json" Version="13.0" /><PackageReference Include="Microsoft.Extensions.Logging" /></ItemGroup></Project>"#,
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "csharp");
    assert!(deps.contains(&"Newtonsoft.Json".to_string()));
    assert!(deps.contains(&"Microsoft.Extensions.Logging".to_string()));
}

#[test]
fn test_csproj_missing() {
    let tmp = TempDir::new().unwrap();
    assert!(read_manifest(tmp.path()).unwrap().is_none());
}

#[test]
fn test_csproj_multiple_files_deterministic_order() {
    let tmp = TempDir::new().unwrap();
    // Two .csproj files — alpha-first should win deterministically
    fs::write(
        tmp.path().join("Alpha.csproj"),
        r#"<Project><ItemGroup><PackageReference Include="AlphaDep" /></ItemGroup></Project>"#,
    )
    .unwrap();
    fs::write(
        tmp.path().join("Zebra.csproj"),
        r#"<Project><ItemGroup><PackageReference Include="ZebraDep" /></ItemGroup></Project>"#,
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "csharp");
    // Alpha.csproj must always win
    assert!(deps.contains(&"AlphaDep".to_string()));
    assert!(!deps.contains(&"ZebraDep".to_string()));
}

// ── Package.swift ─────────────────────────────────────────────────────────

#[test]
fn test_package_swift_extracts_dep_names_from_url() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("Package.swift"),
        "let package = Package(\n  dependencies: [\n    .package(url: \"https://github.com/vapor/vapor.git\", from: \"4.0.0\"),\n    .package(url: \"https://github.com/apple/swift-argument-parser\", from: \"1.0.0\"),\n  ]\n)\n",
    )
    .unwrap();
    let (tag, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "swift");
    // Must extract package names, not raw lines
    assert!(deps.contains(&"vapor".to_string()));
    assert!(deps.contains(&"swift-argument-parser".to_string()));
    // Must NOT contain raw `.package(` lines
    for dep in &deps {
        assert!(
            !dep.contains(".package("),
            "dep should be a name, not a raw line: {dep}"
        );
    }
}

#[test]
fn test_package_swift_name_param_preferred() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("Package.swift"),
        ".package(name: \"MyLib\", url: \"https://github.com/org/some-repo.git\", from: \"1.0\")\n",
    )
    .unwrap();
    let (_, deps) = read_manifest(tmp.path()).unwrap().unwrap();
    // name: parameter wins over URL basename
    assert!(deps.contains(&"MyLib".to_string()));
}

#[test]
fn test_package_swift_missing() {
    let tmp = TempDir::new().unwrap();
    assert!(read_manifest(tmp.path()).unwrap().is_none());
}

// ── read_manifest priority ────────────────────────────────────────────────

#[test]
fn test_read_manifest_no_files_returns_none() {
    let tmp = TempDir::new().unwrap();
    let result = read_manifest(tmp.path()).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_read_manifest_gemfile_takes_priority_over_requirements() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Gemfile"), "gem 'rails'\n").unwrap();
    fs::write(tmp.path().join("requirements.txt"), "flask\n").unwrap();
    let (tag, _) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "ruby");
}

#[test]
fn test_read_manifest_pyproject_takes_priority_over_requirements() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pyproject.toml"),
        "[project]\ndependencies = [\"flask\"]\n",
    )
    .unwrap();
    fs::write(tmp.path().join("requirements.txt"), "django\n").unwrap();
    let (tag, _) = read_manifest(tmp.path()).unwrap().unwrap();
    assert_eq!(tag, "python");
}
