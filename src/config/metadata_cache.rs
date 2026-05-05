use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::cache::cache_dir;
use crate::error::AppError;

const METADATA_SUBDIR: &str = "metadata";

/// Default TTL: short enough that new versions surface within minutes, long
/// enough that `info` → `add` → `install` in rapid succession reuses one fetch.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnDisk {
    /// RFC3339.
    fetched_at: String,
    /// Raw JSON body returned by `/packages/{name}`.
    body: String,
}

#[derive(Debug)]
pub struct CachedMetadata {
    pub body: String,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
}

impl CachedMetadata {
    pub fn is_fresh(&self, ttl: Duration) -> bool {
        let age = chrono::Utc::now().signed_duration_since(self.fetched_at);
        age.num_seconds() >= 0 && (age.num_seconds() as u64) < ttl.as_secs()
    }
}

/// Respect `APKG_NO_METADATA_CACHE=1` to disable entirely.
pub fn is_disabled() -> bool {
    std::env::var("APKG_NO_METADATA_CACHE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Override default TTL via `APKG_METADATA_TTL_SECS`.
pub fn configured_ttl() -> Duration {
    std::env::var("APKG_METADATA_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_TTL)
}

fn entry_path(name: &str) -> Result<PathBuf, AppError> {
    let encoded = urlencoding::encode(name).into_owned();
    Ok(cache_dir()?
        .join(METADATA_SUBDIR)
        .join(format!("{encoded}.json")))
}

/// Load a cached entry if present. Does NOT check TTL — callers decide freshness.
pub fn load(name: &str) -> Result<Option<CachedMetadata>, AppError> {
    let path = entry_path(name)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let on_disk: OnDisk = serde_json::from_str(&raw).map_err(|e| AppError::Parse {
        what: "cached metadata".into(),
        cause: e.to_string(),
    })?;
    let fetched_at = chrono::DateTime::parse_from_rfc3339(&on_disk.fetched_at)
        .map_err(|e| AppError::Parse {
            what: "cached metadata timestamp".into(),
            cause: e.to_string(),
        })?
        .with_timezone(&chrono::Utc);
    Ok(Some(CachedMetadata {
        body: on_disk.body,
        fetched_at,
    }))
}

pub fn store(name: &str, body: &str) -> Result<(), AppError> {
    let path = entry_path(name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let on_disk = OnDisk {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        body: body.to_string(),
    };
    // Best-effort atomicity: write to a temp sibling then rename.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string(&on_disk)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    fn with_temp_cache<F: FnOnce()>(f: F) {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("APKG_CACHE_DIR", tmp.path()) };
        unsafe { std::env::remove_var("APKG_NO_METADATA_CACHE") };
        unsafe { std::env::remove_var("APKG_METADATA_TTL_SECS") };
        f();
        unsafe { std::env::remove_var("APKG_CACHE_DIR") };
    }

    #[test]
    fn test_roundtrip() {
        with_temp_cache(|| {
            let body = r#"{"name":"x","versions":{}}"#;
            store("x", body).unwrap();
            let loaded = load("x").unwrap().expect("entry");
            assert_eq!(loaded.body, body);
            assert!(loaded.is_fresh(DEFAULT_TTL));
        });
    }

    #[test]
    fn test_scoped_name_is_encoded() {
        with_temp_cache(|| {
            store("@acme/foo", "{}").unwrap();
            assert!(load("@acme/foo").unwrap().is_some());
            // Filename must not embed raw '@acme/foo' (the '/' would create a subdir).
            let p = entry_path("@acme/foo").unwrap();
            assert!(!p.to_string_lossy().contains("@acme/foo"));
        });
    }

    #[test]
    fn test_is_fresh_respects_ttl() {
        let now = chrono::Utc::now();
        let old = CachedMetadata {
            body: String::new(),
            fetched_at: now - chrono::Duration::seconds(600),
        };
        assert!(!old.is_fresh(Duration::from_secs(60)));
        let young = CachedMetadata {
            body: String::new(),
            fetched_at: now - chrono::Duration::seconds(30),
        };
        assert!(young.is_fresh(Duration::from_secs(60)));
    }

    #[test]
    fn test_missing_entry_returns_none() {
        with_temp_cache(|| {
            assert!(load("nope").unwrap().is_none());
        });
    }

    #[test]
    fn test_corrupt_entry_is_parse_error() {
        with_temp_cache(|| {
            let path = entry_path("bad").unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"not json").unwrap();
            let err = load("bad").unwrap_err();
            assert!(matches!(err, AppError::Parse { .. }));
        });
    }

    #[test]
    fn test_is_disabled_env_toggle() {
        let _lock = env_lock();
        unsafe { std::env::set_var("APKG_NO_METADATA_CACHE", "1") };
        assert!(is_disabled());
        unsafe { std::env::set_var("APKG_NO_METADATA_CACHE", "0") };
        assert!(!is_disabled());
        unsafe { std::env::remove_var("APKG_NO_METADATA_CACHE") };
        assert!(!is_disabled());
    }

    #[test]
    fn test_configured_ttl_env_override() {
        let _lock = env_lock();
        unsafe { std::env::set_var("APKG_METADATA_TTL_SECS", "42") };
        assert_eq!(configured_ttl(), Duration::from_secs(42));
        unsafe { std::env::remove_var("APKG_METADATA_TTL_SECS") };
        assert_eq!(configured_ttl(), DEFAULT_TTL);
    }
}
