pub mod claude;
pub mod codex;
pub mod cursor;
// TODO: re-enable when ready
// pub mod kiro;
// pub mod windsurf;

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
    // Deserialized but not currently used in setup logic — kept for forward compatibility.
    #[serde(default)]
    #[allow(dead_code)]
    pub platform: Vec<String>,
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
    Codex,
    // TODO: re-enable when ready
    // Windsurf,
    // Kiro,
}

impl Tool {
    /// Map a config key (e.g. "cursor", "claude-code") to a Tool variant.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "cursor" => Some(Tool::Cursor),
            "claude-code" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            // TODO: re-enable when ready
            // "windsurf" => Some(Tool::Windsurf),
            // "kiro" => Some(Tool::Kiro),
            _ => None,
        }
    }

    /// Return the canonical config key for this tool.
    pub fn to_key(&self) -> &'static str {
        match self {
            Tool::Cursor => "cursor",
            Tool::ClaudeCode => "claude-code",
            Tool::Codex => "codex",
            // TODO: re-enable when ready
            // Tool::Windsurf => "windsurf",
            // Tool::Kiro => "kiro",
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tool::Cursor => write!(f, "Cursor"),
            Tool::ClaudeCode => write!(f, "Claude Code"),
            Tool::Codex => write!(f, "Codex"),
            // TODO: re-enable when ready
            // Tool::Windsurf => write!(f, "Windsurf"),
            // Tool::Kiro => write!(f, "Kiro"),
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
    if project_root.join(".codex").is_dir() {
        tools.push(Tool::Codex);
    }
    // TODO: re-enable when ready
    // if project_root.join(".windsurf").is_dir() {
    //     tools.push(Tool::Windsurf);
    // }
    // if project_root.join(".kiro").is_dir() {
    //     tools.push(Tool::Kiro);
    // }
    tools
}

/// Load `PackageInfo` from the extracted package's apkg.json.
pub fn load_package_info(install_dir: &Path) -> Result<PackageInfo, String> {
    let manifest_path = install_dir.join("apkg.json");
    let content = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read installed manifest: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse installed manifest: {e}"))
}

/// Build the relative path for a package within a tool config directory.
/// Scoped packages become two-component paths mirroring npm conventions:
/// `@scope/name` -> `@scope/name` (two directory levels)
/// `my-package`  -> `my-package`  (single directory level)
pub fn config_pkg_path(name: &str) -> PathBuf {
    PathBuf::from(name)
}

/// Extract the short name from a possibly-scoped package name.
/// `@acme/code-reviewer` -> `code-reviewer`
/// `my-package` -> `my-package`
pub fn package_short_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}


