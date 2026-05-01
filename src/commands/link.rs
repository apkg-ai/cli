use std::path::Path;
use std::{env, fs};

use crate::config::{links, manifest};
use crate::error::AppError;
use crate::util::display;

pub enum LinkAction<'a> {
    /// `apkg link` — register current package globally
    Register,
    /// `apkg link <target>` — link into current project's `apkg_packages/`
    LinkTarget { target: &'a str },
}

pub enum UnlinkAction<'a> {
    /// `apkg unlink` — unregister current package from global store
    Unregister,
    /// `apkg unlink <package>` — remove symlink from `apkg_packages/`
    UnlinkPackage { package: &'a str },
}

pub fn run_link(action: &LinkAction<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;

    match *action {
        LinkAction::Register => {
            let m = manifest::load(&cwd)?;
            let abs_path = cwd.canonicalize()?;
            links::register(&m.name, &abs_path)?;
            display::success(&format!("Linked {} globally.", m.name));
            display::info(&format!(
                "Other projects can now link to it with: apkg link {}",
                m.name
            ));
            Ok(())
        }
        LinkAction::LinkTarget { target } => {
            let (name, source_path) = resolve_target(target, &cwd)?;

            let link_path = cwd.join("apkg_packages").join(&name);

            // For scoped packages, ensure the parent scope directory exists
            if let Some(parent) = link_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // If symlink already exists at destination, remove it first (re-link)
            if link_path.symlink_metadata().is_ok() {
                remove_symlink(&link_path)?;
            }

            create_symlink(&source_path, &link_path)?;
            display::success(&format!("Linked {} -> {}", name, source_path.display()));
            Ok(())
        }
    }
}

pub fn run_unlink(action: &UnlinkAction<'_>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;

    match action {
        UnlinkAction::Unregister => {
            let m = manifest::load(&cwd)?;
            let removed = links::unregister(&m.name)?;
            if removed {
                display::success(&format!("Unlinked {} from global store.", m.name));
            } else {
                display::warn(&format!("{} was not linked globally.", m.name));
            }
            Ok(())
        }
        UnlinkAction::UnlinkPackage { package } => {
            let link_path = cwd.join("apkg_packages").join(package);

            let is_symlink = link_path
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink());
            if !is_symlink {
                return Err(AppError::Other(format!(
                    "{package} is not a linked package"
                )));
            }

            remove_symlink(&link_path)?;
            cleanup_empty_parents(&link_path, &cwd.join("apkg_packages"));
            display::success(&format!("Unlinked {package}."));
            Ok(())
        }
    }
}

/// Determine whether the target is a filesystem path or a globally-registered name.
/// Returns `(package_name, absolute_source_path)`.
fn resolve_target(target: &str, cwd: &Path) -> Result<(String, std::path::PathBuf), AppError> {
    let is_path = target.starts_with('.')
        || target.starts_with('/')
        || target.starts_with('~')
        || cwd.join(target).is_dir();

    if is_path {
        let raw = if let Some(stripped) = target.strip_prefix('~') {
            let home = dirs::home_dir()
                .ok_or_else(|| AppError::Other("Cannot determine home directory".into()))?;
            home.join(stripped.strip_prefix('/').unwrap_or(stripped))
        } else {
            cwd.join(target)
        };
        let resolved = raw
            .canonicalize()
            .map_err(|_| AppError::Other(format!("Path does not exist: {target}")))?;
        let m = manifest::load(&resolved)?;
        Ok((m.name, resolved))
    } else {
        let entry = links::lookup(target)?.ok_or_else(|| {
            AppError::Other(format!(
                "{target} is not registered globally. Run `apkg link` in the package directory first, or provide a path."
            ))
        })?;
        let source = std::path::PathBuf::from(&entry.path);
        if !source.exists() {
            return Err(AppError::Other(format!(
                "Linked path no longer exists: {}",
                entry.path
            )));
        }
        Ok((target.to_string(), source))
    }
}

fn create_symlink(target: &Path, link: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)?;
    }
    Ok(())
}

fn remove_symlink(link: &Path) -> Result<(), AppError> {
    // On Unix, symlinks (even to directories) are removed with remove_file.
    // On Windows, symlinks to directories need remove_dir.
    #[cfg(unix)]
    {
        fs::remove_file(link)?;
    }
    #[cfg(windows)]
    {
        fs::remove_dir(link)?;
    }
    Ok(())
}

