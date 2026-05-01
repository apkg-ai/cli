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
    // Codex treats commands as skills — place them in the skills directory.
    let type_dir = if info.package_type == PackageType::Command {
        PackageType::Skill.dir_name()
    } else {
        info.package_type.dir_name()
    };
    let target_dir = project_root.join(".codex").join(type_dir).join(&pkg_path);
    fs::create_dir_all(&target_dir).map_err(|e| {
        format!(
            "Failed to create .codex/{type_dir}/{}/: {e}",
            pkg_path.display()
        )
    })?;

    let require_frontmatter =
        !matches!(info.package_type, PackageType::Command | PackageType::Rule);
    let md_files = find_definition_files(install_dir, require_frontmatter);

    // Skills (and commands, which Codex treats as skills) stay as markdown —
    // copy them directly instead of transforming to TOML.
    if matches!(
        info.package_type,
        PackageType::Skill | PackageType::Command | PackageType::Rule
    ) {
        let rename_to_skill = info.package_type == PackageType::Command;
        let mut created =
            setup_codex_skill(&target_dir, install_dir, info, &md_files, rename_to_skill)?;

        // After writing rule files, update the managed section in AGENTS.md
        // so Codex can discover the rules.
        if info.package_type == PackageType::Rule {
            let rules = collect_codex_rules(project_root);
            if let Some(path) = update_agents_md_rules(project_root, &rules)? {
                created.push(path);
            }
        }

        return Ok(created);
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
        fs::write(&dest, &toml).map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        created.push(dest);
    }

    Ok(created)
}

