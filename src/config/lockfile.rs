use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const LOCKFILE_NAME: &str = "qpm-lock.json";
pub const LOCKFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub struct Lockfile {
    pub lockfile_version: u32,
    pub requires: bool,
    pub resolved: String,
    pub packages: BTreeMap<String, LockedPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedPackage {
    pub version: String,
    pub resolved: String,
    pub integrity: String,
    pub dependencies: BTreeMap<String, String>,
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(rename = "type")]
    pub package_type: String,
    #[serde(default)]
    pub optional: bool,
}

/// Build a lockfile key from a package name and version: `"name@version"`.
pub fn lock_key(name: &str, version: &str) -> String {
    format!("{name}@{version}")
}

/// Load a lockfile from the given directory. Returns `Ok(None)` if the file
/// does not exist.
pub fn load(dir: &Path) -> Result<Option<Lockfile>, AppError> {
    let path = dir.join(LOCKFILE_NAME);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| AppError::Other(format!("Failed to read {LOCKFILE_NAME}: {e}")))?;
    let lockfile: Lockfile = serde_json::from_str(&content)
        .map_err(|e| AppError::Other(format!("Invalid {LOCKFILE_NAME}: {e}")))?;
    Ok(Some(lockfile))
}

/// Write a lockfile to the given directory.
pub fn save(dir: &Path, lockfile: &Lockfile) -> Result<(), AppError> {
    let path = dir.join(LOCKFILE_NAME);
    let content = serde_json::to_string_pretty(lockfile)?;
    fs::write(&path, format!("{content}\n"))?;
    Ok(())
}

/// Find the locked entry for a package name (any version).
/// Splits each key on the last `@` so scoped names like `@scope/pkg@1.0.0`
/// are handled correctly.
pub fn find_by_name<'a>(lockfile: &'a Lockfile, name: &str) -> Option<&'a LockedPackage> {
    lockfile.packages.iter().find_map(|(key, entry)| {
        if let Some(idx) = key.rfind('@') {
            if idx > 0 && key[..idx] == *name {
                return Some(entry);
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_key_unscoped() {
        assert_eq!(lock_key("my-pkg", "1.0.0"), "my-pkg@1.0.0");
    }

    #[test]
    fn test_lock_key_scoped() {
        assert_eq!(lock_key("@acme/tool", "2.3.4"), "@acme/tool@2.3.4");
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut packages = BTreeMap::new();
        packages.insert(
            "foo@1.0.0".to_string(),
            LockedPackage {
                version: "1.0.0".to_string(),
                resolved: "https://registry.qpm.dev/api/v1/packages/foo/1.0.0/tarball".to_string(),
                integrity: "sha256-abc123".to_string(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                package_type: "skill".to_string(),
                optional: false,
            },
        );
        let lf = Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: "2026-01-01T00:00:00Z".to_string(),
            packages,
        };

        let json = serde_json::to_string_pretty(&lf).unwrap();
        let parsed: Lockfile = serde_json::from_str(&json).unwrap();
        assert_eq!(lf, parsed);
    }

    #[test]
    fn test_load_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(LOCKFILE_NAME), "not json").unwrap();
        let result = load(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let mut packages = BTreeMap::new();
        packages.insert(
            "bar@2.0.0".to_string(),
            LockedPackage {
                version: "2.0.0".to_string(),
                resolved: "https://example.com/bar/2.0.0/tarball".to_string(),
                integrity: "sha256-xyz".to_string(),
                dependencies: {
                    let mut d = BTreeMap::new();
                    d.insert("baz".to_string(), "1.0.0".to_string());
                    d
                },
                peer_dependencies: BTreeMap::new(),
                package_type: "agent".to_string(),
                optional: false,
            },
        );
        let lf = Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: "2026-03-15T00:00:00Z".to_string(),
            packages,
        };

        save(tmp.path(), &lf).unwrap();
        let loaded = load(tmp.path()).unwrap().expect("lockfile should exist");
        assert_eq!(lf, loaded);
    }

    #[test]
    fn test_find_by_name_found() {
        let mut packages = BTreeMap::new();
        packages.insert(
            "my-pkg@1.0.0".to_string(),
            LockedPackage {
                version: "1.0.0".to_string(),
                resolved: String::new(),
                integrity: String::new(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                package_type: "skill".to_string(),
                optional: false,
            },
        );
        packages.insert(
            "@scope/tool@2.0.0".to_string(),
            LockedPackage {
                version: "2.0.0".to_string(),
                resolved: String::new(),
                integrity: String::new(),
                dependencies: BTreeMap::new(),
                peer_dependencies: BTreeMap::new(),
                package_type: "agent".to_string(),
                optional: false,
            },
        );
        let lf = Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: String::new(),
            packages,
        };

        let entry = find_by_name(&lf, "my-pkg").expect("should find my-pkg");
        assert_eq!(entry.version, "1.0.0");

        let entry = find_by_name(&lf, "@scope/tool").expect("should find @scope/tool");
        assert_eq!(entry.version, "2.0.0");
    }

    #[test]
    fn test_find_by_name_missing() {
        let lf = Lockfile {
            lockfile_version: LOCKFILE_VERSION,
            requires: true,
            resolved: String::new(),
            packages: BTreeMap::new(),
        };
        assert!(find_by_name(&lf, "nonexistent").is_none());
    }
}
