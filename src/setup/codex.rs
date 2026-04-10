use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::manifest::PackageType;

use super::{
    config_pkg_path, find_definition_files, package_short_name, parse_frontmatter,
    resolve_system_prompt, strip_frontmatter, PackageInfo,
};

/// Generate and write Codex agent TOML files for the given package.
///
/// If the package contains `.md` files with YAML frontmatter (actual agent/skill
/// definitions), those are transformed into `.toml` files. Otherwise, a generated
/// fallback TOML is written.
pub fn setup_codex(
    project_root: &Path,
    install_dir: &Path,
    info: &PackageInfo,
) -> Result<Vec<PathBuf>, String> {
    let pkg_path = config_pkg_path(&info.name);
    let type_dir = info.package_type.dir_name();
    let target_dir = project_root
        .join(".codex")
        .join(type_dir)
        .join(&pkg_path);
    fs::create_dir_all(&target_dir).map_err(|e| {
        format!(
            "Failed to create .codex/{type_dir}/{}/: {e}",
            pkg_path.display()
        )
    })?;

    let md_files = find_definition_files(install_dir, true);

    // Skills stay as markdown for Codex — copy them directly instead of
    // transforming to TOML.
    if info.package_type == PackageType::Skill {
        return setup_codex_skill(&target_dir, install_dir, info, &md_files);
    }

    if md_files.is_empty() {
        let content = generate_fallback_toml(install_dir, info);
        let short = package_short_name(&info.name);
        let path = target_dir.join(format!("{short}.toml"));
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        return Ok(vec![path]);
    }

    let mut created = Vec::new();

    for src in &md_files {
        let raw = fs::read_to_string(src)
            .map_err(|e| format!("Failed to read {}: {e}", src.display()))?;

        let toml = md_to_toml(&raw, &info.name, &info.description);

        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dest = target_dir.join(format!("{stem}.toml"));
        fs::write(&dest, &toml)
            .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        created.push(dest);
    }

    Ok(created)
}

/// Set up a skill for Codex by copying markdown files as-is (no TOML
/// transformation). When no `.md` definition files exist, a fallback `.md`
/// file is generated from the package info.
fn setup_codex_skill(
    target_dir: &Path,
    install_dir: &Path,
    info: &PackageInfo,
    md_files: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    if md_files.is_empty() {
        let short = package_short_name(&info.name);
        let dest = target_dir.join(format!("{short}.md"));
        let content = generate_fallback_skill_md(install_dir, info);
        fs::write(&dest, content)
            .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        return Ok(vec![dest]);
    }

    let mut created = Vec::new();
    for src in md_files {
        let file_name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dest = target_dir.join(&file_name);
        fs::copy(src, &dest)
            .map_err(|e| format!("Failed to copy {} to {}: {e}", src.display(), dest.display()))?;
        created.push(dest);
    }
    Ok(created)
}

/// Generate a fallback markdown file for a skill when no `.md` definitions
/// are present.
fn generate_fallback_skill_md(_install_dir: &Path, info: &PackageInfo) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: \"{}\"", info.name);
    let _ = writeln!(out, "description: \"{}\"", info.description);
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "{}", info.description);
    out
}

/// Transform a markdown file (with YAML frontmatter) into Codex TOML.
fn md_to_toml(content: &str, fallback_name: &str, fallback_desc: &str) -> String {
    let pairs = parse_frontmatter(content);
    let body = strip_frontmatter(content);

    let name = frontmatter_value(&pairs, "name").unwrap_or(fallback_name);
    let description = frontmatter_value(&pairs, "description").unwrap_or(fallback_desc);
    let model = frontmatter_value(&pairs, "model");

    generate_codex_toml(name, description, body, model)
}

