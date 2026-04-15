use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::manifest::PackageType;

use super::{config_pkg_path, find_definition_files, package_short_name, resolve_system_prompt, PackageInfo};

/// Generate and write Claude Code config files for the given package.
///
/// If the package contains `.md` files with YAML frontmatter (actual agent/skill
/// definitions), those are copied directly into `.claude/{type}/`. Otherwise, a
/// generated summary file is written as a fallback.
pub fn setup_claude(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let type_dir = info.package_type.dir_name();
    let target_dir = project_root.join(".claude").join(type_dir);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create .claude/{type_dir}/: {e}"))?;

    let require_frontmatter = matches!(
        info.package_type,
        PackageType::Skill | PackageType::Agent
    );
    let md_files = find_definition_files(install_dir, require_frontmatter);

    let pkg_path = config_pkg_path(&info.name);
    let sub_dir = target_dir.join(&pkg_path);
    fs::create_dir_all(&sub_dir)
        .map_err(|e| format!("Failed to create .claude/{type_dir}/{}/: {e}", pkg_path.display()))?;

    if md_files.is_empty() {
        // Fallback: generate a summary file (legacy behaviour for packages
        // that don't ship their own .md definitions).
        let content = generate_claude_command(install_dir, info);
        let short = package_short_name(&info.name);
        let path = sub_dir.join(format!("{short}.md"));
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
        let dest = sub_dir.join(&file_name);
        fs::copy(src, &dest).map_err(|e| {
            format!(
                "Failed to copy {} to {}: {e}",
                src.display(),
                dest.display()
            )
        })?;
        created.push(dest);
    }

    Ok(created)
}

fn generate_claude_command(install_dir: &Path, info: &PackageInfo) -> String {
    let mut out = String::new();

    // Heading + description + type
    let _ = write!(
        out,
        "# {}\n\n{}\n\nType: {}\n",
        info.name, info.description, info.package_type
    );

    match info.package_type {
        PackageType::Skill => write_skill_section(&mut out, info),
        PackageType::Agent => write_agent_section(&mut out, install_dir, info),
        PackageType::Command => {
            out.push_str("\nUse this command as a slash command in Claude Code.\n");
        }
        PackageType::Rule => {
            out.push_str("\nThis rule is applied automatically by Claude Code.\n");
        }
        PackageType::Project => {}
    }

    // Entry point
    if let Some(main) = &info.main {
        let _ = writeln!(out, "Entry point: {}/{main}", install_dir.display());
    }

    // Usage hint
    match info.package_type {
        PackageType::Skill => {
            if let Some(skill) = &info.skill {
                if !skill.capabilities.is_empty() {
                    let caps = skill.capabilities.join(" or ");
                    let _ = write!(out, "\nUse this skill when you need {caps} capabilities.\n");
                }
            }
        }
        PackageType::Agent => {
            out.push_str(
                "\nUse this agent for tasks described in its system prompt and tool set.\n",
            );
        }
        PackageType::Command => {
            out.push_str("\nInvoke this command with the corresponding slash command.\n");
        }
        PackageType::Rule => {
            out.push_str("\nThis rule is automatically active in the project.\n");
        }
        PackageType::Project => {}
    }

    // Install location
    let _ = write!(
        out,
        "\nThis package is installed at `{}/`.\n",
        install_dir.display()
    );

    out
}

fn write_skill_section(out: &mut String, info: &PackageInfo) {
    if let Some(skill) = &info.skill {
        if !skill.capabilities.is_empty() {
            let _ = writeln!(out, "Capabilities: {}", skill.capabilities.join(", "));
        }
    }
}

