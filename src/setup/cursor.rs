use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::manifest::PackageType;

use super::{config_pkg_path, find_definition_files, package_short_name, resolve_system_prompt, strip_frontmatter, PackageInfo};

/// Generate and write Cursor config files for the given package.
///
/// Cursor uses four separate directories:
/// - `.cursor/skills/{stem}/SKILL.md` — skills (folder-based)
/// - `.cursor/agents/{stem}/`         — agents (.md with frontmatter)
/// - `.cursor/commands/{stem}/`       — commands (.md slash commands)
/// - `.cursor/rules/{stem}/`          — rules (.mdc with frontmatter)
///
/// If the package contains `.md` definition files, those are copied into the
/// appropriate subdirectory. Otherwise, a generated fallback file is written.
pub fn setup_cursor(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    match info.package_type {
        PackageType::Skill => setup_skill(project_root, install_dir, info),
        PackageType::Agent => setup_agent(project_root, install_dir, info),
        PackageType::Command => setup_command(project_root, install_dir, info),
        PackageType::Rule => setup_rule(project_root, install_dir, info),
        PackageType::Project => Ok(Vec::new()),
    }
}

// ---------------------------------------------------------------------------
// Skills — .cursor/skills/{stem}/SKILL.md  (folder-based)
// ---------------------------------------------------------------------------

fn setup_skill(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let pkg_path = config_pkg_path(&info.name);
    let skill_dir = project_root
        .join(".cursor")
        .join("skills")
        .join(&pkg_path);
    fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create .cursor/skills/{}/: {e}", pkg_path.display()))?;

    let md_files = find_definition_files(install_dir, true);
    let skill_path = skill_dir.join("SKILL.md");

    if md_files.is_empty() {
        // No definition files — generate a fallback SKILL.md.
        let content = generate_skill_md(install_dir, info, &info.name);
        fs::write(&skill_path, &content)
            .map_err(|e| format!("Failed to write {}: {e}", skill_path.display()))?;
        return Ok(vec![skill_path]);
    }

    // Use the first definition file's body as SKILL.md content,
    // wrapped with Cursor-specific frontmatter.
    let primary = &md_files[0];
    let original = fs::read_to_string(primary)
        .map_err(|e| format!("Failed to read {}: {e}", primary.display()))?;
    let content = wrap_skill_frontmatter(info, &info.name, &original);
    fs::write(&skill_path, &content)
        .map_err(|e| format!("Failed to write {}: {e}", skill_path.display()))?;
    let mut created = vec![skill_path];

    // Additional definition files go to references/.
    if md_files.len() > 1 {
        let refs_dir = skill_dir.join("references");
        fs::create_dir_all(&refs_dir)
            .map_err(|e| format!("Failed to create references/: {e}"))?;
        for src in &md_files[1..] {
            let file_name = src
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dest = refs_dir.join(&file_name);
            fs::copy(src, &dest).map_err(|e| {
                format!("Failed to copy {} to {}: {e}", src.display(), dest.display())
            })?;
            created.push(dest);
        }
    }

    Ok(created)
}

fn generate_skill_md(install_dir: &Path, info: &PackageInfo, frontmatter_name: &str) -> String {
    let mut out = String::new();
    let desc = info.description.replace('"', "'");

    // Frontmatter
    let _ = write!(
        out,
        "---\nname: \"{frontmatter_name}\"\ndescription: \"{name} — {desc}\"\n---\n\n",
        name = info.name,
    );

    // Body
    let _ = write!(out, "# {} (skill)\n\n{}\n", info.name, info.description);

    if let Some(skill) = &info.skill {
        if !skill.capabilities.is_empty() {
            let _ = writeln!(out, "\nCapabilities: {}", skill.capabilities.join(", "));
        }
    }

    if let Some(main) = &info.main {
        let _ = writeln!(out, "\nEntry point: {}/{main}", install_dir.display());
    }

    if let Some(skill) = &info.skill {
        if !skill.capabilities.is_empty() {
            let caps = skill.capabilities.join(" or ");
            let _ = write!(out, "\nUse this skill when you need {caps} capabilities.\n");
        }
    }

    let _ = write!(
        out,
        "\nThis package is installed at `{}/`.\n",
        install_dir.display()
    );

    out
}