/// Find `.md` files in `install_dir` that contain YAML frontmatter and are
/// likely agent/skill definitions (not documentation).
pub(crate) fn find_definition_files(install_dir: &Path, require_frontmatter: bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(install_dir) else {
        return Vec::new();
    };

    let excluded: &[&str] = &["readme.md", "changelog.md", "license.md"];

    entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
        .filter(|e| {
            let name = e.file_name();
            let lower = name.to_string_lossy().to_lowercase();
            let is_excluded_name = excluded.contains(&lower.as_str());

            if is_excluded_name {
                // Even excluded names are kept if they have frontmatter
                // (frontmatter = definition file, not documentation).
                return fs::read_to_string(e.path())
                    .map(|c| c.starts_with("---\n") || c.starts_with("---\r\n"))
                    .unwrap_or(false);
            }
            true
        })
        .filter(|e| {
            if !require_frontmatter {
                return true;
            }
            fs::read_to_string(e.path())
                .map(|c| c.starts_with("---\n") || c.starts_with("---\r\n"))
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect()
}

/// Strip YAML frontmatter from content, returning only the body.
pub(crate) fn strip_frontmatter(content: &str) -> &str {
    if content.starts_with("---\n") || content.starts_with("---\r\n") {
        if let Some(end) = content[3..].find("\n---") {
            let after = end + 3 + 4; // skip past closing ---\n
            return content[after..].trim_start_matches('\n');
        }
    }
    content
}

/// Parse YAML frontmatter into key-value pairs.
/// Returns an empty vec if no frontmatter is present.
pub(crate) fn parse_frontmatter(content: &str) -> Vec<(String, String)> {
    let start = if content.starts_with("---\n") {
        4
    } else if content.starts_with("---\r\n") {
        5
    } else {
        return Vec::new();
    };

    let Some(end) = content[3..].find("\n---") else {
        return Vec::new();
    };

    let block = &content[start..3 + end];
    block
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let colon = line.find(':')?;
            let key = line[..colon].trim().to_string();
            let mut val = line[colon + 1..].trim().to_string();
            // Strip surrounding quotes
            if (val.starts_with('"') && val.ends_with('"'))
                || (val.starts_with('\'') && val.ends_with('\''))
            {
                val = val[1..val.len() - 1].to_string();
            }
            Some((key, val))
        })
        .collect()
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
    matches!(
        package_type,
        PackageType::Skill | PackageType::Agent | PackageType::Command | PackageType::Rule
    )
    // Note: PackageType::Project is intentionally excluded — projects consume
    // packages but are not installed into other projects.
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
        let result = match tool {
            Tool::ClaudeCode => {
                claude::setup_claude(&ctx.project_root, &ctx.install_dir, &info)
            }
            Tool::Cursor => {
                cursor::setup_cursor(&ctx.project_root, &ctx.install_dir, &info)
            }
            Tool::Codex => {
                codex::setup_codex(&ctx.project_root, &ctx.install_dir, &info)
            }
            // TODO: re-enable when ready
            // Tool::Windsurf => windsurf::setup_windsurf(..),
            // Tool::Kiro => kiro::setup_kiro(..),
        };
        match result {
            Ok(paths) => {
                for path in paths {
                    created.push(SetupAction { tool, path });
                }
            }
            Err(msg) => warnings.push(format!("Tool setup skipped ({tool}): {msg}")),
        }
    }

    // After all tool-specific setups, attempt to create CLAUDE.md -> AGENTS.md symlink.
    if let Some(action) = maybe_create_claude_md_symlink(&ctx.project_root, &tools) {
        created.push(action);
    }

    SetupReport {
        tools,
        created,
        warnings,
    }
}

/// If conditions are met, create a `CLAUDE.md` symlink pointing to `AGENTS.md`.
///
/// Conditions:
/// 1. The `symlinkClaudeMd` user setting is enabled (default: true)
/// 2. Claude Code is among the active tools
/// 3. `AGENTS.md` exists in `project_root`
/// 4. `CLAUDE.md` does not exist in `project_root` (neither as file nor symlink)
pub fn maybe_create_claude_md_symlink(
    project_root: &Path,
    tools: &[Tool],
) -> Option<SetupAction> {
    let enabled = Settings::load()
        .map(|s| s.symlink_claude_md_enabled())
        .unwrap_or(true);
    if !enabled {
        return None;
    }

    if !tools.contains(&Tool::ClaudeCode) {
        return None;
    }

    let agents_md = project_root.join("AGENTS.md");
    let claude_md = project_root.join("CLAUDE.md");

    if !agents_md.exists() {
        return None;
    }

    // Use symlink_metadata to detect any entry at that path (including dangling symlinks).
    if claude_md.symlink_metadata().is_ok() {
        return None;
    }

    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink("AGENTS.md", &claude_md).is_ok() {
            return Some(SetupAction {
                tool: Tool::ClaudeCode,
                path: claude_md,
            });
        }
    }
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_file("AGENTS.md", &claude_md).is_ok() {
            return Some(SetupAction {
                tool: Tool::ClaudeCode,
                path: claude_md,
            });
        }
    }

    None
}