/// Generate a fallback TOML when no `.md` definition files are present.
fn generate_fallback_toml(install_dir: &Path, info: &PackageInfo) -> String {
    let instructions = match info.package_type {
        PackageType::Agent => {
            if let Some(agent) = &info.agent {
                if let Some(prompt_val) = &agent.system_prompt {
                    resolve_system_prompt(prompt_val, install_dir)
                } else {
                    format!("{} agent", info.description)
                }
            } else {
                format!("{} agent", info.description)
            }
        }
        _ => info.description.clone(),
    };

    generate_codex_toml(&info.name, &info.description, &instructions, None)
}

/// Build a TOML string with the Codex agent schema.
fn generate_codex_toml(
    name: &str,
    description: &str,
    developer_instructions: &str,
    model: Option<&str>,
) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "name = \"{}\"", escape_toml_basic(name));
    let _ = writeln!(
        out,
        "description = \"{}\"",
        escape_toml_basic(description)
    );

    // Omit model if absent or "inherit" (inherit means use parent session model).
    if let Some(m) = model {
        if !m.eq_ignore_ascii_case("inherit") {
            let _ = writeln!(out, "model = \"{}\"", escape_toml_basic(m));
        }
    }

    let escaped = escape_toml_multiline(developer_instructions);
    let _ = write!(out, "developer_instructions = \"\"\"\n{escaped}\"\"\"\n");

    out
}