/// Wrap a definition file's content with Cursor skill frontmatter.
/// Strips any existing frontmatter before prepending the Cursor-specific one.
fn wrap_skill_frontmatter(info: &PackageInfo, frontmatter_name: &str, content: &str) -> String {
    let desc = info.description.replace('"', "'");
    let header = format!(
        "---\nname: \"{frontmatter_name}\"\ndescription: \"{name} — {desc}\"\n---\n\n",
        name = info.name,
    );

    let body = strip_frontmatter(content);
    format!("{header}{body}")
}


// ---------------------------------------------------------------------------
// Agents — .cursor/agents/{stem}/ (.md with name/description/model frontmatter)
// ---------------------------------------------------------------------------

fn setup_agent(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let pkg_path = config_pkg_path(&info.name);
    let agent_dir = project_root
        .join(".cursor")
        .join("agents")
        .join(&pkg_path);
    fs::create_dir_all(&agent_dir)
        .map_err(|e| format!("Failed to create .cursor/agents/{}/: {e}", pkg_path.display()))?;

    let md_files = find_definition_files(install_dir, true);

    if md_files.is_empty() {
        let content = generate_agent_md(install_dir, info, &info.name);
        let short = package_short_name(&info.name);
        let path = agent_dir.join(format!("{short}.md"));
        fs::write(&path, &content)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        return Ok(vec![path]);
    }

    let mut created = Vec::new();
    for src in &md_files {
        let file_name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dest = agent_dir.join(&file_name);
        fs::copy(src, &dest).map_err(|e| {
            format!("Failed to copy {} to {}: {e}", src.display(), dest.display())
        })?;
        created.push(dest);
    }

    Ok(created)
}

fn generate_agent_md(install_dir: &Path, info: &PackageInfo, frontmatter_name: &str) -> String {
    let mut out = String::new();
    let desc = info.description.replace('"', "'");

    // Cursor agent frontmatter
    let _ = write!(
        out,
        "---\nname: \"{frontmatter_name}\"\ndescription: \"{name} — {desc}\"\nmodel: inherit\n---\n\n",
        name = info.name,
    );

    let _ = write!(out, "# {} (agent)\n\n{}\n", info.name, info.description);

    if let Some(agent) = &info.agent {
        if let Some(prompt_val) = &agent.system_prompt {
            let resolved = resolve_system_prompt(prompt_val, install_dir);
            let _ = write!(out, "\n## System Prompt\n\n{resolved}");
            if !resolved.ends_with('\n') {
                out.push('\n');
            }
        }

        if !agent.tools.is_empty() {
            out.push_str("\n## Available Tools\n\n");
            for tool in &agent.tools {
                let req = if tool.required {
                    "required"
                } else {
                    "optional"
                };
                let _ = writeln!(out, "- {} (`{}`) — {req}", tool.name, tool.package);
            }
        }

        if !agent.model_preference.is_empty() {
            let _ = write!(
                out,
                "\nPreferred models: {}\n",
                agent.model_preference.join(", ")
            );
        }
    }

    if let Some(main) = &info.main {
        let _ = writeln!(out, "\nEntry point: {}/{main}", install_dir.display());
    }

    let _ = write!(
        out,
        "\nThis package is installed at `{}/`.\n",
        install_dir.display()
    );

    out
}

// ---------------------------------------------------------------------------
// Commands — .cursor/commands/{stem}/ (.md slash commands)
// ---------------------------------------------------------------------------

fn setup_command(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let pkg_path = config_pkg_path(&info.name);
    let cmd_dir = project_root
        .join(".cursor")
        .join("commands")
        .join(&pkg_path);
    fs::create_dir_all(&cmd_dir)
        .map_err(|e| format!("Failed to create .cursor/commands/{}/: {e}", pkg_path.display()))?;

    let md_files = find_definition_files(install_dir, false);

    if md_files.is_empty() {
        let content = generate_command_md(install_dir, info);
        let short = package_short_name(&info.name);
        let path = cmd_dir.join(format!("{short}.md"));
        fs::write(&path, &content)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        return Ok(vec![path]);
    }

    let mut created = Vec::new();
    for src in &md_files {
        let file_name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dest = cmd_dir.join(&file_name);
        fs::copy(src, &dest).map_err(|e| {
            format!("Failed to copy {} to {}: {e}", src.display(), dest.display())
        })?;
        created.push(dest);
    }

    Ok(created)
}

