use std::env;
use std::path::Path;

use crate::config::manifest;
use crate::error::AppError;
use crate::util::display;
use crate::util::package::DepCategory;

pub struct RemoveOptions<'a> {
    pub package: &'a str,
    pub category: DepCategory,
}

pub fn run(opts: &RemoveOptions<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let mut m = manifest::load(&cwd)?;

    let deps = match opts.category {
        DepCategory::Dependencies => m.dependencies.as_mut(),
        DepCategory::DevDependencies => m.dev_dependencies.as_mut(),
        DepCategory::PeerDependencies => m.peer_dependencies.as_mut(),
    };

    let Some(deps) = deps else {
        return Err(AppError::InvalidInput(format!(
            "Package \"{}\" is not in {}",
            opts.package,
            opts.category.label(),
        )));
    };

    if deps.remove(opts.package).is_none() {
        return Err(AppError::InvalidInput(format!(
            "Package \"{}\" is not in {}",
            opts.package,
            opts.category.label(),
        )));
    }

    // Clear the map if it's now empty so it's omitted from apkg.json
    if deps.is_empty() {
        match opts.category {
            DepCategory::Dependencies => m.dependencies = None,
            DepCategory::DevDependencies => m.dev_dependencies = None,
            DepCategory::PeerDependencies => m.peer_dependencies = None,
        }
    }

    manifest::save(&cwd, &m)?;

    // Remove installed files
    let install_dir = cwd.join("apkg_packages").join(opts.package);
    let removed_files = if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)?;
        cleanup_empty_parents(&install_dir, &cwd.join("apkg_packages"));
        true
    } else {
        false
    };

    // Clean up tool-setup files (e.g. .claude/agents/*, .cursor/skills/*)
    cleanup_claude_setup(&cwd, opts.package);
    cleanup_cursor_setup(&cwd, opts.package);
    cleanup_codex_setup(&cwd, opts.package);

    display::success(&format!(
        "Removed {} from {}",
        opts.package,
        opts.category.label()
    ));
    if removed_files {
        display::label_value("Deleted", &install_dir.display().to_string());
    }

    Ok(())
}

/// Remove Claude Code setup files (`.claude/{type}/@scope/name/` or `.claude/{type}/name/`)
/// that belong to the given package.
fn cleanup_claude_setup(project_root: &Path, package_name: &str) {
    let pkg_path = crate::setup::config_pkg_path(package_name);

    for subdir in &["agents", "skills", "commands", "rules"] {
        let dir = project_root.join(".claude").join(subdir);

        let pkg_dir = dir.join(&pkg_path);
        if pkg_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }
        cleanup_empty_scope_dir(&pkg_dir, &dir);
    }
}

/// Remove Codex setup files (`.codex/{type}/@scope/name/` or `.codex/{type}/name/`)
/// that belong to the given package, and rebuild the AGENTS.md managed rules section.
fn cleanup_codex_setup(project_root: &Path, package_name: &str) {
    let pkg_path = crate::setup::config_pkg_path(package_name);

    for subdir in &["agents", "skills", "rules"] {
        let dir = project_root.join(".codex").join(subdir);

        let pkg_dir = dir.join(&pkg_path);
        if pkg_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }
        cleanup_empty_scope_dir(&pkg_dir, &dir);
    }

    // Rebuild the managed rules section in AGENTS.md after removing rule files.
    let rules = crate::setup::codex::collect_codex_rules(project_root);
    let _ = crate::setup::codex::update_agents_md_rules(project_root, &rules);

    // Clean up any dangling CLAUDE.md symlink if AGENTS.md was removed.
    crate::setup::maybe_cleanup_claude_md_symlink(project_root);
}

/// Remove Cursor setup files (`.cursor/{type}/@scope/name/` or `.cursor/{type}/name/`)
/// that belong to the given package.
fn cleanup_cursor_setup(project_root: &Path, package_name: &str) {
    let pkg_path = crate::setup::config_pkg_path(package_name);

    for subdir in &["skills", "agents", "commands", "rules"] {
        let dir = project_root.join(".cursor").join(subdir);

        let pkg_dir = dir.join(&pkg_path);
        if pkg_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }
        cleanup_empty_scope_dir(&pkg_dir, &dir);
    }
}

