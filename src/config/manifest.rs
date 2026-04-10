use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageType {
    Project,
    Skill,
    Agent,
    // McpServer,
    Command,
    // Config,
    // Library,
    // Composite,
    Rule,
}

impl PackageType {
    pub const VARIANTS: &[&str] = &[
        "project",
        "skill",
        "agent",
        // "mcp-server",
        "command",
        // "config",
        // "library",
        // "composite",
        "rule",
    ];

    /// Plural directory name used for tool setup (e.g. `.claude/skills/`).
    pub fn dir_name(&self) -> &'static str {
        match self {
            PackageType::Project => "projects",
            PackageType::Skill => "skills",
            PackageType::Agent => "agents",
            // PackageType::McpServer => "mcp-servers",
            PackageType::Command => "commands",
            // PackageType::Config => "configs",
            // PackageType::Library => "libraries",
            // PackageType::Composite => "composites",
            PackageType::Rule => "rules",
        }
    }

    /// Returns true if this type represents a publishable package.
    pub fn is_publishable(&self) -> bool {
        !matches!(self, PackageType::Project)
    }
}

impl std::fmt::Display for PackageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PackageType::Project => "project",
            PackageType::Skill => "skill",
            PackageType::Agent => "agent",
            // PackageType::McpServer => "mcp-server",
            PackageType::Command => "command",
            // PackageType::Config => "config",
            // PackageType::Library => "library",
            // PackageType::Composite => "composite",
            PackageType::Rule => "rule",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub package_type: PackageType,
    pub description: String,
    pub license: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<Author>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripts: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_permissions: Option<HookPermissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct HookPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
}

pub const MANIFEST_FILE: &str = "apkg.json";

pub fn load(dir: &Path) -> Result<Manifest, AppError> {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return Err(AppError::ManifestNotFound);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| AppError::Manifest(format!("Failed to read {MANIFEST_FILE}: {e}")))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .map_err(|e| AppError::Manifest(format!("Invalid {MANIFEST_FILE}: {e}")))?;
    Ok(manifest)
}

pub fn save(dir: &Path, manifest: &Manifest) -> Result<(), AppError> {
    let path = dir.join(MANIFEST_FILE);
    let content = serde_json::to_string_pretty(manifest)?;
    fs::write(&path, format!("{content}\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let json = r#"{
            "name": "my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "A skill",
            "license": "MIT"
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "my-skill");
        assert_eq!(m.version, "1.0.0");
        assert!(matches!(m.package_type, PackageType::Skill));
        assert_eq!(m.description, "A skill");
        assert_eq!(m.license, "MIT");
        assert!(m.keywords.is_none());
        assert!(m.dependencies.is_none());
    }

    #[test]
    fn test_parse_full_manifest() {
        let json = r#"{
            "name": "@acme/summarizer",
            "version": "2.0.0",
            "type": "command",
            "description": "A command",
            "license": "Apache-2.0",
            "readme": "README.md",
            "keywords": ["ai", "command"],
            "authors": [{"name": "acme"}],
            "repository": "https://github.com/acme/summarizer",
            "homepage": "https://acme.dev",
            "dependencies": {
                "some-dep": "^1.0.0"
            }
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "@acme/summarizer");
        assert!(matches!(m.package_type, PackageType::Command));
        assert_eq!(m.keywords.unwrap(), vec!["ai", "command"]);
        assert_eq!(m.dependencies.unwrap().len(), 1);
    }

    #[test]
    fn test_reject_unknown_fields() {
        let json = r#"{
            "name": "my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "A skill",
            "license": "MIT",
            "unknownField": true
        }"#;
        let result: Result<Manifest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_package_types() {
        for type_str in PackageType::VARIANTS {
            let json = format!(
                r#"{{"name":"t","version":"0.1.0","type":"{type_str}","description":"d","license":"MIT"}}"#
            );
            let m: Manifest = serde_json::from_str(&json).unwrap();
            assert_eq!(m.package_type.to_string(), *type_str);
        }
    }

    #[test]
    fn test_roundtrip_save_load() {
        let tmp = tempfile::tempdir().unwrap();
        let m = Manifest {
            name: "test-pkg".to_string(),
            version: "0.1.0".to_string(),
            package_type: PackageType::Agent,
            description: "test".to_string(),
            license: "MIT".to_string(),
            readme: None,
            keywords: Some(vec!["ai".to_string()]),
            authors: None,
            repository: None,
            homepage: None,
            dependencies: None,
            dev_dependencies: None,
            peer_dependencies: None,
            scripts: None,
            hook_permissions: None,
        };
        save(tmp.path(), &m).unwrap();
        let loaded = load(tmp.path()).unwrap();
        assert_eq!(loaded.name, "test-pkg");
        assert!(matches!(loaded.package_type, PackageType::Agent));
        assert_eq!(loaded.keywords.unwrap(), vec!["ai"]);
    }

    #[test]
    fn test_load_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load(tmp.path());
        assert!(result.is_err());
    }
}