fn generate_command_md(install_dir: &Path, info: &PackageInfo) -> String {
    let mut out = String::new();

    let _ = write!(out, "# {}\n\n{}\n", info.name, info.description);
    out.push_str("\nUse this command as a slash command in Cursor.\n");

    if let Some(main) = &info.main {
        let _ = writeln!(out, "\nEntry point: {}/{main}", install_dir.display());
    }

    out.push_str("\nInvoke this command with the corresponding slash command.\n");

    let _ = write!(
        out,
        "\nThis package is installed at `{}/`.\n",
        install_dir.display()
    );

    out
}

// ---------------------------------------------------------------------------
// Rules — .cursor/rules/{stem}/ (.mdc with description/alwaysApply frontmatter)
// ---------------------------------------------------------------------------

fn setup_rule(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let pkg_path = config_pkg_path(&info.name);
    let rule_dir = project_root
        .join(".cursor")
        .join("rules")
        .join(&pkg_path);
    fs::create_dir_all(&rule_dir)
        .map_err(|e| format!("Failed to create .cursor/rules/{}/: {e}", pkg_path.display()))?;

    let md_files = find_definition_files(install_dir, false);

    if md_files.is_empty() {
        let content = generate_rule_mdc(info);
        let short = package_short_name(&info.name);
        let path = rule_dir.join(format!("{short}.mdc"));
        fs::write(&path, &content)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        return Ok(vec![path]);
    }

    let mut created = Vec::new();
    for src in &md_files {
        let file_name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Convert .md to .mdc for Cursor rules
        let mdc_name = if file_name.ends_with(".md") {
            format!("{}c", file_name)
        } else {
            file_name
        };
        let dest = rule_dir.join(&mdc_name);

        // Read the original content and prepend Cursor rule frontmatter
        let original = fs::read_to_string(src)
            .map_err(|e| format!("Failed to read {}: {e}", src.display()))?;
        let content = prepend_rule_frontmatter(info, &original);
        fs::write(&dest, content)
            .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        created.push(dest);
    }

    Ok(created)
}

fn generate_rule_mdc(info: &PackageInfo) -> String {
    let mut out = String::new();
    let desc = info.description.replace('"', "'");

    let _ = write!(
        out,
        "---\ndescription: \"{name} — {desc}\"\nalwaysApply: true\n---\n\n",
        name = info.name,
    );

    let _ = write!(out, "# {}\n\n{}\n", info.name, info.description);
    out.push_str("\nThis rule is automatically active in the project.\n");

    out
}