fn write_agent_section(out: &mut String, install_dir: &Path, info: &PackageInfo) {
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
            platform: Vec::new(),
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
            platform: Vec::new(),
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
                model_preference: vec!["claude-sonnet-4-6".to_string()],
                memory: None,
                orchestration: None,
            }),
        }
    }

    #[test]
    fn test_generate_claude_command_skill() {
        let tmp = TempDir::new().unwrap();
        let content = generate_claude_command(tmp.path(), &skill_info());
        assert!(content.contains("# @acme/code-reviewer"));
        assert!(content.contains("Type: skill"));
        assert!(content.contains("Capabilities: code-review, bug-detection"));
        assert!(content.contains("src/index.ts"));
        assert!(content
            .contains("Use this skill when you need code-review or bug-detection capabilities."));
    }

    #[test]
    fn test_generate_claude_command_agent() {
        let tmp = TempDir::new().unwrap();
        let content = generate_claude_command(tmp.path(), &agent_info());
        assert!(content.contains("# @acme/research-agent"));
        assert!(content.contains("Type: agent"));
        assert!(content.contains("## System Prompt"));
        assert!(content.contains("You are a research assistant."));
        assert!(content.contains("web-search (`@acme/web-search`) — required"));
        assert!(content.contains("formatter (`@acme/fmt`) — optional"));
        assert!(content.contains("Preferred models: claude-sonnet-4-6"));
    }

    #[test]
    fn test_setup_claude_fallback_when_no_md_files() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_claude(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert_eq!(paths[0].file_name().unwrap(), "code-reviewer.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("skills/@acme/code-reviewer"));
    }

    #[test]
    fn test_setup_claude_copies_frontmatter_md_files() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("review-agent.md"),
            "---\nname: review-agent\ntools: Read, Grep\n---\nYou are a reviewer.",
        )
        .unwrap();

        let paths = setup_claude(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);
        let dest = &paths[0];
        assert!(dest.exists());
        assert_eq!(dest.file_name().unwrap(), "review-agent.md");
        assert!(dest
            .parent()
            .unwrap()
            .ends_with("agents/@acme/research-agent"));
        let content = fs::read_to_string(dest).unwrap();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("You are a reviewer."));
    }

    #[test]
    fn test_setup_claude_ignores_readme_and_non_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        // Should be ignored: no frontmatter
        fs::write(install_dir.join("notes.md"), "Just some notes.").unwrap();
        // Should be ignored: excluded name without frontmatter
        fs::write(install_dir.join("README.md"), "Project readme").unwrap();
        // Should be copied: valid definition
        fs::write(
            install_dir.join("agent.md"),
            "---\nname: agent\n---\nSystem prompt here.",
        )
        .unwrap();

        let paths = setup_claude(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "agent.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("agents/@acme/research-agent"));
    }

    #[test]
    fn test_find_definition_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("agent.md"),
            "---\nname: agent\n---\nContent",
        )
        .unwrap();
        // README.md without frontmatter is excluded
        fs::write(tmp.path().join("README.md"), "Project readme").unwrap();
        fs::write(tmp.path().join("notes.md"), "No frontmatter").unwrap();
        fs::write(tmp.path().join("other.txt"), "---\nfake\n---\n").unwrap();

        let files = find_definition_files(tmp.path(), true);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "agent.md");
    }

    #[test]
    fn test_find_definition_files_no_frontmatter_required() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("audit.md"), "Run a comprehensive audit...").unwrap();
        fs::write(tmp.path().join("README.md"), "Project readme").unwrap();
        fs::write(tmp.path().join("other.txt"), "not markdown").unwrap();

        let files = find_definition_files(tmp.path(), false);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().unwrap(), "audit.md");
    }

    #[test]
    fn test_setup_claude_command_copies_plain_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/command-audit");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("audit.md"),
            "Run a comprehensive audit of the project.",
        )
        .unwrap();

        let info = PackageInfo {
            name: "@sheplu/command-audit".to_string(),
            package_type: PackageType::Command,
            description: "Audit command".to_string(),
            main: None,
            platform: Vec::new(),
            skill: None,
            agent: None,
        };

        let paths = setup_claude(tmp.path(), &install_dir, &info).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert_eq!(paths[0].file_name().unwrap(), "audit.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("commands/@sheplu/command-audit"));
        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("Run a comprehensive audit"));
    }

    #[test]
    fn test_setup_claude_command_fallback_when_no_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/my-command");
        fs::create_dir_all(&install_dir).unwrap();

        let info = PackageInfo {
            name: "my-command".to_string(),
            package_type: PackageType::Command,
            description: "A useful command".to_string(),
            main: None,
            platform: Vec::new(),
            skill: None,
            agent: None,
        };

        let paths = setup_claude(tmp.path(), &install_dir, &info).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].exists());
        assert_eq!(paths[0].file_name().unwrap(), "my-command.md");
        assert!(paths[0]
            .parent()
            .unwrap()
            .ends_with("commands/my-command"));
    }

    #[test]
    fn test_generate_claude_command_agent_with_file_prompt() {
        let tmp = TempDir::new().unwrap();
        let prompts_dir = tmp.path().join("prompts");
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

        let content = generate_claude_command(tmp.path(), &info);
        assert!(content.contains("You are a specialized agent."));
    }
}
