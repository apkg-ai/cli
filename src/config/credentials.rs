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

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let creds_dir = tmp.path().join(".apkg");
        fs::create_dir_all(&creds_dir).unwrap();
        let creds_file = creds_dir.join("credentials.json");

        let creds = Credentials {
            registry: "https://registry.apkg.ai/api/v1".to_string(),
            access_token: "tok_abc123".to_string(),
            refresh_token: "rt_def456".to_string(),
            username: "testuser".to_string(),
        };

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
}
