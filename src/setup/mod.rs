pub mod claude;
pub mod codex;
pub mod cursor;
pub mod kiro;
pub mod windsurf;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::manifest::PackageType;
use crate::config::settings::Settings;

/// Lightweight struct for reading setup-relevant fields from apkg.json.
/// Does NOT use `deny_unknown_fields` so it tolerates extra fields
/// the strict `Manifest` struct doesn't know about.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub package_type: PackageType,
    pub description: String,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub skill: Option<SkillInfo>,
    #[serde(default)]
    pub agent: Option<AgentInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    #[serde(default)]
    pub capabilities: Vec<String>,
    // Deserialized but not currently used — kept for forward compatibility.
    #[serde(default)]
    #[allow(dead_code)]
    pub model_compatibility: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    pub streaming: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    #[serde(default)]
    pub tools: Vec<AgentTool>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub model_preference: Vec<String>,
    // Deserialized but not currently used — kept for forward compatibility.
    #[serde(default)]
    #[allow(dead_code)]
    pub memory: Option<serde_json::Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub orchestration: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTool {
    pub name: String,
    pub package: String,
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Cursor,
    ClaudeCode,
    Windsurf,
    Kiro,
    Codex,
}

impl Tool {
    /// Map a config key (e.g. "cursor", "claude-code") to a Tool variant.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "cursor" => Some(Tool::Cursor),
            "claude-code" => Some(Tool::ClaudeCode),
            "windsurf" => Some(Tool::Windsurf),
            "kiro" => Some(Tool::Kiro),
            "codex" => Some(Tool::Codex),
            _ => None,
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tool::Cursor => write!(f, "Cursor"),
            Tool::ClaudeCode => write!(f, "Claude Code"),
            Tool::Windsurf => write!(f, "Windsurf"),
            Tool::Kiro => write!(f, "Kiro"),
            Tool::Codex => write!(f, "Codex"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupTarget {
    All,
    Only(Tool),
}

pub struct SetupContext {
    pub project_root: PathBuf,
    pub install_dir: PathBuf,
    pub target: SetupTarget,
}

pub struct SetupAction {
    #[allow(dead_code)]
    pub tool: Tool,
    pub path: PathBuf,
}

pub struct SetupReport {
    pub tools: Vec<Tool>,
    pub created: Vec<SetupAction>,
    pub warnings: Vec<String>,
}

/// Detect which AI tools are configured in the project.
pub fn detect_tools(project_root: &Path) -> Vec<Tool> {
    let mut tools = Vec::new();
    if project_root.join(".cursor").is_dir() {
        tools.push(Tool::Cursor);
    }
    if project_root.join(".claude").is_dir() {
        tools.push(Tool::ClaudeCode);
    }
    if project_root.join(".windsurf").is_dir() {
        tools.push(Tool::Windsurf);
    }
    if project_root.join(".kiro").is_dir() {
        tools.push(Tool::Kiro);
    }
    if project_root.join(".codex").is_dir() {
        tools.push(Tool::Codex);
    }
    tools
}

/// Load `PackageInfo` from the extracted package's apkg.json.
pub fn load_package_info(install_dir: &Path) -> Result<PackageInfo, String> {
    let manifest_path = install_dir.join("apkg.json");
    let content = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read installed manifest: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse installed manifest: {e}"))
}

/// Sanitize a package name for use as a filename.
/// `@scope/name` -> `scope--name`
pub fn config_file_stem(name: &str) -> String {
    let stripped = name.strip_prefix('@').unwrap_or(name);
    stripped.replace('/', "--")
}

/// If the value looks like a file path, read its content from the package dir.
/// Otherwise return it as-is (inline text).
pub fn resolve_system_prompt(value: &str, install_dir: &Path) -> String {
    let ext_is = |ext: &str| {
        Path::new(value)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    };
    let is_file_path = value.contains('/') || ext_is("md") || ext_is("txt");

    if is_file_path {
        let path = install_dir.join(value);
        match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => value.to_string(),
        }
    } else {
        value.to_string()
    }
}

/// Returns true if the package type supports tool setup.
fn is_setup_eligible(package_type: &PackageType) -> bool {
    matches!(package_type, PackageType::Skill | PackageType::Agent)
}

/// Run post-install setup for detected AI tools.
/// Never returns an error — setup failures are captured as warnings.
/// Resolve which tools to set up from the `defaultSetup` config.
/// Returns `None` if no config is set (caller should fall back to auto-detect).
fn tools_from_config() -> Option<Vec<Tool>> {
    let settings = Settings::load().ok()?;
    let keys = settings.enabled_setup_tools()?;
    let tools: Vec<Tool> = keys.into_iter().filter_map(Tool::from_key).collect();
    Some(tools)
}

pub fn run_setup(ctx: &SetupContext) -> SetupReport {
    let tools: Vec<Tool> = match &ctx.target {
        SetupTarget::All => {
            // If defaultSetup config exists, use it; otherwise auto-detect.
            tools_from_config().unwrap_or_else(|| detect_tools(&ctx.project_root))
        }
        // Explicit --setup flag: always use that tool — overrides config.
        SetupTarget::Only(target) => vec![*target],
    };

    if tools.is_empty() {
        return SetupReport {
            tools: Vec::new(),
            created: Vec::new(),
            warnings: Vec::new(),
        };
    }

    let info = match load_package_info(&ctx.install_dir) {
        Ok(info) => info,
        Err(msg) => {
            return SetupReport {
                tools,
                created: Vec::new(),
                warnings: vec![format!("Tool setup skipped: {msg}")],
            };
        }
    };

    if !is_setup_eligible(&info.package_type) {
        return SetupReport {
            tools,
            created: Vec::new(),
            warnings: Vec::new(),
        };
    }

    let mut created = Vec::new();
    let mut warnings = Vec::new();

    for &tool in &tools {
        if tool == Tool::ClaudeCode {
            match claude::setup_claude(&ctx.project_root, &ctx.install_dir, &info) {
                Ok(paths) => {
                    for path in paths {
                        created.push(SetupAction { tool, path });
                    }
                }
                Err(msg) => warnings.push(format!("Tool setup skipped ({tool}): {msg}")),
            }
            continue;
        }

        let result = match tool {
            Tool::Cursor => cursor::setup_cursor(&ctx.project_root, &ctx.install_dir, &info),
            Tool::Windsurf => windsurf::setup_windsurf(&ctx.project_root, &ctx.install_dir, &info),
            Tool::Kiro => kiro::setup_kiro(&ctx.project_root, &ctx.install_dir, &info),
            Tool::Codex => codex::setup_codex(&ctx.project_root, &ctx.install_dir, &info),
            Tool::ClaudeCode => unreachable!(),
        };
        match result {
            Ok(path) => created.push(SetupAction { tool, path }),
            Err(msg) => warnings.push(format!("Tool setup skipped ({tool}): {msg}")),
        }
    }

    SetupReport {
        tools,
        created,
        warnings,
    }
}

/// Display a human-readable summary of setup actions.
pub fn display_report(report: &SetupReport) {
    use crate::util::display;

    for warning in &report.warnings {
        display::warn(warning);
    }

    if report.created.is_empty() {
        return;
    }

    let tool_names: Vec<String> = report.tools.iter().map(ToString::to_string).collect();
    println!();
    display::info(&format!("Detected tools: {}", tool_names.join(", ")));
    for action in &report.created {
        println!("  Created: {}", action.path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_file_stem_scoped() {
        assert_eq!(
            config_file_stem("@acme/code-reviewer"),
            "acme--code-reviewer"
        );
    }

    #[test]
    fn test_config_file_stem_unscoped() {
        assert_eq!(config_file_stem("my-package"), "my-package");
    }

    #[test]
    fn test_config_file_stem_nested_scope() {
        assert_eq!(config_file_stem("@org/sub/name"), "org--sub--name");
    }

    #[test]
    fn test_detect_tools_none() {
        let tmp = TempDir::new().unwrap();
        let tools = detect_tools(tmp.path());
        assert!(tools.is_empty());
    }

    #[test]
    fn test_detect_tools_cursor_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Cursor]);
    }