/// Look up a value by key in a list of frontmatter pairs.
fn frontmatter_value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Escape a string for use inside a TOML basic string (`"..."`).
fn escape_toml_basic(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escape content for use inside a TOML multi-line basic string (`"""..."""`).
fn escape_toml_multiline(s: &str) -> String {
    // Inside """, the only problematic sequence is three or more consecutive quotes.
    s.replace("\"\"\"", "\"\"\\\"")
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
    fn test_generate_codex_toml_basic() {
        let toml = generate_codex_toml("my-agent", "A helpful agent", "You are helpful.", None);
        assert!(toml.contains("name = \"my-agent\""));
        assert!(toml.contains("description = \"A helpful agent\""));
        assert!(toml.contains("developer_instructions = \"\"\"\nYou are helpful.\"\"\""));
        assert!(!toml.contains("model ="));
    }

    #[test]
    fn test_generate_codex_toml_with_model() {
        let toml = generate_codex_toml(
            "my-agent",
            "Agent",
            "Instructions.",
            Some("gpt-5.4-mini"),
        );
        assert!(toml.contains("model = \"gpt-5.4-mini\""));
    }

    #[test]
    fn test_generate_codex_toml_inherit_model_omitted() {
        let toml = generate_codex_toml("my-agent", "Agent", "Instructions.", Some("inherit"));
        assert!(!toml.contains("model ="));
    }

    #[test]
    fn test_generate_codex_toml_escapes_quotes() {
        let toml =
            generate_codex_toml("my-agent", "Agent with \"quotes\"", "Say \"hello\".", None);
        assert!(toml.contains("description = \"Agent with \\\"quotes\\\"\""));
        // Inside multi-line basic strings, isolated quotes don't need escaping
        assert!(toml.contains("Say \"hello\"."));
    }

    #[test]
    fn test_generate_codex_toml_multiline_instructions() {
        let body = "Line one.\n\nLine two.\n\n## Section\n\nMore content.\n";
        let toml = generate_codex_toml("agent", "desc", body, None);
        assert!(toml.contains("developer_instructions = \"\"\"\nLine one."));
        assert!(toml.contains("## Section"));
        assert!(toml.contains("More content.\n\"\"\""));
    }

    #[test]
    fn test_generate_codex_toml_escapes_triple_quotes() {
        let body = "Text with \"\"\" inside.";
        let toml = generate_codex_toml("agent", "desc", body, None);
        assert!(toml.contains("\"\"\\\""));
    }

    #[test]
    fn test_md_to_toml() {
        let md = "---\nname: \"my-agent\"\ndescription: \"My agent\"\nmodel: gpt-5.4\n---\nYou are a specialist.\n";
        let toml = md_to_toml(md, "fallback", "fallback desc");
        assert!(toml.contains("name = \"my-agent\""));
        assert!(toml.contains("description = \"My agent\""));
        assert!(toml.contains("model = \"gpt-5.4\""));
        assert!(toml.contains("developer_instructions = \"\"\"\nYou are a specialist.\n\"\"\""));
    }

    #[test]
    fn test_md_to_toml_inherit_model() {
        let md = "---\nname: \"agent\"\ndescription: \"desc\"\nmodel: inherit\n---\nBody.\n";
        let toml = md_to_toml(md, "f", "f");
        assert!(!toml.contains("model ="));
    }

    #[test]
    fn test_md_to_toml_fallback_values() {
        let md = "---\ncolor: green\n---\nBody only frontmatter has no name.\n";
        let toml = md_to_toml(md, "pkg-name", "pkg description");
        assert!(toml.contains("name = \"pkg-name\""));
        assert!(toml.contains("description = \"pkg description\""));
    }

    #[test]
    fn test_setup_codex_transforms_md_to_toml() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("agent.md"),
            "---\nname: \"research-agent\"\ndescription: \"Research things\"\n---\nYou are a researcher.\n",
        )
        .unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        assert!(path.exists());
        assert_eq!(path.extension().unwrap(), "toml");
        assert!(path
            .parent()
            .unwrap()
            .ends_with("agents/@acme/research-agent"));

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("name = \"research-agent\""));
        assert!(content.contains("description = \"Research things\""));
        assert!(content.contains("developer_instructions = \"\"\"\nYou are a researcher.\n\"\"\""));
        // No YAML frontmatter in output
        assert!(!content.contains("---"));
    }

    #[test]
    fn test_setup_codex_fallback_from_package_info() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "research-agent.toml");

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("name = \"@acme/research-agent\""));
        assert!(content.contains("You are a research assistant."));
    }

    #[test]
    fn test_setup_codex_fallback_skill() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        assert!(path
            .parent()
            .unwrap()
            .ends_with("skills/@acme/code-reviewer"));
        assert_eq!(path.extension().unwrap(), "md");

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("name: \"@acme/code-reviewer\""));
        assert!(content.contains("AI-powered code review"));
    }

    #[test]
    fn test_setup_codex_skill_copies_md_as_is() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let original =
            "---\nname: \"review\"\ndescription: \"Review code\"\n---\nYou review code carefully.\n";
        fs::write(install_dir.join("review.md"), original).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        assert_eq!(path.extension().unwrap(), "md");
        assert!(path
            .parent()
            .unwrap()
            .ends_with("skills/@acme/code-reviewer"));

        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content, original);
    }

    #[test]
    fn test_setup_codex_skill_copies_multiple_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("review.md"),
            "---\nname: \"review\"\ndescription: \"Review\"\n---\nReview code.\n",
        )
        .unwrap();
        fs::write(
            install_dir.join("lint.md"),
            "---\nname: \"lint\"\ndescription: \"Lint\"\n---\nLint code.\n",
        )
        .unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.extension().unwrap() == "md"));

        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"review.md".to_string()));
        assert!(names.contains(&"lint.md".to_string()));
    }

    #[test]
    fn test_setup_codex_multiple_definitions() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/research-agent");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("explorer.md"),
            "---\nname: \"explorer\"\ndescription: \"Explore code\"\n---\nExplore the codebase.\n",
        )
        .unwrap();
        fs::write(
            install_dir.join("worker.md"),
            "---\nname: \"worker\"\ndescription: \"Do work\"\n---\nDo the work.\n",
        )
        .unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &agent_info()).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.exists()));
        assert!(paths
            .iter()
            .all(|p| p.extension().unwrap() == "toml"));

        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"explorer.toml".to_string()));
        assert!(names.contains(&"worker.toml".to_string()));
    }
}
