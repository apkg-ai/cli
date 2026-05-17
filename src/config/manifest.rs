use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        "project", "skill", "agent",   // "mcp-server",
        "command", // "config",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    #[default]
    Public,
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
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
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<BTreeMap<String, String>>,
    pub visibility: Visibility,
}

pub const MANIFEST_FILE: &str = "apkg.json";

/// Known AI coding tool identifiers. Keep in sync with the server's
/// `KNOWN_TOOLS` in `api/api-package/src/validation/schemas.ts`.
pub const KNOWN_TOOLS: &[&str] = &["claude-code", "cursor", "codex"];

/// Returns an error message if any entry in `targets` is not a known tool,
/// or if `targets` is empty. Returns `None` on success.
pub fn validate_targets_known(targets: &[String]) -> Option<String> {
    if targets.is_empty() {
        return Some(format!(
            "`targets` must not be empty. Known tools: {}",
            KNOWN_TOOLS.join(", ")
        ));
    }
    let unknown: Vec<&String> = targets
        .iter()
        .filter(|v| !KNOWN_TOOLS.contains(&v.as_str()))
        .collect();
    if unknown.is_empty() {
        None
    } else {
        let list = unknown
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "Unknown tool(s) in `targets`: {list}. Known tools: {}",
            KNOWN_TOOLS.join(", ")
        ))
    }
}

/// Returns an error message if `origin` is not a known tool, or is not
/// a member of `targets`. Returns `None` on success.
pub fn validate_origin(origin: &str, targets: &[String]) -> Option<String> {
    if !KNOWN_TOOLS.contains(&origin) {
        return Some(format!(
            "Unknown `origin` \"{origin}\". Known tools: {}",
            KNOWN_TOOLS.join(", ")
        ));
    }
    if !targets.iter().any(|t| t == origin) {
        return Some(format!(
            "`origin` \"{origin}\" must be included in `targets`"
        ));
    }
    None
}

