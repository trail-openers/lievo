// Cargo workspace detection

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn detect_cargo_workspace(repo_path: &Path) -> Option<HashMap<String, String>> {
    let cargo_toml = repo_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        return None;
    }

    let content = fs::read_to_string(&cargo_toml).ok()?;
    let toml_value: toml::Value = toml::from_str(&content).ok()?;

    let workspace = toml_value.get("workspace")?;
    let members = workspace.get("members")?.as_array()?;

    if members.is_empty() {
        return None;
    }

    let mut subsystems = HashMap::new();
    subsystems.insert(".".to_string(), "root".to_string());

    for member in members {
        if let Some(member_str) = member.as_str() {
            // Phase A simplification: only simple paths, no glob expansion
            if !member_str.contains('*') {
                let name = member_str
                    .split('/')
                    .next_back()
                    .unwrap_or(member_str)
                    .to_string();
                subsystems.insert(member_str.to_string(), name);
            }
        }
    }

    Some(subsystems)
}
