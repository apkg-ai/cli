use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub registry: String,
    pub access_token: String,
    pub refresh_token: String,
    pub username: String,
}

fn credentials_path() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Other("Cannot determine home directory".into()))?;
    Ok(home.join(".apkg").join("credentials.json"))
}

pub fn load() -> Result<Option<Credentials>, AppError> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let creds: Credentials = serde_json::from_str(&content)?;
    Ok(Some(creds))
}

pub fn save(creds: &Credentials) -> Result<(), AppError> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(creds)?;
    fs::write(&path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

pub fn remove() -> Result<bool, AppError> {
    let path = credentials_path()?;
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

    fn sample_credentials() -> Credentials {
        Credentials {
            registry: "https://registry.apkg.ai/api/v1".to_string(),
            access_token: "tok_abc123".to_string(),
            refresh_token: "rt_def456".to_string(),
            username: "testuser".to_string(),
        }
    }

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let creds_dir = tmp.path().join(".apkg");
        fs::create_dir_all(&creds_dir).unwrap();
        let creds_file = creds_dir.join("credentials.json");

        let creds = sample_credentials();

        let content = serde_json::to_string_pretty(&creds).unwrap();
        fs::write(&creds_file, &content).unwrap();

        let loaded: Credentials =
            serde_json::from_str(&fs::read_to_string(&creds_file).unwrap()).unwrap();
        assert_eq!(loaded.username, "testuser");
        assert_eq!(loaded.access_token, "tok_abc123");
        assert_eq!(loaded.registry, "https://registry.apkg.ai/api/v1");
    }

    #[test]
    fn test_credentials_json_format() {
        let json = r#"{
            "registry": "https://registry.apkg.ai/api/v1",
            "accessToken": "tok_abc",
            "refreshToken": "rt_def",
            "username": "alice"
        }"#;
        let creds: Credentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.username, "alice");
        assert_eq!(creds.access_token, "tok_abc");
    }

    #[test]
    fn test_load_returns_none_when_missing() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let result = load().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_reads_existing_file() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        save(&sample_credentials()).unwrap();

        let loaded = load().unwrap().expect("credentials should be present");
        assert_eq!(loaded.username, "testuser");
        assert_eq!(loaded.access_token, "tok_abc123");
        assert_eq!(loaded.refresh_token, "rt_def456");
        assert_eq!(loaded.registry, "https://registry.apkg.ai/api/v1");
    }

    #[test]
    fn test_save_creates_parent_dir_and_file() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        save(&sample_credentials()).unwrap();

        let expected = tmp.path().join(".apkg").join("credentials.json");
        assert!(expected.exists(), "credentials.json should be created");

        let content = fs::read_to_string(&expected).unwrap();
        // Persisted file must use camelCase keys so other clients can read it.
        assert!(content.contains("\"accessToken\""));
        assert!(content.contains("\"refreshToken\""));
    }

    #[test]
    fn test_remove_returns_true_when_file_exists() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        save(&sample_credentials()).unwrap();
        let removed = remove().unwrap();

        assert!(removed);
        assert!(!tmp.path().join(".apkg").join("credentials.json").exists());
    }

    #[test]
    fn test_remove_returns_false_when_missing() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let removed = remove().unwrap();
        assert!(!removed);
    }

    #[cfg(unix)]
    #[test]
    fn test_save_sets_unix_permissions_to_0600() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        save(&sample_credentials()).unwrap();

        let path = tmp.path().join(".apkg").join("credentials.json");
        let mode = path.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "credentials.json must be user-only readable");
    }
}