/// Set up a skill for Codex by copying markdown files as-is (no TOML
/// transformation). When no `.md` definition files exist, a fallback `.md`
/// file is generated from the package info.
///
/// When `rename_to_skill` is true (commands promoted to skills), all files
/// are written as `SKILL.md` to match the Codex skill convention.
fn setup_codex_skill(
    target_dir: &Path,
    install_dir: &Path,
    info: &PackageInfo,
    md_files: &[PathBuf],
    rename_to_skill: bool,
) -> Result<Vec<PathBuf>, String> {
    if md_files.is_empty() {
        let dest_name = if rename_to_skill {
            "SKILL.md".to_string()
        } else {
            format!("{}.md", package_short_name(&info.name))
        };
        let dest = target_dir.join(dest_name);
        let content = generate_fallback_skill_md(install_dir, info);
        fs::write(&dest, content)
            .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
        return Ok(vec![dest]);
    }

    let mut created = Vec::new();
    for (i, src) in md_files.iter().enumerate() {
        let dest_name = if rename_to_skill && i == 0 {
            "SKILL.md".to_string()
        } else {
            src.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let dest = target_dir.join(&dest_name);
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
    let _ = writeln!(out, "description = \"{}\"", escape_toml_basic(description));

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

const RULES_SECTION_START: &str = "<!-- apkg:rules -->";
const RULES_SECTION_END: &str = "<!-- /apkg:rules -->";

/// Scan `.codex/rules/` for all installed rule `.md` files.
/// Returns `(title, relative_path)` pairs sorted by path for deterministic output.
pub fn collect_codex_rules(project_root: &Path) -> Vec<(String, PathBuf)> {
    let rules_dir = project_root.join(".codex").join("rules");
    let mut rules = Vec::new();

    let Ok(scopes) = fs::read_dir(&rules_dir) else {
        return rules;
    };

    for scope_entry in scopes.filter_map(Result::ok) {
        let scope_path = scope_entry.path();
        if !scope_path.is_dir() {
            continue;
        }

        // Could be a scope dir (@acme) containing package dirs, or a package dir directly.
        let scope_name = scope_entry.file_name().to_string_lossy().into_owned();
        if scope_name.starts_with('@') {
            // Scoped: iterate package dirs inside scope
            if let Ok(pkgs) = fs::read_dir(&scope_path) {
                for pkg_entry in pkgs.filter_map(Result::ok) {
                    if pkg_entry.path().is_dir() {
                        collect_rules_from_pkg_dir(project_root, &pkg_entry.path(), &mut rules);
                    }
                }
            }
        } else {
            // Unscoped package dir
            collect_rules_from_pkg_dir(project_root, &scope_path, &mut rules);
        }
    }

    rules.sort_by(|a, b| a.1.cmp(&b.1));
    rules
}

/// Collect rule entries from a single package directory under `.codex/rules/`.
fn collect_rules_from_pkg_dir(
    project_root: &Path,
    pkg_dir: &Path,
    rules: &mut Vec<(String, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(pkg_dir) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            let title = extract_rule_title(&path);
            let rel = path
                .strip_prefix(project_root)
                .unwrap_or(&path)
                .to_path_buf();
            rules.push((title, rel));
        }
    }
}

/// Extract a human-readable title from a rule file.
/// Tries frontmatter `description`, then `name`, then falls back to the file stem.
fn extract_rule_title(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_default();
    let pairs = parse_frontmatter(&content);

    if let Some((_, desc)) = pairs.iter().find(|(k, _)| k == "description") {
        if !desc.is_empty() {
            return desc.clone();
        }
    }
    if let Some((_, name)) = pairs.iter().find(|(k, _)| k == "name") {
        if !name.is_empty() {
            return name.clone();
        }
    }

    // Fallback: prettify the file stem
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rule".to_string())
}

/// Update (or create) the managed `<!-- apkg:rules -->` section in `AGENTS.md`.
///
/// - If `rules` is empty and no managed section exists, does nothing and returns `None`.
/// - If `rules` is empty and a managed section exists, removes it.
/// - Otherwise, writes/replaces the managed section with the current rule list.
///
/// Returns `Some(path)` to `AGENTS.md` when the file was written.
pub fn update_agents_md_rules(
    project_root: &Path,
    rules: &[(String, PathBuf)],
) -> Result<Option<PathBuf>, String> {
    let agents_md = project_root.join("AGENTS.md");
    let existing = fs::read_to_string(&agents_md).unwrap_or_default();

    let has_section = existing.contains(RULES_SECTION_START);

    // Nothing to do: no rules and no existing section to clean up.
    if rules.is_empty() && !has_section {
        return Ok(None);
    }

    let section = if rules.is_empty() {
        String::new()
    } else {
        let mut s = String::new();
        let _ = writeln!(s, "{RULES_SECTION_START}");
        let _ = writeln!(s, "## Rules");
        let _ = writeln!(s);
        for (title, rel_path) in rules {
            let _ = writeln!(s, "- [{}]({})", title, rel_path.display());
        }
        let _ = write!(s, "{RULES_SECTION_END}");
        s
    };

    let new_content = if has_section {
        // Replace existing section (including markers).
        if let (Some(start), Some(end)) = (
            existing.find(RULES_SECTION_START),
            existing.find(RULES_SECTION_END),
        ) {
            let before = &existing[..start];
            let after = &existing[end + RULES_SECTION_END.len()..];

            if section.is_empty() {
                // Remove the section and any surrounding blank lines.
                let before = before.trim_end_matches('\n');
                let after = after.trim_start_matches('\n');
                if before.is_empty() && after.is_empty() {
                    String::new()
                } else if before.is_empty() {
                    after.to_string()
                } else if after.is_empty() {
                    format!("{before}\n")
                } else {
                    format!("{before}\n\n{after}")
                }
            } else {
                format!("{before}{section}{after}")
            }
        } else {
            // Malformed markers — append fresh section.
            let mut out = existing.clone();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(&section);
            out.push('\n');
            out
        }
    } else {
        // No existing section — append.
        let mut out = existing;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&section);
        out.push('\n');
        out
    };

    if new_content.is_empty() {
        // If file becomes empty, remove it.
        let _ = fs::remove_file(&agents_md);
    } else {
        fs::write(&agents_md, &new_content)
            .map_err(|e| format!("Failed to write {}: {e}", agents_md.display()))?;
    }

    Ok(Some(agents_md))
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
        let toml = generate_codex_toml("my-agent", "Agent", "Instructions.", Some("gpt-5.4-mini"));
        assert!(toml.contains("model = \"gpt-5.4-mini\""));
    }

    #[test]
    fn test_generate_codex_toml_inherit_model_omitted() {
        let toml = generate_codex_toml("my-agent", "Agent", "Instructions.", Some("inherit"));
        assert!(!toml.contains("model ="));
    }

    #[test]
    fn test_generate_codex_toml_escapes_quotes() {
        let toml = generate_codex_toml("my-agent", "Agent with \"quotes\"", "Say \"hello\".", None);
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
        assert!(paths.iter().all(|p| p.extension().unwrap() == "toml"));

        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"explorer.toml".to_string()));
        assert!(names.contains(&"worker.toml".to_string()));
    }

    // --- Command-as-skill tests ---

    fn command_info() -> PackageInfo {
        PackageInfo {
            name: "@sheplu/generate-changelog".to_string(),
            package_type: PackageType::Command,
            description: "Generate a changelog from git history".to_string(),
            main: None,
            platform: Vec::new(),
            skill: None,
            agent: None,
        }
    }

    #[test]
    fn test_setup_codex_command_as_skill_copies_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/generate-changelog");
        fs::create_dir_all(&install_dir).unwrap();
        let original = "---\nname: changelog\ndescription: Generate a changelog\n---\nGenerate a structured changelog.\n";
        fs::write(install_dir.join("changelog.md"), original).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &command_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        // Commands are renamed to SKILL.md for Codex
        assert_eq!(path.file_name().unwrap(), "SKILL.md");
        // Commands go into the skills directory for Codex
        assert!(path
            .parent()
            .unwrap()
            .ends_with("skills/@sheplu/generate-changelog"));

        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content, original);
    }

    #[test]
    fn test_setup_codex_command_as_skill_fallback() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/generate-changelog");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &command_info()).unwrap();
        assert_eq!(paths.len(), 1);

        let path = &paths[0];
        assert_eq!(path.file_name().unwrap(), "SKILL.md");
        assert!(path
            .parent()
            .unwrap()
            .ends_with("skills/@sheplu/generate-changelog"));

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("name:"));
        assert!(content.contains("Generate a changelog"));
    }

    #[test]
    fn test_setup_codex_command_as_skill_plain_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@sheplu/generate-changelog");
        fs::create_dir_all(&install_dir).unwrap();
        // Command with no frontmatter — should still be found (require_frontmatter=false)
        fs::write(
            install_dir.join("audit.md"),
            "Run a comprehensive audit of the project.",
        )
        .unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &command_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "SKILL.md");

        let content = fs::read_to_string(&paths[0]).unwrap();
        assert!(content.contains("Run a comprehensive audit"));
    }

    // --- Rule tests ---

    fn rule_info() -> PackageInfo {
        PackageInfo {
            name: "@acme/no-todo-comments".to_string(),
            package_type: PackageType::Rule,
            description: "Disallow TODO comments in code".to_string(),
            main: None,
            platform: Vec::new(),
            skill: None,
            agent: None,
        }
    }

    #[test]
    fn test_setup_codex_rule_copies_md_and_updates_agents_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/no-todo-comments");
        fs::create_dir_all(&install_dir).unwrap();
        let original = "---\nname: no-todo\ndescription: Disallow TODO comments\n---\nDo not leave TODO comments in code.\n";
        fs::write(install_dir.join("no-todo.md"), original).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &rule_info()).unwrap();
        // Rule file + AGENTS.md
        assert_eq!(paths.len(), 2);

        let rule_path = &paths[0];
        assert_eq!(rule_path.extension().unwrap(), "md");
        assert!(rule_path
            .parent()
            .unwrap()
            .ends_with("rules/@acme/no-todo-comments"));
        let content = fs::read_to_string(rule_path).unwrap();
        assert_eq!(content, original);

        // AGENTS.md created with managed section
        let agents_md = tmp.path().join("AGENTS.md");
        assert!(agents_md.exists());
        let agents_content = fs::read_to_string(&agents_md).unwrap();
        assert!(agents_content.contains("<!-- apkg:rules -->"));
        assert!(agents_content.contains("<!-- /apkg:rules -->"));
        assert!(agents_content.contains("[Disallow TODO comments]"));
        assert!(agents_content.contains(".codex/rules/@acme/no-todo-comments/no-todo.md"));
    }

    #[test]
    fn test_setup_codex_rule_fallback_updates_agents_md() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/no-todo-comments");
        fs::create_dir_all(&install_dir).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &rule_info()).unwrap();
        // Rule file + AGENTS.md
        assert_eq!(paths.len(), 2);

        let rule_path = &paths[0];
        assert!(rule_path
            .parent()
            .unwrap()
            .ends_with("rules/@acme/no-todo-comments"));
        assert_eq!(rule_path.extension().unwrap(), "md");

        let content = fs::read_to_string(rule_path).unwrap();
        assert!(content.contains("name:"));
        assert!(content.contains("Disallow TODO comments"));

        let agents_md = tmp.path().join("AGENTS.md");
        assert!(agents_md.exists());
    }

    #[test]
    fn test_setup_codex_rule_plain_md_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/no-todo-comments");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(
            install_dir.join("rule.md"),
            "Never leave TODO comments in production code.",
        )
        .unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &rule_info()).unwrap();
        assert!(paths.iter().any(|p| p.extension().unwrap() == "md"
            && p.parent()
                .unwrap()
                .ends_with("rules/@acme/no-todo-comments")));

        let rule_path = paths
            .iter()
            .find(|p| p.file_name().unwrap() != "AGENTS.md")
            .unwrap();
        let content = fs::read_to_string(rule_path).unwrap();
        assert!(content.contains("Never leave TODO comments"));
    }

    // --- AGENTS.md managed section tests ---

    #[test]
    fn test_update_agents_md_creates_file() {
        let tmp = TempDir::new().unwrap();
        let rules = vec![(
            "Disallow TODO".to_string(),
            PathBuf::from(".codex/rules/@acme/no-todo/no-todo.md"),
        )];
        let result = update_agents_md_rules(tmp.path(), &rules).unwrap();
        assert!(result.is_some());

        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("<!-- apkg:rules -->"));
        assert!(content.contains("## Rules"));
        assert!(content.contains("- [Disallow TODO](.codex/rules/@acme/no-todo/no-todo.md)"));
        assert!(content.contains("<!-- /apkg:rules -->"));
    }

    #[test]
    fn test_update_agents_md_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("AGENTS.md"),
            "# My Project Agents\n\nSome existing content.\n",
        )
        .unwrap();

        let rules = vec![(
            "My rule".to_string(),
            PathBuf::from(".codex/rules/my-rule/rule.md"),
        )];
        update_agents_md_rules(tmp.path(), &rules).unwrap();

        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.starts_with("# My Project Agents"));
        assert!(content.contains("Some existing content."));
        assert!(content.contains("<!-- apkg:rules -->"));
        assert!(content.contains("[My rule]"));
        assert!(content.contains("<!-- /apkg:rules -->"));
    }

    #[test]
    fn test_update_agents_md_replaces_existing_section() {
        let tmp = TempDir::new().unwrap();
        let initial = "# Agents\n\n<!-- apkg:rules -->\n## Rules\n\n- [Old rule](.codex/rules/old/old.md)\n<!-- /apkg:rules -->\n\nMore content.\n";
        fs::write(tmp.path().join("AGENTS.md"), initial).unwrap();

        let rules = vec![(
            "New rule".to_string(),
            PathBuf::from(".codex/rules/new/new.md"),
        )];
        update_agents_md_rules(tmp.path(), &rules).unwrap();

        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("# Agents"));
        assert!(content.contains("[New rule](.codex/rules/new/new.md)"));
        assert!(!content.contains("Old rule"));
        assert!(content.contains("More content."));
    }

    #[test]
    fn test_update_agents_md_removes_section_when_empty() {
        let tmp = TempDir::new().unwrap();
        let initial = "# Agents\n\n<!-- apkg:rules -->\n## Rules\n\n- [Old rule](.codex/rules/old/old.md)\n<!-- /apkg:rules -->\n\nMore content.\n";
        fs::write(tmp.path().join("AGENTS.md"), initial).unwrap();

        let rules: Vec<(String, PathBuf)> = vec![];
        update_agents_md_rules(tmp.path(), &rules).unwrap();

        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(!content.contains("apkg:rules"));
        assert!(content.contains("# Agents"));
        assert!(content.contains("More content."));
    }

    #[test]
    fn test_update_agents_md_noop_when_empty_and_no_file() {
        let tmp = TempDir::new().unwrap();
        let rules: Vec<(String, PathBuf)> = vec![];
        let result = update_agents_md_rules(tmp.path(), &rules).unwrap();
        assert!(result.is_none());
        assert!(!tmp.path().join("AGENTS.md").exists());
    }

    #[test]
    fn test_update_agents_md_removes_file_when_only_section() {
        let tmp = TempDir::new().unwrap();
        let initial = "<!-- apkg:rules -->\n## Rules\n\n- [Rule](.codex/rules/r/r.md)\n<!-- /apkg:rules -->\n";
        fs::write(tmp.path().join("AGENTS.md"), initial).unwrap();

        let rules: Vec<(String, PathBuf)> = vec![];
        update_agents_md_rules(tmp.path(), &rules).unwrap();

        // File should be removed since it would be empty.
        assert!(!tmp.path().join("AGENTS.md").exists());
    }

    #[test]
    fn test_collect_codex_rules_scoped_and_unscoped() {
        let tmp = TempDir::new().unwrap();

        // Scoped rule
        let scoped = tmp.path().join(".codex/rules/@acme/no-todo");
        fs::create_dir_all(&scoped).unwrap();
        fs::write(
            scoped.join("no-todo.md"),
            "---\nname: no-todo\ndescription: Disallow TODO\n---\nContent.\n",
        )
        .unwrap();

        // Unscoped rule
        let unscoped = tmp.path().join(".codex/rules/my-rule");
        fs::create_dir_all(&unscoped).unwrap();
        fs::write(unscoped.join("rule.md"), "Just a plain rule.").unwrap();

        let rules = collect_codex_rules(tmp.path());
        assert_eq!(rules.len(), 2);

        // Sorted by path, so .codex/rules/@acme comes before .codex/rules/my-rule
        assert_eq!(rules[0].0, "Disallow TODO");
        assert_eq!(rules[1].0, "rule"); // fallback to file stem
    }

    #[test]
    fn test_extract_rule_title_from_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rule.md");
        fs::write(
            &path,
            "---\nname: my-rule\ndescription: A great rule\n---\nBody.\n",
        )
        .unwrap();
        assert_eq!(extract_rule_title(&path), "A great rule");
    }

    #[test]
    fn test_extract_rule_title_fallback_to_name() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("rule.md");
        fs::write(&path, "---\nname: my-rule\n---\nBody.\n").unwrap();
        assert_eq!(extract_rule_title(&path), "my-rule");
    }

    #[test]
    fn test_extract_rule_title_fallback_to_stem() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("my-custom-rule.md");
        fs::write(&path, "No frontmatter here.").unwrap();
        assert_eq!(extract_rule_title(&path), "my-custom-rule");
    }

    #[test]
    fn test_setup_codex_actual_skill_keeps_original_name() {
        // Verify that real skills are NOT renamed to SKILL.md
        let tmp = TempDir::new().unwrap();
        let install_dir = tmp.path().join("apkg_packages/@acme/code-reviewer");
        fs::create_dir_all(&install_dir).unwrap();
        let original =
            "---\nname: \"review\"\ndescription: \"Review code\"\n---\nYou review code carefully.\n";
        fs::write(install_dir.join("review.md"), original).unwrap();

        let paths = setup_codex(tmp.path(), &install_dir, &skill_info()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file_name().unwrap(), "review.md");
    }
}
