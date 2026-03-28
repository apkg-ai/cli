use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,

    #[serde(default)]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, String>,

    #[serde(default)]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub default_setup: BTreeMap<String, bool>,
}

impl Settings {
    pub fn service_url(&self, service: &str) -> Option<&str> {
        self.services.get(service).map(String::as_str)
    }

    pub fn base_url(&self, service: &str) -> String {
        self.service_url(service)
            .or(self.registry.as_deref())
            .unwrap_or(super::DEFAULT_REGISTRY)
            .trim_end_matches('/')
            .to_string()
    }

    pub fn set(&mut self, key: &str, value: &str) {
        if key == "registry" {
            self.registry = Some(value.to_string());
        } else if let Some(svc) = key.strip_prefix("services.") {
            self.services.insert(svc.to_string(), value.to_string());
        } else if let Some(tool) = key.strip_prefix("defaultSetup.") {
            let enabled = value == "true" || value == "1";
            self.default_setup.insert(tool.to_string(), enabled);
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        if key == "registry" {
            self.registry.as_deref()
        } else if let Some(svc) = key.strip_prefix("services.") {
            self.services.get(svc).map(String::as_str)
        } else if let Some(tool) = key.strip_prefix("defaultSetup.") {
            self.default_setup.get(tool).map(|v| if *v { "true" } else { "false" })
        } else {
            None
        }
    }

    pub fn delete(&mut self, key: &str) -> bool {
        if key == "registry" {
            let had = self.registry.is_some();
            self.registry = None;
            had
        } else if let Some(svc) = key.strip_prefix("services.") {
            self.services.remove(svc).is_some()
        } else if let Some(tool) = key.strip_prefix("defaultSetup.") {
            self.default_setup.remove(tool).is_some()
        } else {
            false
        }
    }

    pub fn entries(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        if let Some(reg) = &self.registry {
            entries.push(("registry".to_string(), reg.clone()));
        }
        for (k, v) in &self.services {
            entries.push((format!("services.{k}"), v.clone()));
        }
        for (k, v) in &self.default_setup {
            entries.push((format!("defaultSetup.{k}"), v.to_string()));
        }
        entries
    }

    /// Returns the list of tool keys explicitly enabled in `defaultSetup`.
    /// Returns `None` if `defaultSetup` is empty (meaning: use auto-detect).
    pub fn enabled_setup_tools(&self) -> Option<Vec<&str>> {
        if self.default_setup.is_empty() {
            return None;
        }
        Some(
            self.default_setup
                .iter()
                .filter(|(_, enabled)| **enabled)
                .map(|(k, _)| k.as_str())
                .collect(),
        )
    }

    pub fn load() -> Result<Self, AppError> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)?;
        let settings: Self = serde_json::from_str(&content)
            .map_err(|e| AppError::Other(format!("Invalid config.json: {e}")))?;
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), AppError> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, format!("{content}\n"))?;
        Ok(())
    }
}

fn settings_path() -> Result<PathBuf, AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Other("Cannot determine home directory".into()))?;
    Ok(home.join(".apkg").join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_base_url() {
        let s = Settings::default();
        assert_eq!(s.base_url("auth"), super::super::DEFAULT_REGISTRY);
    }

    #[test]
    fn test_registry_override() {
        let s = Settings {
            registry: Some("http://localhost:9000".to_string()),
            ..Default::default()
        };
        assert_eq!(s.base_url("auth"), "http://localhost:9000");
        assert_eq!(s.base_url("package"), "http://localhost:9000");
    }

    #[test]
    fn test_per_service_override() {
        let mut services = BTreeMap::new();
        services.insert("auth".to_string(), "http://localhost:8787".to_string());
        services.insert("package".to_string(), "http://localhost:8794".to_string());
        let s = Settings {
            registry: Some("http://localhost:9000".to_string()),
            services,
            ..Default::default()
        };
        assert_eq!(s.base_url("auth"), "http://localhost:8787");
        assert_eq!(s.base_url("package"), "http://localhost:8794");
        assert_eq!(s.base_url("search"), "http://localhost:9000");
    }

    #[test]
    fn test_set_and_get() {
        let mut s = Settings::default();
        s.set("registry", "http://example.com");
        assert_eq!(s.get("registry"), Some("http://example.com"));
        s.set("services.auth", "http://localhost:8787");
        assert_eq!(s.get("services.auth"), Some("http://localhost:8787"));
    }

    #[test]
    fn test_delete() {
        let mut s = Settings::default();
        s.set("registry", "http://example.com");
        s.set("services.auth", "http://localhost:8787");
        assert!(s.delete("services.auth"));
        assert_eq!(s.get("services.auth"), None);
        assert!(!s.delete("services.auth"));
    }

    #[test]
    fn test_entries() {
        let mut s = Settings::default();
        s.set("registry", "http://example.com");
        s.set("services.auth", "http://localhost:8787");
        let entries = s.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "registry");
        assert_eq!(entries[1].0, "services.auth");
    }

    #[test]
    fn test_roundtrip_json() {
        let mut s = Settings::default();
        s.set("registry", "https://registry.apkg.ai/api/v1");
        s.set("services.auth", "http://localhost:8787");
        s.set("services.package", "http://localhost:8794");
        let json = serde_json::to_string_pretty(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.registry, s.registry);
        assert_eq!(loaded.services.len(), 2);
    }

    #[test]
    fn test_default_setup_set_and_get() {
        let mut s = Settings::default();
        s.set("defaultSetup.cursor", "true");
        assert_eq!(s.get("defaultSetup.cursor"), Some("true"));
        s.set("defaultSetup.claude-code", "false");
        assert_eq!(s.get("defaultSetup.claude-code"), Some("false"));
    }

    #[test]
    fn test_default_setup_delete() {
        let mut s = Settings::default();
        s.set("defaultSetup.cursor", "true");
        assert!(s.delete("defaultSetup.cursor"));
        assert_eq!(s.get("defaultSetup.cursor"), None);
        assert!(!s.delete("defaultSetup.cursor"));
    }

    #[test]
    fn test_default_setup_entries() {
        let mut s = Settings::default();
        s.set("defaultSetup.cursor", "true");
        s.set("defaultSetup.claude-code", "false");
        let entries = s.entries();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|(k, v)| k == "defaultSetup.claude-code" && v == "false"));
        assert!(entries.iter().any(|(k, v)| k == "defaultSetup.cursor" && v == "true"));
    }

    #[test]
    fn test_enabled_setup_tools_empty() {
        let s = Settings::default();
        assert!(s.enabled_setup_tools().is_none());
    }

    #[test]
    fn test_enabled_setup_tools_filters() {
        let mut s = Settings::default();
        s.set("defaultSetup.cursor", "true");
        s.set("defaultSetup.claude-code", "false");
        let tools = s.enabled_setup_tools().unwrap();
        assert_eq!(tools, vec!["cursor"]);
    }

    #[test]
    fn test_default_setup_roundtrip_json() {
        let mut s = Settings::default();
        s.set("defaultSetup.cursor", "true");
        s.set("defaultSetup.claude-code", "false");
        let json = serde_json::to_string_pretty(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.default_setup.len(), 2);
        assert_eq!(loaded.default_setup.get("cursor"), Some(&true));
        assert_eq!(loaded.default_setup.get("claude-code"), Some(&false));
    }
}
