use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    pub key_id: String,
    pub name: String,
    pub public_key: String,
    pub private_key: String,
    pub created_at: String,
}

fn keys_dir() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Other("Cannot determine home directory".into()))?;
    Ok(home.join(".apkg").join("keys"))
}

fn key_filename(key_id: &str) -> String {
    key_id.replace([':', '/', '+'], "_")
}

pub fn save(key: &StoredKey) -> Result<(), AppError> {
    let dir = keys_dir()?;
    fs::create_dir_all(&dir)?;

    let filename = format!("{}.json", key_filename(&key.key_id));
    let path = dir.join(filename);
    let content = serde_json::to_string_pretty(key)?;
    fs::write(&path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

pub fn load(key_id: &str) -> Result<Option<StoredKey>, AppError> {
    let dir = keys_dir()?;
    let filename = format!("{}.json", key_filename(key_id));
    let path = dir.join(filename);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let key: StoredKey = serde_json::from_str(&content)?;
    Ok(Some(key))
}

pub fn list_local() -> Result<Vec<StoredKey>, AppError> {
    let dir = keys_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut keys = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let content = fs::read_to_string(&path)?;
            if let Ok(key) = serde_json::from_str::<StoredKey>(&content) {
                keys.push(key);
            }
        }
    }
    keys.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(keys)
}

pub fn delete(key_id: &str) -> Result<bool, AppError> {
    let dir = keys_dir()?;
    let filename = format!("{}.json", key_filename(key_id));
    let path = dir.join(filename);
    if path.exists() {
        fs::remove_file(&path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_filename_sanitizes() {
        assert_eq!(key_filename("SHA256:abc+def/ghi="), "SHA256_abc_def_ghi=");
    }

    #[test]
    fn test_stored_key_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let keys_dir = tmp.path().join(".apkg").join("keys");
        fs::create_dir_all(&keys_dir).unwrap();

        let key = StoredKey {
            key_id: "SHA256:testkey123".to_string(),
            name: "test-key".to_string(),
            public_key: "cHVibGlj".to_string(),
            private_key: "cHJpdmF0ZQ==".to_string(),
            created_at: "2026-03-14T12:00:00Z".to_string(),
        };

        let content = serde_json::to_string_pretty(&key).unwrap();
        let filename = format!("{}.json", key_filename(&key.key_id));
        fs::write(keys_dir.join(&filename), &content).unwrap();

        let loaded: StoredKey =
            serde_json::from_str(&fs::read_to_string(keys_dir.join(&filename)).unwrap()).unwrap();
        assert_eq!(loaded.key_id, "SHA256:testkey123");
        assert_eq!(loaded.name, "test-key");
        assert_eq!(loaded.public_key, "cHVibGlj");
    }

    fn make_test_key(id: &str, name: &str) -> StoredKey {
        StoredKey {
            key_id: id.to_string(),
            name: name.to_string(),
            public_key: "cHVibGlj".to_string(),
            private_key: "cHJpdmF0ZQ==".to_string(),
            created_at: "2026-03-14T12:00:00Z".to_string(),
        }
    }

    /// Save a key to a specific directory (bypasses HOME env var).
    fn save_to(dir: &std::path::Path, key: &StoredKey) {
        fs::create_dir_all(dir).unwrap();
        let filename = format!("{}.json", key_filename(&key.key_id));
        let content = serde_json::to_string_pretty(key).unwrap();
        fs::write(dir.join(filename), content).unwrap();
    }

    /// Load a key from a specific directory (bypasses HOME env var).
    fn load_from(dir: &std::path::Path, key_id: &str) -> Option<StoredKey> {
        let filename = format!("{}.json", key_filename(key_id));
        let path = dir.join(filename);
        if !path.exists() {
            return None;
        }
        let content = fs::read_to_string(&path).unwrap();
        Some(serde_json::from_str(&content).unwrap())
    }

    /// List keys from a specific directory.
    fn list_from(dir: &std::path::Path) -> Vec<StoredKey> {
        if !dir.exists() {
            return Vec::new();
        }
        let mut keys = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let content = fs::read_to_string(&path).unwrap();
                if let Ok(key) = serde_json::from_str::<StoredKey>(&content) {
                    keys.push(key);
                }
            }
        }
        keys.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        keys
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keys");
        let key = make_test_key("SHA256:abc123", "my-key");
        save_to(&dir, &key);

        let loaded = load_from(&dir, "SHA256:abc123").unwrap();
        assert_eq!(loaded.key_id, "SHA256:abc123");
        assert_eq!(loaded.name, "my-key");
        assert_eq!(loaded.public_key, "cHVibGlj");
    }

    #[test]
    fn test_load_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keys");
        fs::create_dir_all(&dir).unwrap();
        assert!(load_from(&dir, "SHA256:nope").is_none());
    }

    #[test]
    fn test_list_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keys");
        assert!(list_from(&dir).is_empty());
    }

    #[test]
    fn test_list_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keys");
        save_to(&dir, &make_test_key("SHA256:key1", "first"));
        save_to(&dir, &make_test_key("SHA256:key2", "second"));

        let keys = list_from(&dir);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_delete_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("keys");
        save_to(&dir, &make_test_key("SHA256:del", "to-delete"));

        let filename = format!("{}.json", key_filename("SHA256:del"));
        let path = dir.join(&filename);
        assert!(path.exists());
        fs::remove_file(&path).unwrap();
        assert!(load_from(&dir, "SHA256:del").is_none());
    }

    #[test]
    fn test_key_filename_special_chars() {
        assert_eq!(key_filename("SHA256:a+b/c:d"), "SHA256_a_b_c_d");
    }
}
