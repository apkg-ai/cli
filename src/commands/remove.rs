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
        return Err(AppError::Other(format!(
            "Package \"{}\" is not in {}",
            opts.package,
            opts.category.label(),
        )));
    };

    if deps.remove(opts.package).is_none() {
        return Err(AppError::Other(format!(
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

    // Clean up tool-setup files (e.g. .claude/agents/*, .claude/skills/*)
    cleanup_claude_setup(&cwd, opts.package);

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

/// Remove Claude Code setup files (`.claude/agents/` and `.claude/skills/`)
/// that belong to the given package, matching both legacy pointer files and
/// copied definition files by their naming prefix.
fn cleanup_claude_setup(project_root: &Path, package_name: &str) {
    let stem = crate::setup::config_file_stem(package_name);

    for subdir in &["agents", "skills", "commands", "rules"] {
        let dir = project_root.join(".claude").join(subdir);

        // New format: remove the subdirectory .claude/{type}/{stem}/
        let pkg_dir = dir.join(&stem);
        if pkg_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&pkg_dir);
        }

        // Legacy cleanup: remove flat files that match old naming convention.
        // This ensures packages installed before the migration are cleaned up.
        let legacy = format!("{stem}.md");
        let prefix = format!("{stem}--");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.as_ref() == legacy || name.starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
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

        // New format: subdirectory for the package
        let pkg_dir = agents_dir.join("sheplu--agent-reviewer");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("code-reviewer.md"), "def").unwrap();

        // Legacy flat files (should also be cleaned up)
        std::fs::write(agents_dir.join("sheplu--agent-reviewer.md"), "pointer").unwrap();
        std::fs::write(
            agents_dir.join("sheplu--agent-reviewer--code-reviewer.md"),
            "def",
        )
        .unwrap();
        // Unrelated file — should NOT be removed
        std::fs::write(agents_dir.join("other--pkg.md"), "other").unwrap();

        cleanup_claude_setup(tmp.path(), "@sheplu/agent-reviewer");

        // Subdirectory should be removed
        assert!(!pkg_dir.exists());
        // Legacy flat files should be removed
        assert!(!agents_dir.join("sheplu--agent-reviewer.md").exists());
        assert!(!agents_dir
            .join("sheplu--agent-reviewer--code-reviewer.md")
            .exists());
        // Unrelated file should survive
        assert!(agents_dir.join("other--pkg.md").exists());
    }

    #[test]
    fn test_cleanup_claude_setup_removes_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let commands_dir = tmp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();

        let pkg_dir = commands_dir.join("sheplu--command-audit");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("audit.md"), "audit content").unwrap();

        cleanup_claude_setup(tmp.path(), "@sheplu/command-audit");

        assert!(!pkg_dir.exists());
    }

    #[test]
    fn test_cleanup_claude_setup_removes_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let rules_dir = tmp.path().join(".claude").join("rules");
        std::fs::create_dir_all(&rules_dir).unwrap();

        let pkg_dir = rules_dir.join("acme--my-rule");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("lint.md"), "rule content").unwrap();

        cleanup_claude_setup(tmp.path(), "@acme/my-rule");

        assert!(!pkg_dir.exists());
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
