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