/// If `CLAUDE.md` is a symlink pointing to `AGENTS.md` and `AGENTS.md` no
/// longer exists, remove the dangling symlink.
pub fn maybe_cleanup_claude_md_symlink(project_root: &Path) {
    let claude_md = project_root.join("CLAUDE.md");

    let Ok(metadata) = claude_md.symlink_metadata() else {
        return;
    };
    if !metadata.file_type().is_symlink() {
        return;
    }

    let Ok(target) = fs::read_link(&claude_md) else {
        return;
    };

    // Only clean up if it points to AGENTS.md
    if target != Path::new("AGENTS.md") && target != project_root.join("AGENTS.md") {
        return;
    }

    if !project_root.join("AGENTS.md").exists() {
        let _ = fs::remove_file(&claude_md);
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
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Mutex to serialize tests that mutate the HOME env var.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Run a closure with HOME pointing at a fresh temp directory so that
    /// `Settings::load()` finds no user config and returns defaults.
    fn with_temp_home<F>(f: F)
    where
        F: FnOnce(),
    {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };
        f();
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn test_config_pkg_path_scoped() {
        assert_eq!(
            config_pkg_path("@acme/code-reviewer"),
            PathBuf::from("@acme/code-reviewer")
        );
    }

    #[test]
    fn test_config_pkg_path_unscoped() {
        assert_eq!(config_pkg_path("my-package"), PathBuf::from("my-package"));
    }

    #[test]
    fn test_config_pkg_path_nested_scope() {
        assert_eq!(
            config_pkg_path("@org/sub/name"),
            PathBuf::from("@org/sub/name")
        );
    }

    #[test]
    fn test_package_short_name_scoped() {
        assert_eq!(package_short_name("@acme/code-reviewer"), "code-reviewer");
    }

    #[test]
    fn test_package_short_name_unscoped() {
        assert_eq!(package_short_name("my-package"), "my-package");
    }

    #[test]
    fn test_package_short_name_nested() {
        assert_eq!(package_short_name("@org/sub/name"), "name");
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
        assert!(is_setup_eligible(&PackageType::Command));
        assert!(is_setup_eligible(&PackageType::Rule));
        assert!(!is_setup_eligible(&PackageType::Project));
    }

    #[test]
    fn test_run_setup_no_tools() {
        with_temp_home(|| {
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
        });
    }

    #[test]
    fn test_run_setup_command_type() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();

        let install_dir = tmp.path().join("apkg_packages/@sheplu/command-audit");
        fs::create_dir_all(&install_dir).unwrap();
        let json = r#"{
            "name": "@sheplu/command-audit",
            "version": "1.0.0",
            "type": "command",
            "description": "Audit command",
            "license": "MIT"
        }"#;
        fs::write(install_dir.join("apkg.json"), json).unwrap();
        fs::write(
            install_dir.join("audit.md"),
            "Run a comprehensive audit.",
        )
        .unwrap();

        let report = run_setup(&SetupContext {
            project_root: tmp.path().to_path_buf(),
            install_dir,
            target: SetupTarget::Only(Tool::ClaudeCode),
        });

        assert_eq!(report.tools, vec![Tool::ClaudeCode]);
        assert_eq!(report.created.len(), 1);
        assert!(tmp
            .path()
            .join(".claude/commands/@sheplu/command-audit/audit.md")
            .exists());
    }

    #[test]
    fn test_run_setup_creates_configs() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
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

            assert_eq!(report.tools.len(), 1);
            assert_eq!(report.created.len(), 1);
            assert!(report.warnings.is_empty());
            assert!(tmp
                .path()
                .join(".claude/skills/@acme/code-reviewer/code-reviewer.md")
                .exists());
        });
    }

    #[test]
    fn test_run_setup_warns_on_bad_manifest() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::create_dir(tmp.path().join(".claude")).unwrap();

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
        });
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
            .join(".cursor/skills/@acme/code-reviewer/SKILL.md")
            .exists());
        assert!(!tmp
            .path()
            .join(".claude/skills/@acme/code-reviewer/code-reviewer.md")
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
            .join(".cursor/skills/@acme/code-reviewer.mdc")
            .exists());
        assert!(tmp
            .path()
            .join(".claude/skills/@acme/code-reviewer/code-reviewer.md")
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
            .join(".claude/skills/@acme/code-reviewer/code-reviewer.md")
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
            .join(".cursor/skills/@acme/code-reviewer/SKILL.md")
            .exists());
    }

    #[test]
    fn test_tool_display() {
        assert_eq!(Tool::Cursor.to_string(), "Cursor");
        assert_eq!(Tool::ClaudeCode.to_string(), "Claude Code");
        assert_eq!(Tool::Codex.to_string(), "Codex");
    }

    // TODO: re-enable when ready
    // #[test]
    // fn test_detect_tools_windsurf_only() {
    //     let tmp = TempDir::new().unwrap();
    //     fs::create_dir(tmp.path().join(".windsurf")).unwrap();
    //     let tools = detect_tools(tmp.path());
    //     assert_eq!(tools, vec![Tool::Windsurf]);
    // }

    // #[test]
    // fn test_detect_tools_kiro_only() {
    //     let tmp = TempDir::new().unwrap();
    //     fs::create_dir(tmp.path().join(".kiro")).unwrap();
    //     let tools = detect_tools(tmp.path());
    //     assert_eq!(tools, vec![Tool::Kiro]);
    // }

    #[test]
    fn test_detect_tools_codex_only() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".codex")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(tools, vec![Tool::Codex]);
    }

    #[test]
    fn test_detect_tools_all_three() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        fs::create_dir(tmp.path().join(".codex")).unwrap();
        let tools = detect_tools(tmp.path());
        assert_eq!(
            tools,
            vec![Tool::Cursor, Tool::ClaudeCode, Tool::Codex]
        );
    }

    #[test]
    fn test_tool_to_key() {
        assert_eq!(Tool::Cursor.to_key(), "cursor");
        assert_eq!(Tool::ClaudeCode.to_key(), "claude-code");
        assert_eq!(Tool::Codex.to_key(), "codex");
    }

    #[test]
    fn test_tool_to_key_roundtrip() {
        for tool in &[Tool::Cursor, Tool::ClaudeCode, Tool::Codex] {
            assert_eq!(Tool::from_key(tool.to_key()), Some(*tool));
        }
    }

    #[test]
    fn test_tool_from_key() {
        assert_eq!(Tool::from_key("cursor"), Some(Tool::Cursor));
        assert_eq!(Tool::from_key("claude-code"), Some(Tool::ClaudeCode));
        assert_eq!(Tool::from_key("codex"), Some(Tool::Codex));
        assert_eq!(Tool::from_key("unknown"), None);
        // TODO: re-enable when ready
        // assert_eq!(Tool::from_key("windsurf"), Some(Tool::Windsurf));
        // assert_eq!(Tool::from_key("kiro"), Some(Tool::Kiro));
    }

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nname: \"my-agent\"\ndescription: \"A test agent\"\nmodel: gpt-4o\n---\nBody here.\n";
        let pairs = parse_frontmatter(content);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], ("name".to_string(), "my-agent".to_string()));
        assert_eq!(pairs[1], ("description".to_string(), "A test agent".to_string()));
        assert_eq!(pairs[2], ("model".to_string(), "gpt-4o".to_string()));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just some text.";
        let pairs = parse_frontmatter(content);
        assert!(pairs.is_empty());
    }

    #[test]
    fn test_strip_frontmatter_basic() {
        let content = "---\nname: agent\n---\nBody content.\n";
        assert_eq!(strip_frontmatter(content), "Body content.\n");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "Just text.";
        assert_eq!(strip_frontmatter(content), "Just text.");
    }

    #[test]
    fn test_find_definition_files_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("agent.md"), "---\nname: a\n---\nBody").unwrap();
        // README.md with frontmatter is now included (frontmatter = definition)
        fs::write(tmp.path().join("README.md"), "---\ntitle: hi\n---\n").unwrap();
        fs::write(tmp.path().join("notes.md"), "No frontmatter").unwrap();
        let files = find_definition_files(tmp.path(), true);
        assert_eq!(files.len(), 2);
        let names: Vec<_> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"agent.md".to_string()));
        assert!(names.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_find_definition_files_excluded_name_without_frontmatter() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("changelog.md"), "Just a plain changelog").unwrap();
        fs::write(tmp.path().join("agent.md"), "---\nname: a\n---\nBody").unwrap();
        let files = find_definition_files(tmp.path(), false);
        // changelog.md without frontmatter is excluded, agent.md is included
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "agent.md");
    }

    #[test]
    fn test_find_definition_files_excluded_name_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("changelog.md"),
            "---\nname: changelog\n---\nGenerate a changelog.",
        )
        .unwrap();
        let files = find_definition_files(tmp.path(), false);
        // changelog.md WITH frontmatter is included
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "changelog.md");
    }

    #[test]
    fn test_find_definition_files_no_frontmatter_required() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("cmd.md"), "Run audit.").unwrap();
        fs::write(tmp.path().join("README.md"), "Project readme").unwrap();
        let files = find_definition_files(tmp.path(), false);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "cmd.md");
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_create_claude_md_symlink_creates() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::create_dir(tmp.path().join(".claude")).unwrap();
            fs::write(tmp.path().join("AGENTS.md"), "# Rules\n").unwrap();

            let tools = vec![Tool::ClaudeCode];
            let result = maybe_create_claude_md_symlink(tmp.path(), &tools);
            assert!(result.is_some());

            let claude_md = tmp.path().join("CLAUDE.md");
            assert!(claude_md.symlink_metadata().unwrap().file_type().is_symlink());
            assert_eq!(fs::read_link(&claude_md).unwrap(), Path::new("AGENTS.md"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_create_claude_md_symlink_skips_no_claude_tool() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::write(tmp.path().join("AGENTS.md"), "# Rules\n").unwrap();

            let tools = vec![Tool::Cursor];
            let result = maybe_create_claude_md_symlink(tmp.path(), &tools);
            assert!(result.is_none());
            assert!(!tmp.path().join("CLAUDE.md").exists());
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_create_claude_md_symlink_skips_no_agents_md() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::create_dir(tmp.path().join(".claude")).unwrap();

            let tools = vec![Tool::ClaudeCode];
            let result = maybe_create_claude_md_symlink(tmp.path(), &tools);
            assert!(result.is_none());
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_create_claude_md_symlink_skips_existing_file() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::create_dir(tmp.path().join(".claude")).unwrap();
            fs::write(tmp.path().join("AGENTS.md"), "# Rules\n").unwrap();
            fs::write(tmp.path().join("CLAUDE.md"), "existing content").unwrap();

            let tools = vec![Tool::ClaudeCode];
            let result = maybe_create_claude_md_symlink(tmp.path(), &tools);
            assert!(result.is_none());
            // Original file untouched
            assert_eq!(fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(), "existing content");
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_create_claude_md_symlink_skips_existing_symlink() {
        with_temp_home(|| {
            let tmp = TempDir::new().unwrap();
            fs::create_dir(tmp.path().join(".claude")).unwrap();
            fs::write(tmp.path().join("AGENTS.md"), "# Rules\n").unwrap();
            std::os::unix::fs::symlink("AGENTS.md", tmp.path().join("CLAUDE.md")).unwrap();

            let tools = vec![Tool::ClaudeCode];
            let result = maybe_create_claude_md_symlink(tmp.path(), &tools);
            assert!(result.is_none());
        });
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_cleanup_removes_dangling() {
        let tmp = TempDir::new().unwrap();
        // Create symlink to AGENTS.md, but don't create AGENTS.md
        std::os::unix::fs::symlink("AGENTS.md", tmp.path().join("CLAUDE.md")).unwrap();

        maybe_cleanup_claude_md_symlink(tmp.path());
        assert!(tmp.path().join("CLAUDE.md").symlink_metadata().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_cleanup_keeps_valid() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "# Rules\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", tmp.path().join("CLAUDE.md")).unwrap();

        maybe_cleanup_claude_md_symlink(tmp.path());
        // Symlink should still exist
        assert!(tmp.path().join("CLAUDE.md").symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_cleanup_ignores_regular_file() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("CLAUDE.md"), "regular file").unwrap();

        maybe_cleanup_claude_md_symlink(tmp.path());
        // Regular file should still exist
        assert!(tmp.path().join("CLAUDE.md").exists());
        assert_eq!(fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(), "regular file");
    }

    #[cfg(unix)]
    #[test]
    fn test_maybe_cleanup_ignores_other_target() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("OTHER.md"), "other").unwrap();
        std::os::unix::fs::symlink("OTHER.md", tmp.path().join("CLAUDE.md")).unwrap();

        maybe_cleanup_claude_md_symlink(tmp.path());
        // Symlink to a different target should be untouched
        assert!(tmp.path().join("CLAUDE.md").symlink_metadata().unwrap().file_type().is_symlink());
    }
}