/// Remove empty parent directories up to (but not including) `stop_at`.
fn cleanup_empty_parents(path: &Path, stop_at: &Path) {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent == stop_at {
            break;
        }
        if fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none()) {
            let _ = fs::remove_dir(parent);
        } else {
            break;
        }
        current = parent.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{Manifest, PackageType};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        match crate::test_utils::ENV_LOCK.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_manifest(dir: &Path, name: &str) {
        let m = Manifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            package_type: PackageType::Skill,
            description: String::new(),
            license: "MIT".to_string(),
            readme: None,
            keywords: None,
            authors: None,
            repository: None,
            homepage: None,
            platform: vec!["claude-code".to_string()],
            dependencies: None,
            dev_dependencies: None,
            peer_dependencies: None,
            scripts: None,
            hook_permissions: None,
        };
        crate::config::manifest::save(dir, &m).unwrap();
    }

    /// Anchor CWD to `CARGO_MANIFEST_DIR` on drop so we never leave a stale
    /// tempdir as the process CWD (which would poison ENV_LOCK for later tests).
    struct CwdGuard;
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
        }
    }

    #[test]
    fn test_run_link_register_writes_global_entry() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let pkg_dir = tmp.path().join("my-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        write_manifest(&pkg_dir, "@test/my-pkg");

        let _cwd = CwdGuard;
        std::env::set_current_dir(&pkg_dir).unwrap();

        run_link(&LinkAction::Register).unwrap();

        let entry = links::lookup("@test/my-pkg").unwrap().expect("registered");
        assert_eq!(
            entry.path,
            pkg_dir.canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn test_run_unlink_unregister_success() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let pkg_dir = tmp.path().join("my-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        write_manifest(&pkg_dir, "@test/my-pkg");
        links::register("@test/my-pkg", &pkg_dir.canonicalize().unwrap()).unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(&pkg_dir).unwrap();

        run_unlink(&UnlinkAction::Unregister).unwrap();

        assert!(links::lookup("@test/my-pkg").unwrap().is_none());
    }

    #[test]
    fn test_run_unlink_unregister_when_not_registered() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let pkg_dir = tmp.path().join("my-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        write_manifest(&pkg_dir, "@test/my-pkg");

        let _cwd = CwdGuard;
        std::env::set_current_dir(&pkg_dir).unwrap();

        // Warn branch — still Ok.
        run_unlink(&UnlinkAction::Unregister).unwrap();
    }

    #[test]
    fn test_run_link_target_by_relative_path() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        let source = tmp.path().join("source");
        fs::create_dir_all(&consumer).unwrap();
        fs::create_dir_all(&source).unwrap();
        write_manifest(&source, "@test/source-pkg");

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        run_link(&LinkAction::LinkTarget {
            target: "../source",
        })
        .unwrap();

        let link = consumer.join("apkg_packages/@test/source-pkg");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn test_run_link_target_by_registered_name() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        write_manifest(&source, "@test/source-pkg");
        links::register("@test/source-pkg", &source.canonicalize().unwrap()).unwrap();

        let consumer = tmp.path().join("consumer");
        fs::create_dir_all(&consumer).unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        run_link(&LinkAction::LinkTarget {
            target: "@test/source-pkg",
        })
        .unwrap();

        let link = consumer.join("apkg_packages/@test/source-pkg");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    }

    #[test]
    fn test_run_link_target_relinks_existing_symlink() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        let source_a = tmp.path().join("source-a");
        let source_b = tmp.path().join("source-b");
        fs::create_dir_all(&consumer).unwrap();
        fs::create_dir_all(&source_a).unwrap();
        fs::create_dir_all(&source_b).unwrap();
        write_manifest(&source_a, "@test/pkg");
        write_manifest(&source_b, "@test/pkg");

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        run_link(&LinkAction::LinkTarget {
            target: "../source-a",
        })
        .unwrap();
        // Second call must replace, not error on existing symlink.
        run_link(&LinkAction::LinkTarget {
            target: "../source-b",
        })
        .unwrap();

        let link = consumer.join("apkg_packages/@test/pkg");
        let resolved = link.canonicalize().unwrap();
        assert_eq!(resolved, source_b.canonicalize().unwrap());
    }

    #[test]
    fn test_run_link_target_missing_path_errors() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        fs::create_dir_all(&consumer).unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        // Absolute path that doesn't exist is treated as a path (starts with '/').
        let err = run_link(&LinkAction::LinkTarget {
            target: "/definitely/nonexistent/path",
        })
        .unwrap_err();
        assert!(err.to_string().contains("Path does not exist"));
    }

    #[test]
    fn test_run_link_target_unregistered_name_errors() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        fs::create_dir_all(&consumer).unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        let err = run_link(&LinkAction::LinkTarget {
            target: "@nope/missing",
        })
        .unwrap_err();
        assert!(err.to_string().contains("not registered globally"));
    }

    #[test]
    fn test_run_unlink_package_removes_symlink_and_empty_parent() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        let source = tmp.path().join("source");
        fs::create_dir_all(&consumer).unwrap();
        fs::create_dir_all(&source).unwrap();
        write_manifest(&source, "@test/pkg");

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();
        run_link(&LinkAction::LinkTarget {
            target: "../source",
        })
        .unwrap();

        run_unlink(&UnlinkAction::UnlinkPackage {
            package: "@test/pkg",
        })
        .unwrap();

        // Symlink is gone AND the @test scope dir is cleaned up (empty parent).
        assert!(!consumer.join("apkg_packages/@test/pkg").exists());
        assert!(!consumer.join("apkg_packages/@test").exists());
    }

    #[test]
    fn test_run_unlink_package_errors_when_not_linked() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let consumer = tmp.path().join("consumer");
        fs::create_dir_all(&consumer).unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(&consumer).unwrap();

        let err = run_unlink(&UnlinkAction::UnlinkPackage {
            package: "@test/pkg",
        })
        .unwrap_err();
        assert!(err.to_string().contains("is not a linked package"));
    }
}