/// If `path` is inside a scope directory (e.g., .claude/skills/@acme/code-reviewer),
/// clean up the scope directory (.claude/skills/@acme/) if it is now empty.
fn cleanup_empty_scope_dir(path: &Path, stop_at: &Path) {
    if let Some(parent) = path.parent() {
        if parent != stop_at && std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Remove empty parent directories up to (but not including) `stop_at`.
/// Handles scoped packages: removing `apkg_packages/@scope/pkg` may leave
/// an empty `apkg_packages/@scope/` directory.
fn cleanup_empty_parents(path: &Path, stop_at: &Path) {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent == stop_at {
            break;
        }
        // Only remove if empty
        if std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(parent);
        } else {
            break;
        }
        current = parent.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_claude_setup_removes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        // Scoped subdirectory for the package
        let pkg_dir = agents_dir.join("@sheplu").join("agent-reviewer");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("code-reviewer.md"), "def").unwrap();

        // Unrelated file — should NOT be removed
        std::fs::write(agents_dir.join("other-pkg.md"), "other").unwrap();

        cleanup_claude_setup(tmp.path(), "@sheplu/agent-reviewer");

        // Scoped subdirectory should be removed
        assert!(!pkg_dir.exists());
        // Empty scope dir should be removed
        assert!(!agents_dir.join("@sheplu").exists());
        // Unrelated file should survive
        assert!(agents_dir.join("other-pkg.md").exists());
    }

    #[test]
    fn test_cleanup_claude_setup_removes_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();

        let pkg_dir = commands_dir.join("@sheplu").join("command-audit");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("audit.md"), "audit content").unwrap();

        cleanup_claude_setup(tmp.path(), "@sheplu/command-audit");

        assert!(!pkg_dir.exists());
        assert!(!commands_dir.join("@sheplu").exists());
    }

    #[test]
    fn test_cleanup_claude_setup_removes_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let rules_dir = tmp.path().join(".claude").join("rules");
        std::fs::create_dir_all(&rules_dir).unwrap();

        let pkg_dir = rules_dir.join("@acme").join("my-rule");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("lint.md"), "rule content").unwrap();

        cleanup_claude_setup(tmp.path(), "@acme/my-rule");

        assert!(!pkg_dir.exists());
        assert!(!rules_dir.join("@acme").exists());
    }

    #[test]
    fn test_cleanup_cursor_setup_removes_skill_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".cursor").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        let pkg_dir = skills_dir.join("@acme").join("code-reviewer");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("SKILL.md"), "skill content").unwrap();

        // Unrelated skill — should NOT be removed
        let other_dir = skills_dir.join("other--skill");
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(other_dir.join("SKILL.md"), "other").unwrap();

        cleanup_cursor_setup(tmp.path(), "@acme/code-reviewer");

        assert!(!pkg_dir.exists());
        assert!(!skills_dir.join("@acme").exists());
        assert!(other_dir.exists());
    }

    #[test]
    fn test_cleanup_cursor_setup_removes_agent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join(".cursor").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        let pkg_dir = agents_dir.join("@acme").join("research-agent");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("research-agent.md"), "agent").unwrap();

        cleanup_cursor_setup(tmp.path(), "@acme/research-agent");

        assert!(!pkg_dir.exists());
        assert!(!agents_dir.join("@acme").exists());
    }

    #[test]
    fn test_cleanup_cursor_setup_removes_rule_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let rules_dir = tmp.path().join(".cursor").join("rules");
        std::fs::create_dir_all(&rules_dir).unwrap();

        let pkg_dir = rules_dir.join("@acme").join("my-rule");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("my-rule.mdc"), "rule").unwrap();

        cleanup_cursor_setup(tmp.path(), "@acme/my-rule");

        assert!(!pkg_dir.exists());
        assert!(!rules_dir.join("@acme").exists());
    }

    #[test]
    fn test_cleanup_codex_setup_removes_agent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join(".codex").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        let pkg_dir = agents_dir.join("@acme").join("research-agent");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("research-agent.toml"), "name = \"test\"").unwrap();

        cleanup_codex_setup(tmp.path(), "@acme/research-agent");

        assert!(!pkg_dir.exists());
        assert!(!agents_dir.join("@acme").exists());
    }

    #[test]
    fn test_cleanup_codex_setup_removes_rule_and_updates_agents_md() {
        let tmp = tempfile::tempdir().unwrap();

        // Set up a rule in .codex/rules/
        let rules_dir = tmp.path().join(".codex").join("rules");
        let pkg_dir = rules_dir.join("@acme").join("no-todo");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("no-todo.md"),
            "---\nname: no-todo\ndescription: Disallow TODO\n---\nContent.\n",
        )
        .unwrap();

        // Set up AGENTS.md with the rule referenced
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "# Agents\n\n<!-- apkg:rules -->\n## Rules\n\n- [Disallow TODO](.codex/rules/@acme/no-todo/no-todo.md)\n<!-- /apkg:rules -->\n",
        )
        .unwrap();

        cleanup_codex_setup(tmp.path(), "@acme/no-todo");

        // Rule dir should be removed
        assert!(!pkg_dir.exists());

        // AGENTS.md should have section removed (no remaining rules)
        let content = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(!content.contains("apkg:rules"));
        assert!(content.contains("# Agents"));
    }

    #[test]
    fn test_cleanup_codex_setup_keeps_other_rules_in_agents_md() {
        let tmp = tempfile::tempdir().unwrap();

        // Set up two rules
        let rules_dir = tmp.path().join(".codex").join("rules");
        let pkg1 = rules_dir.join("@acme").join("no-todo");
        std::fs::create_dir_all(&pkg1).unwrap();
        std::fs::write(
            pkg1.join("no-todo.md"),
            "---\ndescription: Disallow TODO\n---\nContent.\n",
        )
        .unwrap();

        let pkg2 = rules_dir.join("@acme").join("snake-case");
        std::fs::create_dir_all(&pkg2).unwrap();
        std::fs::write(
            pkg2.join("snake-case.md"),
            "---\ndescription: Use snake_case\n---\nContent.\n",
        )
        .unwrap();

        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "<!-- apkg:rules -->\n## Rules\n\n- [Disallow TODO](.codex/rules/@acme/no-todo/no-todo.md)\n- [Use snake_case](.codex/rules/@acme/snake-case/snake-case.md)\n<!-- /apkg:rules -->\n",
        )
        .unwrap();

        // Remove only the first rule
        cleanup_codex_setup(tmp.path(), "@acme/no-todo");

        let content = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("<!-- apkg:rules -->"));
        assert!(content.contains("[Use snake_case]"));
        assert!(!content.contains("no-todo"));
    }

    #[cfg(unix)]
    #[test]
    fn test_cleanup_codex_removes_dangling_claude_md_symlink() {
        let tmp = tempfile::tempdir().unwrap();

        // Set up a single rule in .codex/rules/
        let rules_dir = tmp.path().join(".codex").join("rules");
        let pkg_dir = rules_dir.join("@acme").join("only-rule");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("only-rule.md"),
            "---\nname: only-rule\ndescription: The only rule\n---\nContent.\n",
        )
        .unwrap();

        // AGENTS.md with the rule
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "<!-- apkg:rules -->\n## Rules\n\n- [The only rule](.codex/rules/@acme/only-rule/only-rule.md)\n<!-- /apkg:rules -->\n",
        )
        .unwrap();

        // CLAUDE.md symlink to AGENTS.md
        std::os::unix::fs::symlink("AGENTS.md", tmp.path().join("CLAUDE.md")).unwrap();

        // Remove the rule — should clean up AGENTS.md and the CLAUDE.md symlink
        cleanup_codex_setup(tmp.path(), "@acme/only-rule");

        assert!(!tmp.path().join("AGENTS.md").exists());
        assert!(tmp.path().join("CLAUDE.md").symlink_metadata().is_err());
    }

    #[test]
    fn test_cleanup_empty_parents_removes_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("apkg_packages");
        let scope_dir = base.join("@scope");
        let pkg_dir = scope_dir.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        // Simulate the package dir already being removed
        std::fs::remove_dir(&pkg_dir).unwrap();
        cleanup_empty_parents(&pkg_dir, &base);
        assert!(!scope_dir.exists(), "empty @scope dir should be removed");
    }

    #[test]
    fn test_cleanup_empty_parents_keeps_nonempty() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("apkg_packages");
        let scope_dir = base.join("@scope");
        let pkg_dir = scope_dir.join("pkg");
        let other_dir = scope_dir.join("other-pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::remove_dir(&pkg_dir).unwrap();
        cleanup_empty_parents(&pkg_dir, &base);
        assert!(
            scope_dir.exists(),
            "@scope dir should remain because other-pkg exists"
        );
    }
}