    #[test]
    fn test_detect_tools_claude_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::ClaudeCode]);
    }

    #[test]
    fn test_detect_tools_both() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Cursor, Tool::ClaudeCode]);
    }

    #[test]
    fn test_resolve_system_prompt_inline() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_system_prompt("You are a helpful assistant.", tmp.path());
        assert_eq!(result, "You are a helpful assistant.");
    }

    #[test]
    fn test_resolve_system_prompt_file_path() {
        let tmp = TempDir::new().unwrap();
        let prompts_dir = tmp.path().join("prompts");
        fs::create_dir(&prompts_dir).unwrap();
        fs::write(prompts_dir.join("system.md"), "File-based prompt content").unwrap();

        let result = resolve_system_prompt("prompts/system.md", tmp.path());
        assert_eq!(result, "File-based prompt content");
    }

    #[test]
    fn test_resolve_system_prompt_missing_file() {
        let tmp = TempDir::new().unwrap();
        let result = resolve_system_prompt("prompts/missing.md", tmp.path());
        assert_eq!(result, "prompts/missing.md");
    }

    #[test]
    fn test_resolve_system_prompt_md_extension() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("prompt.md"), "Markdown prompt").unwrap();
        let result = resolve_system_prompt("prompt.md", tmp.path());
        assert_eq!(result, "Markdown prompt");
    }

    #[test]
    fn test_load_package_info_skill() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.2.0",
            "type": "skill",
            "description": "AI-powered code review",
            "license": "MIT",
            "main": "src/index.ts",
            "skill": {
                "capabilities": ["code-review", "bug-detection"],
                "inputSchema": "schema/input.json",
                "outputSchema": "schema/output.json"
            }
        }"#;
        fs::write(tmp.path().join("apkg.json"), json).unwrap();
        let info = load_package_info(tmp.path()).unwrap();
        assert_eq!(info.name, "@acme/code-reviewer");
        assert!(matches!(info.package_type, PackageType::Skill));
        assert_eq!(info.main.as_deref(), Some("src/index.ts"));
        let skill = info.skill.unwrap();
        assert_eq!(skill.capabilities, vec!["code-review", "bug-detection"]);
    }

    #[test]
    fn test_load_package_info_agent() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{
            "name": "@acme/research-agent",
            "version": "0.8.0",
            "type": "agent",
            "description": "Research agent",
            "license": "MIT",
            "main": "src/agent.ts",
            "agent": {
                "tools": [
                    { "name": "web-search", "package": "@acme/web-search", "required": true },
                    { "name": "formatter", "package": "@acme/fmt" }
                ],
                "systemPrompt": "prompts/system.md",
                "modelPreference": ["claude-sonnet-4-6", "gpt-4o"]
            }
        }"#;
        fs::write(tmp.path().join("apkg.json"), json).unwrap();
        let info = load_package_info(tmp.path()).unwrap();
        assert!(matches!(info.package_type, PackageType::Agent));
        let agent = info.agent.unwrap();
        assert_eq!(agent.tools.len(), 2);
        assert!(agent.tools[1].required);
        assert_eq!(agent.model_preference, vec!["claude-sonnet-4-6", "gpt-4o"]);
    }

    #[test]
    fn test_load_package_info_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        let result = load_package_info(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_is_setup_eligible() {
        assert!(is_setup_eligible(&PackageType::Skill));
        assert!(is_setup_eligible(&PackageType::Agent));
        assert!(!is_setup_eligible(&PackageType::McpServer));
        assert!(!is_setup_eligible(&PackageType::Prompt));
        assert!(!is_setup_eligible(&PackageType::Config));
        assert!(!is_setup_eligible(&PackageType::Library));
        assert!(!is_setup_eligible(&PackageType::Composite));
    }

    #[test]
    fn test_run_setup_no_tools() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/pkg");
        fs::create_dir_all(&install_dir).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::All,
        });

        assert!(report.tools.is_empty());
        assert!(report.created.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_run_setup_non_eligible_type() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/mcp-server");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "my-server",
            "version": "1.0.0",
            "type": "mcp-server",
            "description": "An MCP server",
            "license": "MIT"
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::All,
        });

        assert_eq!(report.tools.len(), 1);
        assert!(report.created.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_run_setup_creates_configs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.2.0",
            "type": "skill",
            "description": "AI-powered code review",
            "license": "MIT",
            "main": "src/index.ts",
            "skill": {
                "capabilities": ["code-review", "bug-detection"],
                "inputSchema": {},
                "outputSchema": {}
            }
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::All,
        });

        assert_eq!(report.tools.len(), 2);
        assert_eq!(report.created.len(), 2);
        assert!(report.warnings.is_empty());
        assert!(tmp
            .path()
            .join(".cursor/skills/acme--code-reviewer.mdc")
            .exists());
        assert!(tmp
            .path()
            .join(".claude/skills/acme--code-reviewer.md")
            .exists());
    }

    #[test]
    fn test_run_setup_warns_on_bad_manifest() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/broken");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(install_dir.join("apkg.json"), "{ invalid json }").unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::All,
        });

        assert_eq!(report.tools.len(), 1);
        assert!(report.created.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("Tool setup skipped"));
    }

    #[test]
    fn test_run_setup_only_cursor() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.2.0",
            "type": "skill",
            "description": "AI-powered code review",
            "license": "MIT",
            "main": "src/index.ts",
            "skill": {
                "capabilities": ["code-review"],
                "inputSchema": {},
                "outputSchema": {}
            }
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::Only(Tool::Cursor),
        });

        assert_eq!(report.tools, vec![Tool::Cursor]);
        assert_eq!(report.created.len(), 1);
        assert!(tmp
            .path()
            .join(".cursor/skills/acme--code-reviewer.mdc")
            .exists());
        assert!(!tmp
            .path()
            .join(".claude/skills/acme--code-reviewer.md")
            .exists());
    }

    #[test]
    fn test_run_setup_only_claude() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.0.0",
            "type": "skill",
            "description": "Code review",
            "license": "MIT",
            "skill": {
                "capabilities": ["code-review"],
                "inputSchema": {},
                "outputSchema": {}
            }
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::Only(Tool::ClaudeCode),
        });

        assert_eq!(report.tools, vec![Tool::ClaudeCode]);
        assert_eq!(report.created.len(), 1);
        assert!(!tmp
            .path()
            .join(".cursor/skills/acme--code-reviewer.mdc")
            .exists());
        assert!(tmp
            .path()
            .join(".claude/skills/acme--code-reviewer.md")
            .exists());
    }

    #[test]
    fn test_run_setup_only_claude_creates_dir() {
        let tmp = TempDir::new().unwrap();
        // No .claude directory pre-created

        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.0.0",
            "type": "skill",
            "description": "Code review",
            "license": "MIT",
            "skill": {
                "capabilities": ["code-review"],
                "inputSchema": {},
                "outputSchema": {}
            }
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::Only(Tool::ClaudeCode),
        });

        assert_eq!(report.tools, vec![Tool::ClaudeCode]);
        assert_eq!(report.created.len(), 1);
        assert!(tmp
            .path()
            .join(".claude/skills/acme--code-reviewer.md")
            .exists());
    }

    #[test]
    fn test_run_setup_only_cursor_creates_dir() {
        let tmp = TempDir::new().unwrap();
        // No .cursor directory pre-created

        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@acme/code-reviewer",
            "version": "1.0.0",
            "type": "skill",
            "description": "Code review",
            "license": "MIT",
            "skill": {
                "capabilities": ["code-review"],
                "inputSchema": {},
                "outputSchema": {}
            }
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::Only(Tool::Cursor),
        });

        assert_eq!(report.tools, vec![Tool::Cursor]);
        assert_eq!(report.created.len(), 1);
        assert!(tmp
            .path()
            .join(".cursor/skills/acme--code-reviewer.mdc")
            .exists());
    }

    #[test]
    fn test_tool_display() {
        assert_eq!(Tool::Cursor.to_string(), "Cursor");
        assert_eq!(Tool::ClaudeCode.to_string(), "Claude Code");
        assert_eq!(Tool::Windsurf.to_string(), "Windsurf");
        assert_eq!(Tool::Kiro.to_string(), "Kiro");
        assert_eq!(Tool::Codex.to_string(), "Codex");
    }

    #[test]
    fn test_detect_tools_windsurf_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".windsurf")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Windsurf]);
    }

    #[test]
    fn test_detect_tools_kiro_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".kiro")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Kiro]);
    }

    #[test]
    fn test_detect_tools_codex_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".codex")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Codex]);
    }

    #[test]
    fn test_detect_tools_all_five() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        fs::create_dir(tmp.path().join(".windsurf")).unwrap();
        fs::create_dir(tmp.path().join(".kiro")).unwrap();
        fs::create_dir(tmp.path().join(".codex")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(
            tools,
            vec![
                Tool::Cursor,
                Tool::ClaudeCode,
                Tool::Windsurf,
                Tool::Kiro,
                Tool::Codex
            ]
        );
    }

    #[test]
    fn test_tool_from_key() {
        assert_eq!(Tool::from_key("cursor"), Some(Tool::Cursor));
        assert_eq!(Tool::from_key("claude-code"), Some(Tool::ClaudeCode));
        assert_eq!(Tool::from_key("windsurf"), Some(Tool::Windsurf));
        assert_eq!(Tool::from_key("kiro"), Some(Tool::Kiro));
        assert_eq!(Tool::from_key("codex"), Some(Tool::Codex));
        assert_eq!(Tool::from_key("unknown"), None);
    }
}
