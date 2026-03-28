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
}
