use super::*;

#[test]
fn test_extract_mix_deps_does_not_leak_into_other_functions() {
    let content = r#"defmodule MyApp.MixProject do
  def project do
    [app: :myapp]
  end

  defp deps do
    [
      {:phoenix, "~> 1.7"},
      {:ecto, "~> 3.11"}
    ]
  end

  defp aliases do
    [
      {:ok, "some alias"},
      setup: ["deps.get"]
    ]
  end
end
"#;
    let deps = extract_mix_deps(content);
    assert!(
        deps.contains(&"phoenix".to_string()),
        "should contain phoenix"
    );
    assert!(deps.contains(&"ecto".to_string()), "should contain ecto");
    assert!(
        !deps.contains(&"ok".to_string()),
        ":ok from aliases function must not appear in deps"
    );
    assert_eq!(deps.len(), 2, "only phoenix and ecto should be extracted");
}

#[test]
fn test_extract_mix_deps_basic() {
    let content = r#"defmodule App.MixProject do
  defp deps do
    [
      {:plug, "~> 1.14"},
      {:jason, "~> 1.4"}
    ]
  end
end
"#;
    let deps = extract_mix_deps(content);
    assert_eq!(deps, vec!["jason", "plug"]);
}

#[test]
fn test_extract_mix_deps_no_defp_deps() {
    let content = "defmodule Foo do\nend\n";
    let deps = extract_mix_deps(content);
    assert!(deps.is_empty());
}

#[test]
fn test_extract_pom_artifact_ids_basic() {
    let content = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <artifactId>artifact1</artifactId>
    </dependency>
    <dependency>
      <artifactId>artifact2</artifactId>
    </dependency>
  </dependencies>
</project>"#;

    let deps = extract_pom_artifact_ids(content);
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&"artifact1".to_string()));
    assert!(deps.contains(&"artifact2".to_string()));
}

#[test]
fn test_extract_pom_artifact_ids_multiple_in_same_dependency() {
    let content = r#"<?xml version="1.0"?>
<project>
  <dependencies>
    <dependency>
      <artifactId>artifact1</artifactId>
      <artifactId>artifact2</artifactId>
    </dependency>
    <dependency>
      <artifactId>artifact3</artifactId>
    </dependency>
  </dependencies>
</project>"#;

    let deps = extract_pom_artifact_ids(content);
    assert_eq!(
        deps.len(),
        3,
        "Should extract all artifact IDs including multiple in same dependency"
    );
    assert!(deps.contains(&"artifact1".to_string()));
    assert!(deps.contains(&"artifact2".to_string()));
    assert!(deps.contains(&"artifact3".to_string()));
}

#[test]
fn test_extract_pom_artifact_ids_empty() {
    let content = r#"<?xml version="1.0"?>
<project>
</project>"#;

    let deps = extract_pom_artifact_ids(content);
    assert_eq!(deps.len(), 0);
}

#[test]
fn test_extract_pom_artifact_ids_malformed_missing_closing_tag() {
    let content = r#"<project>
  <dependencies>
    <dependency>
      <artifactId>artifact1
    </dependency>
  </dependencies>
</project>"#;

    let deps = extract_pom_artifact_ids(content);
    // Should handle gracefully - either skip or extract what it can
    // The important thing is not to panic
    assert!(!deps.is_empty() || deps.is_empty()); // Just verify it completes
}

#[test]
fn test_extract_pom_artifact_ids_nested_tags() {
    let content = r#"<project>
  <dependencies>
    <dependency>
      <artifactId>
        <nested>artifact1</nested>
      </artifactId>
    </dependency>
  </dependencies>
</project>"#;

    let deps = extract_pom_artifact_ids(content);
    // Should handle gracefully without panicking
    // May extract empty string or malformed content
    assert!(deps.len() <= 1);
}
