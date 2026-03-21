use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::manifest::PackageType;

use super::{config_file_stem, resolve_system_prompt, PackageInfo};

/// Generate and write a Cursor rule file for the given package.
pub fn setup_cursor(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<PathBuf, String> {
    let content = generate_cursor_rule(install_dir, info);
    let stem = config_file_stem(&info.name);
    let type_dir = info.package_type.dir_name();
    let target_dir = project_root.join(".cursor").join(type_dir);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed to create .cursor/{type_dir}/: {e}"))?;

    let path = target_dir.join(format!("{stem}.mdc"));
    fs::write(&path, content)
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(path)
}

fn generate_cursor_rule(install_dir: &Path, info: &PackageInfo) -> String {
    let mut out = String::new();

    // Frontmatter
    let desc = info.description.replace('"', "'");
    let _ = write!(
        out,
        "---\ndescription: \"{name} — {desc}\"\nalwaysApply: false\n---\n\n",
        name = info.name,
    );

    // Heading + description
    let type_label = &info.package_type;
    let _ = write!(out, "# {} ({type_label})\n\n{}\n", info.name, info.description);

    match info.package_type {
        PackageType::Skill => write_skill_section(&mut out, info),
        PackageType::Agent => write_agent_section(&mut out, install_dir, info),
        _ => {}
    }

    // Entry point
    if let Some(main) = &info.main {
        let _ = write!(out, "\n## Entry Point\n\n`{main}`\n");
    }

    // Install location
    let _ = write!(out, "\nInstalled at: `{}/`\n", install_dir.display());

    out
}

fn write_skill_section(out: &mut String, info: &PackageInfo) {
    if let Some(skill) = &info.skill {
        if !skill.capabilities.is_empty() {
            out.push_str("\n## Capabilities\n\n");
            for cap in &skill.capabilities {
                let _ = writeln!(out, "- {cap}");
            }
        }
    }
}

fn write_agent_section(out: &mut String, install_dir: &Path, info: &PackageInfo) {
    if let Some(agent) = &info.agent {
        if let Some(prompt_val) = &agent.system_prompt {
            let resolved = resolve_system_prompt(prompt_val, install_dir);
            out.push_str("\n## System Prompt\n\n");
            out.push_str(&resolved);
            if !resolved.ends_with('\n') {
                out.push('\n');
            }
        }

        if !agent.tools.is_empty() {
            out.push_str("\n## Tools\n\n");
            for tool in &agent.tools {
                let req = if tool.required { "required" } else { "optional" };
                let _ = writeln!(out, "- **{}** (`{}`) — {req}", tool.name, tool.package);
            }
        }

        if !agent.model_preference.is_empty() {
            out.push_str("\n## Model Preference\n\n");
            out.push_str(&agent.model_preference.join(", "));
            out.push('\n');
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
                model_preference: vec!["claude-sonnet-4-6".to_string(), "gpt-4o".to_string()],
                memory: None,
                orchestration: None,
            }),
        }
    }

    #[test]
    fn test_generate_cursor_rule_skill() {
        let tmp = TempDir::new().unwrap();
        let content = generate_cursor_rule(tmp.path(), &skill_info());
        assert!(content.contains("description: \"@acme/code-reviewer"));
        assert!(content.contains("alwaysApply: false"));
        assert!(content.contains("# @acme/code-reviewer (skill)"));
        assert!(content.contains("- code-review"));
        assert!(content.contains("- bug-detection"));
        assert!(content.contains("`src/index.ts`"));
    }

    #[test]
    fn test_generate_cursor_rule_agent() {
        let tmp = TempDir::new().unwrap();
        let content = generate_cursor_rule(tmp.path(), &agent_info());
        assert!(content.contains("# @acme/research-agent (agent)"));
        assert!(content.contains("## System Prompt"));
        assert!(content.contains("You are a research assistant."));
        assert!(content.contains("**web-search** (`@acme/web-search`) — required"));
        assert!(content.contains("**formatter** (`@acme/fmt`) — optional"));
        assert!(content.contains("claude-sonnet-4-6, gpt-4o"));
    }

    #[test]
    fn test_setup_cursor_creates_file() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        let install_dir = tmp.path().join("qpm_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        let path = setup_cursor(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "acme--code-reviewer.mdc");
    }

    #[test]
    fn test_generate_cursor_rule_agent_with_file_prompt() {
        let tmp = TempDir::new().unwrap();
        let prompts_dir = tmp.path().join("prompts");
        fs::create_dir(&prompts_dir).unwrap();
        fs::write(prompts_dir.join("system.md"), "You are a specialized agent.").unwrap();

        let mut info = agent_info();
        if let Some(agent) = &mut info.agent {
            agent.system_prompt = Some("prompts/system.md".to_string());
        }

        let content = generate_cursor_rule(tmp.path(), &info);
        assert!(content.contains("You are a specialized agent."));
    }
}
