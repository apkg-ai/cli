use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::manifest::PackageType;

use super::{config_file_stem, resolve_system_prompt, PackageInfo};

/// Generate and write a Claude Code command file for the given package.
pub fn setup_claude(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<PathBuf, String> {
    let content = generate_claude_command(install_dir, info);
    let stem = config_file_stem(&info.name);
    let type_dir = info.package_type.dir_name();
    let target_dir = project_root.join(".claude").join(type_dir);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create .claude/{type_dir}/: {e}"))?;

    let path = target_dir.join(format!("{stem}.md"));
    fs::write(&path, content)
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(path)
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
        _ => {}
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
                    let _ = write!(
                        out,
                        "\nUse this skill when you need {caps} capabilities.\n"
                    );
                }
            }
        }
        PackageType::Agent => {
            out.push_str(
                "\nUse this agent for tasks described in its system prompt and tool set.\n",
            );
        }
        _ => {}
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
                let req = if tool.required { "required" } else { "optional" };
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
        assert!(content.contains(
            "Use this skill when you need code-review or bug-detection capabilities."
        ));
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
    fn test_setup_claude_creates_file() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        let install_dir = tmp.path().join("qpm_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        let path = setup_claude(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "acme--code-reviewer.md");
    }

    #[test]
    fn test_generate_claude_command_agent_with_file_prompt() {
        let tmp = TempDir::new().unwrap();
        let prompts_dir = tmp.path().join("prompts");
        fs::create_dir(&prompts_dir).unwrap();
        fs::write(prompts_dir.join("system.md"), "You are a specialized agent.").unwrap();

        let mut info = agent_info();
        if let Some(agent) = &mut info.agent {
            agent.system_prompt = Some("prompts/system.md".to_string());
        }

        let content = generate_claude_command(tmp.path(), &info);
        assert!(content.contains("You are a specialized agent."));
    }
}
