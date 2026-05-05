use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkEntry {
    pub name: String,
    pub path: String,
    pub linked_at: String,
}

/// Return the global link-store directory (`~/.apkg/links/`).
pub fn links_dir() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Environment("Cannot determine home directory".into()))?;
    Ok(home.join(".apkg").join("links"))
}

fn entry_path(name: &str) -> Result<PathBuf, AppError> {
    let encoded = urlencoding::encode(name);
    let filename = format!("{encoded}.json");
    Ok(links_dir()?.join(filename))
}

/// Register a package in the global link store.
pub fn register(name: &str, path: &Path) -> Result<(), AppError> {
    let dir = links_dir()?;
    fs::create_dir_all(&dir)?;

    let abs = path.to_string_lossy().into_owned();
    let entry = LinkEntry {
        name: name.to_string(),
        path: abs,
        linked_at: chrono::Utc::now().to_rfc3339(),
    };

    let json = serde_json::to_string_pretty(&entry)?;
    fs::write(entry_path(name)?, json)?;
    Ok(())
}

/// Unregister a package from the global link store.
/// Returns `true` if the entry existed and was removed, `false` if not found.
pub fn unregister(name: &str) -> Result<bool, AppError> {
    let path = entry_path(name)?;
    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Look up a package in the global link store.
pub fn lookup(name: &str) -> Result<Option<LinkEntry>, AppError> {
    let path = entry_path(name)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let entry: LinkEntry = serde_json::from_str(&content).map_err(|e| AppError::Parse {
        what: "link entry".into(),
        cause: e.to_string(),
    })?;
    Ok(Some(entry))
}

/// List all registered links, sorted by name.
#[cfg(test)]
pub fn list() -> Result<Vec<LinkEntry>, AppError> {
    let dir = links_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for item in fs::read_dir(&dir)? {
        let item = item?;
        let path = item.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let content = fs::read_to_string(&path)?;
            if let Ok(entry) = serde_json::from_str::<LinkEntry>(&content) {
                entries.push(entry);
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    fn with_temp_home<F>(f: F)
    where
        F: FnOnce(),
    {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };
        f();
        unsafe { std::env::remove_var("HOME") };
    }

    #[test]
    fn test_links_dir() {
        with_temp_home(|| {
            let dir = links_dir().unwrap();
            assert!(dir.ends_with(".apkg/links"));
        });
    }

    #[test]
    fn test_register_and_lookup() {
        with_temp_home(|| {
            let tmp = tempfile::tempdir().unwrap();
            register("my-lib", tmp.path()).unwrap();
            let entry = lookup("my-lib").unwrap().expect("should exist");
            assert_eq!(entry.name, "my-lib");
            assert_eq!(entry.path, tmp.path().to_string_lossy());
            assert!(!entry.linked_at.is_empty());
        });
    }

    #[test]
    fn test_register_scoped() {
        with_temp_home(|| {
            let tmp = tempfile::tempdir().unwrap();
            register("@scope/pkg", tmp.path()).unwrap();
            let entry = lookup("@scope/pkg").unwrap().expect("should exist");
            assert_eq!(entry.name, "@scope/pkg");
        });
    }

    #[test]
    fn test_lookup_missing() {
        with_temp_home(|| {
            let result = lookup("nonexistent").unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_unregister() {
        with_temp_home(|| {
            let tmp = tempfile::tempdir().unwrap();
            register("my-lib", tmp.path()).unwrap();
            let removed = unregister("my-lib").unwrap();
            assert!(removed);
            let entry = lookup("my-lib").unwrap();
            assert!(entry.is_none());
        });
    }

    #[test]
    fn test_unregister_missing() {
        with_temp_home(|| {
            let removed = unregister("nonexistent").unwrap();
            assert!(!removed);
        });
    }

    #[test]
    fn test_list_entries() {
        with_temp_home(|| {
            let tmp1 = tempfile::tempdir().unwrap();
            let tmp2 = tempfile::tempdir().unwrap();
            register("beta-pkg", tmp1.path()).unwrap();
            register("alpha-pkg", tmp2.path()).unwrap();

            let entries = list().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "alpha-pkg");
            assert_eq!(entries[1].name, "beta-pkg");
        });
    }
}