/// Prepend Cursor rule frontmatter to content that may or may not already have
/// frontmatter. Strips any existing frontmatter before prepending.
fn prepend_rule_frontmatter(info: &PackageInfo, content: &str) -> String {
    let desc = info.description.replace('"', "'");
    let header = format!(
        "---\ndescription: \"{name} — {desc}\"\nalwaysApply: true\n---\n\n",
        name = info.name,
    );

    let body = strip_frontmatter(content);
    format!("{header}{body}")
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{AgentInfo, AgentTool, SkillInfo};
    use tempfile::TempDir;

    fn skill_info() -> PackageInfo {
        PackageInfo {
            name: "@acme/code-reviewer".to_string(),
            package_type: PackageType::Skill,
            description: "AI-powered code review".to_string(),
            main: Some("src/index.ts".to_string()),
            skill: Some(SkillInfo {
                capabilities: vec!["code-review".to_string(), "bug-detection".to_string()],
                model_compatibility: None,
                max_tokens: None,
                streaming: None,
            }),
            agent: None,
        }
    }

    fn agent_info() -> PackageInfo {
        PackageInfo {
            name: "@acme/research-agent".to_string(),
            package_type: PackageType::Agent,
            description: "Autonomous research agent".to_string(),
            main: Some("src/agent.ts".to_string()),
            skill: None,
            agent: Some(AgentInfo {
                tools: vec![
                    AgentTool {
                        name: "web-search".to_string(),
                        package: "@acme/web-search".to_string(),
                        required: true,
                    },
                    AgentTool {
                        name: "formatter".to_string(),
                        package: "@acme/fmt".to_string(),
                        required: false,
                    },
                ],
                system_prompt: Some("You are a research assistant.".to_string()),
                model_preference: vec!["claude-sonnet-4-6".to_string(), "gpt-4o".to_string()],
                memory: None,
                orchestration: None,
            }),
        }
    }

    fn command_info() -> PackageInfo {
        PackageInfo {
            name: "@sheplu/command-audit".to_string(),
            package_type: PackageType::Command,
            description: "Audit command".to_string(),
            main: None,
            skill: None,
            agent: None,
        }
    }

    fn rule_info() -> PackageInfo {
        PackageInfo {
            name: "@acme/my-rule".to_string(),
            package_type: PackageType::Rule,
            description: "Coding standards rule".to_string(),
            main: None,
            skill: None,
            agent: None,
        }
    }

    // --- Skill tests ---

    #[test]
    fn test_generate_skill_md() {
        let tmp = TempDir::new().unwrap();
        let content = generate_skill_md(tmp.path(), &skill_info(), "@acme/code-reviewer");
        assert!(content.contains("name: \"@acme/code-reviewer\""));
        assert!(content.contains("description: \"@acme/code-reviewer"));
        assert!(content.contains("# @acme/code-reviewer (skill)"));
        assert!(content.contains("Capabilities: code-review, bug-detection"));
        assert!(content.contains("src/index.ts"));
    }

    #[test]
    fn test_setup_cursor_skill_creates_skill_dir() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert_eq!(paths[0].file_name().unwrap(), "SKILL.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("skills/@acme/code-reviewer"));
    }

    #[test]
    fn test_setup_cursor_skill_uses_definition_as_skill_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("review.md"),
            "---\nname: review\ntools: Read, Grep\n---\nReview instructions.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &skill_info()).unwrap();
        // Single SKILL.md — no references/ when only one definition file
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "SKILL.md");
        let content = fs::read_to_string(&paths[0]).unwrap();
        // Has Cursor frontmatter
        assert!(content.contains("name: \"@acme/code-reviewer\""));
        assert!(content.contains("description:"));
        // Has the definition body (original frontmatter stripped)
        assert!(content.contains("Review instructions."));
        assert!(!content.contains("tools: Read, Grep"));
    }

    #[test]
    fn test_setup_cursor_skill_extra_defs_go_to_references() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("main.md"),
            "---\nname: main\n---\nMain instructions.",
        )
        .unwrap();
        fs::write(
            install_dir.join("extra.md"),
            "---\nname: extra\n---\nExtra docs.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &skill_info()).unwrap();
        // SKILL.md + one file in references/
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].file_name().unwrap(), "SKILL.md");
        assert!(paths[1].parent().unwrap().ends_with("references"));
    }

    // --- Agent tests ---

    #[test]
    fn test_generate_agent_md() {
        let tmp = TempDir::new().unwrap();
        let content = generate_agent_md(tmp.path(), &agent_info(), "@acme/research-agent");
        assert!(content.contains("name: \"@acme/research-agent\""));
        assert!(content.contains("model: inherit"));
        assert!(content.contains("# @acme/research-agent (agent)"));
        assert!(content.contains("## System Prompt"));
        assert!(content.contains("You are a research assistant."));
        assert!(content.contains("web-search (`@acme/web-search`) — required"));
        assert!(content.contains("formatter (`@acme/fmt`) — optional"));
        assert!(content.contains("claude-sonnet-4-6, gpt-4o"));
    }

    #[test]
    fn test_setup_cursor_agent_creates_dir() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert_eq!(
            paths[0].file_name().unwrap(),
            "research-agent.md"
        );
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("agents/@acme/research-agent"));
    }

    #[test]
    fn test_setup_cursor_agent_copies_definition_files() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("agent.md"),
            "---\nname: agent\n---\nYou are a reviewer.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "agent.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("agents/@acme/research-agent"));
    }

    #[test]
    fn test_setup_cursor_agent_with_file_prompt() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();
        let prompts_dir = install_dir.join("prompts");
        fs::create_dir(&prompts_dir).unwrap();
        fs::write(
            prompts_dir.join("system.md"),
            "You are a specialized agent.",
        )
        .unwrap();

        let mut info = agent_info();
        if let Some(agent) = &mut info.agent {
            agent.system_prompt = Some("prompts/system.md".to_string());
        }

        let paths = setup_cursor(tmp.path(), &install_dir, &info).unwrap();
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("You are a specialized agent."));
    }

    // --- Command tests ---

    #[test]
    fn test_setup_cursor_command_copies_plain_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/command-audit");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("audit.md"),
            "Run a comprehensive audit of the project.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &command_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "audit.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("commands/@sheplu/command-audit"));
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("Run a comprehensive audit"));
    }

    #[test]
    fn test_setup_cursor_command_fallback() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/command-audit");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &command_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].file_name().unwrap(),
            "command-audit.md"
        );
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("# @sheplu/command-audit"));
        assert!(content.contains("slash command"));
    }

    // --- Rule tests ---

    #[test]
    fn test_generate_rule_mdc() {
        let content = generate_rule_mdc(&rule_info());
        assert!(content.contains("description: \"@acme/my-rule"));
        assert!(content.contains("alwaysApply: true"));
        assert!(content.contains("# @acme/my-rule"));
    }

    #[test]
    fn test_setup_cursor_rule_creates_mdc() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/my-rule");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &rule_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "my-rule.mdc");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("rules/@acme/my-rule"));
    }

    #[test]
    fn test_setup_cursor_rule_copies_and_converts_to_mdc() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/my-rule");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("lint.md"),
            "Always use semicolons in TypeScript.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &rule_info()).unwrap();
        assert_eq!(paths.len(), 1);
        // .md -> .mdc
        assert_eq!(paths[0].file_name().unwrap(), "lint.mdc");
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("alwaysApply: true"));
        assert!(content.contains("Always use semicolons"));
    }

    #[test]
    fn test_setup_cursor_rule_strips_existing_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/my-rule");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("style.md"),
            "---\nold: frontmatter\n---\nUse 2-space indentation.",
        )
        .unwrap();

        let paths = setup_cursor(tmp.path(), &install_dir, &rule_info()).unwrap();
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("alwaysApply: true"));
        assert!(content.contains("Use 2-space indentation."));
        assert!(!content.contains("old: frontmatter"));
    }

    // --- find_definition_files tests ---

    #[test]
    fn test_find_definition_files_with_frontmatter() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("agent.md"),
            "---\nname: agent\n---\nContent",
        )
        .unwrap();
        // README.md without frontmatter is excluded
        fs::write(tmp.path().join("README.md"), "Project readme").unwrap();
        fs::write(tmp.path().join("notes.md"), "No frontmatter").unwrap();

        let files = find_definition_files(tmp.path(), true);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "agent.md");
    }

    #[test]
    fn test_find_definition_files_no_frontmatter_required() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("audit.md"), "Run audit...").unwrap();
        fs::write(tmp.path().join("README.md"), "Project readme").unwrap();

        let files = find_definition_files(tmp.path(), false);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "audit.md");
    }

    #[test]
    fn test_prepend_rule_frontmatter_plain_content() {
        let info = rule_info();
        let result = prepend_rule_frontmatter(&info, "Some rule content.");
        assert!(result.starts_with("---\n"));
        assert!(result.contains("alwaysApply: true"));
        assert!(result.contains("Some rule content."));
    }

    #[test]
    fn test_prepend_rule_frontmatter_strips_existing() {
        let info = rule_info();
        let result =
            prepend_rule_frontmatter(&info, "---\nold: value\n---\nActual rule content.");
        assert!(result.contains("alwaysApply: true"));
        assert!(result.contains("Actual rule content."));
        assert!(!result.contains("old: value"));
    }
}