pub fn load(dir: &Path) -> Result<Manifest, AppError> {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return Err(AppError::ManifestNotFound);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| AppError::Manifest(format!("Failed to read {MANIFEST_FILE}: {e}")))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .map_err(|e| AppError::Manifest(format!("Invalid {MANIFEST_FILE}: {e}")))?;
    if manifest.package_type != PackageType::Project {
        if manifest.origin.is_none() {
            return Err(AppError::Manifest(
                "`origin` is required for non-project packages".into(),
            ));
        }
        if manifest.targets.is_none() {
            return Err(AppError::Manifest(
                "`targets` is required for non-project packages".into(),
            ));
        }
    }
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
            "license": "MIT",
            "origin": "claude-code",
            "targets": ["claude-code"],
            "visibility": "public"
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "my-skill");
        assert_eq!(m.version, "1.0.0");
        assert!(matches!(m.package_type, PackageType::Skill));
        assert_eq!(m.description, "A skill");
        assert_eq!(m.license, "MIT");
        assert_eq!(m.origin.as_deref(), Some("claude-code"));
        assert_eq!(m.targets.as_deref(), Some(&["claude-code".to_string()][..]));
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
            "origin": "claude-code",
            "targets": ["claude-code", "cursor"],
            "dependencies": {
                "some-dep": "^1.0.0"
            },
            "visibility": "private"
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "@acme/summarizer");
        assert!(matches!(m.package_type, PackageType::Command));
        assert_eq!(m.origin.as_deref(), Some("claude-code"));
        assert_eq!(m.targets.unwrap(), vec!["claude-code", "cursor"]);
        assert_eq!(m.keywords.unwrap(), vec!["ai", "command"]);
        assert_eq!(m.dependencies.unwrap().len(), 1);
        assert_eq!(m.visibility, Visibility::Private);
    }

    #[test]
    fn test_reject_unknown_fields() {
        let json = r#"{
            "name": "my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "A skill",
            "license": "MIT",
            "origin": "claude-code",
            "targets": ["claude-code"],
            "visibility": "public",
            "unknownField": true
        }"#;
        let result: Result<Manifest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_all_package_types() {
        for type_str in PackageType::VARIANTS {
            let json = format!(
                r#"{{"name":"t","version":"0.1.0","type":"{type_str}","description":"d","license":"MIT","origin":"claude-code","targets":["claude-code"],"visibility":"public"}}"#
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
            origin: Some("claude-code".to_string()),
            targets: Some(vec!["claude-code".to_string()]),
            dependencies: None,
            visibility: Visibility::Public,
        };
        save(tmp.path(), &m).unwrap();
        let loaded = load(tmp.path()).unwrap();
        assert_eq!(loaded.name, "test-pkg");
        assert!(matches!(loaded.package_type, PackageType::Agent));
        assert_eq!(loaded.keywords.unwrap(), vec!["ai"]);
        assert_eq!(loaded.origin.as_deref(), Some("claude-code"));
        assert_eq!(loaded.targets.unwrap(), vec!["claude-code"]);
    }

    #[test]
    fn test_load_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_manifest_with_multiple_targets() {
        let json = r#"{
            "name": "@user/my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "A skill",
            "license": "MIT",
            "origin": "claude-code",
            "targets": ["claude-code", "cursor", "codex"],
            "visibility": "public"
        }"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.origin.as_deref(), Some("claude-code"));
        assert_eq!(m.targets.unwrap(), vec!["claude-code", "cursor", "codex"]);
    }

    #[test]
    fn test_reject_missing_origin_for_non_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(MANIFEST_FILE),
            r#"{
                "name": "@user/my-skill",
                "version": "1.0.0",
                "type": "skill",
                "description": "A skill",
                "license": "MIT",
                "targets": ["claude-code"],
                "visibility": "public"
            }"#,
        )
        .unwrap();
        let err = load(tmp.path()).expect_err("should fail");
        assert!(matches!(err, AppError::Manifest(ref msg) if msg.contains("`origin` is required")));
    }

    #[test]
    fn test_reject_missing_targets_for_non_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(MANIFEST_FILE),
            r#"{
                "name": "@user/my-skill",
                "version": "1.0.0",
                "type": "skill",
                "description": "A skill",
                "license": "MIT",
                "origin": "claude-code",
                "visibility": "public"
            }"#,
        )
        .unwrap();
        let err = load(tmp.path()).expect_err("should fail");
        assert!(
            matches!(err, AppError::Manifest(ref msg) if msg.contains("`targets` is required"))
        );
    }

    #[test]
    fn test_project_without_origin_targets_loads_ok() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(MANIFEST_FILE),
            r#"{
                "name": "@user/my-project",
                "version": "1.0.0",
                "type": "project",
                "description": "A project",
                "license": "MIT",
                "visibility": "public"
            }"#,
        )
        .unwrap();
        let m = load(tmp.path()).expect("should load");
        assert!(matches!(m.package_type, PackageType::Project));
        assert!(m.origin.is_none());
        assert!(m.targets.is_none());
    }

    #[test]
    fn test_reject_missing_visibility() {
        let json = r#"{
            "name": "@user/my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "A skill",
            "license": "MIT",
            "origin": "claude-code",
            "targets": ["claude-code"]
        }"#;
        let result: Result<Manifest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_visibility_default_is_public() {
        assert_eq!(Visibility::default(), Visibility::Public);
        assert_eq!(Visibility::Public.to_string(), "public");
        assert_eq!(Visibility::Private.to_string(), "private");
    }

    #[test]
    fn test_visibility_serialized_as_kebab_case() {
        let json = serde_json::to_string(&Visibility::Private).unwrap();
        assert_eq!(json, "\"private\"");
    }

    #[test]
    fn test_validate_targets_known_ok() {
        let values = vec!["claude-code".to_string(), "cursor".to_string()];
        assert!(validate_targets_known(&values).is_none());
    }

    #[test]
    fn test_validate_targets_known_unknown() {
        let values = vec!["claude-code".to_string(), "some-future-tool".to_string()];
        let err = validate_targets_known(&values).expect("should error");
        assert!(err.contains("some-future-tool"));
    }

    #[test]
    fn test_validate_targets_known_empty() {
        let err = validate_targets_known(&[]).expect("should error");
        assert!(err.contains("must not be empty"));
    }

    #[test]
    fn test_validate_origin_ok() {
        let targets = vec!["claude-code".to_string(), "cursor".to_string()];
        assert!(validate_origin("claude-code", &targets).is_none());
    }

    #[test]
    fn test_validate_origin_unknown() {
        let targets = vec!["gemini".to_string()];
        let err = validate_origin("gemini", &targets).expect("should error");
        assert!(err.contains("Unknown `origin`"));
    }

    #[test]
    fn test_validate_origin_not_in_targets() {
        let targets = vec!["cursor".to_string()];
        let err = validate_origin("claude-code", &targets).expect("should error");
        assert!(err.contains("must be included in `targets`"));
    }
}
