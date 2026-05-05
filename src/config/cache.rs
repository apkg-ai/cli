use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::util::integrity::sha256_integrity;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMetadata {
    pub integrity: String,
    pub cached_at: String,
}

pub struct CacheEntry {
    pub data: Vec<u8>,
    pub integrity: String,
    #[allow(dead_code)]
    pub cached_at: String,
}

pub struct CacheEntryInfo {
    pub name: String,
    pub version: String,
    pub size: u64,
    pub cached_at: String,
}

pub struct CacheCleanResult {
    pub count: usize,
    pub bytes_freed: u64,
}

pub struct CacheVerifyResult {
    pub checked: usize,
    pub ok: usize,
    pub corrupted: Vec<String>,
}

pub fn cache_dir() -> Result<PathBuf, AppError> {
    if let Ok(dir) = std::env::var("APKG_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Environment("Cannot determine home directory".into()))?;
    Ok(home.join(".apkg").join("cache"))
}

pub fn entry_dir(name: &str, version: &str) -> Result<PathBuf, AppError> {
    let encoded = urlencoding::encode(name);
    Ok(cache_dir()?.join(encoded.as_ref()).join(version))
}

pub fn store(name: &str, version: &str, data: &[u8], integrity: &str) -> Result<(), AppError> {
    let dir = entry_dir(name, version)?;
    fs::create_dir_all(&dir)?;

    fs::write(dir.join("package.tar.zst"), data)?;

    let metadata = CacheMetadata {
        integrity: integrity.to_string(),
        cached_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&metadata)?;
    fs::write(dir.join("metadata.json"), json)?;

    Ok(())
}

pub fn load(name: &str, version: &str) -> Result<Option<CacheEntry>, AppError> {
    let dir = entry_dir(name, version)?;
    let tarball_path = dir.join("package.tar.zst");
    let metadata_path = dir.join("metadata.json");

    if !tarball_path.exists() || !metadata_path.exists() {
        return Ok(None);
    }

    let data = fs::read(&tarball_path)?;
    let meta_content = fs::read_to_string(&metadata_path)?;
    let meta: CacheMetadata = serde_json::from_str(&meta_content).map_err(|e| AppError::Parse {
        what: "cache metadata".into(),
        cause: e.to_string(),
    })?;

    Ok(Some(CacheEntry {
        data,
        integrity: meta.integrity,
        cached_at: meta.cached_at,
    }))
}

pub fn list_entries() -> Result<Vec<CacheEntryInfo>, AppError> {
    let base = cache_dir()?;
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();

    for pkg_entry in fs::read_dir(&base)? {
        let pkg_entry = pkg_entry?;
        if !pkg_entry.file_type()?.is_dir() {
            continue;
        }
        let encoded_name = pkg_entry.file_name();
        let lossy = encoded_name.to_string_lossy().into_owned();
        let name = urlencoding::decode(&lossy)
            .map_or_else(|_| lossy.clone(), std::borrow::Cow::into_owned);

        for ver_entry in fs::read_dir(pkg_entry.path())? {
            let ver_entry = ver_entry?;
            if !ver_entry.file_type()?.is_dir() {
                continue;
            }
            let version = ver_entry.file_name().to_string_lossy().into_owned();
            let tarball_path = ver_entry.path().join("package.tar.zst");
            let metadata_path = ver_entry.path().join("metadata.json");

            if !tarball_path.exists() || !metadata_path.exists() {
                continue;
            }

            let size = fs::metadata(&tarball_path)?.len();
            let meta_content = fs::read_to_string(&metadata_path)?;
            let meta: CacheMetadata = match serde_json::from_str(&meta_content) {
                Ok(m) => m,
                Err(_) => continue,
            };

            entries.push(CacheEntryInfo {
                name: name.clone(),
                version,
                size,
                cached_at: meta.cached_at,
            });
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
    Ok(entries)
}

pub fn remove_entry(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub fn clean() -> Result<CacheCleanResult, AppError> {
    let base = cache_dir()?;
    if !base.exists() {
        return Ok(CacheCleanResult {
            count: 0,
            bytes_freed: 0,
        });
    }

    let entries = list_entries()?;
    let count = entries.len();
    let bytes_freed: u64 = entries.iter().map(|e| e.size).sum();

    if count > 0 {
        fs::remove_dir_all(&base)?;
        fs::create_dir_all(&base)?;
    }

    Ok(CacheCleanResult { count, bytes_freed })
}

pub fn verify() -> Result<CacheVerifyResult, AppError> {
    let entries = list_entries()?;
    let checked = entries.len();
    let mut ok = 0;
    let mut corrupted = Vec::new();

    for entry in &entries {
        let dir = entry_dir(&entry.name, &entry.version)?;
        let tarball_path = dir.join("package.tar.zst");
        let metadata_path = dir.join("metadata.json");

        let data = fs::read(&tarball_path)?;
        let meta_content = fs::read_to_string(&metadata_path)?;
        let Ok(meta) = serde_json::from_str::<CacheMetadata>(&meta_content) else {
            let label = format!("{}@{}", entry.name, entry.version);
            corrupted.push(label);
            remove_entry(&dir)?;
            continue;
        };

        let actual = sha256_integrity(&data);
        if actual == meta.integrity {
            ok += 1;
        } else {
            let label = format!("{}@{}", entry.name, entry.version);
            corrupted.push(label);
            remove_entry(&dir)?;
        }
    }

    Ok(CacheVerifyResult {
        checked,
        ok,
        corrupted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    fn with_temp_cache<F>(f: F)
    where
        F: FnOnce(),
    {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("cache");
        unsafe { std::env::set_var("APKG_CACHE_DIR", &cache_path) };
        f();
        unsafe { std::env::remove_var("APKG_CACHE_DIR") };
    }

    #[test]
    fn test_cache_dir_default() {
        let _lock = env_lock();
        unsafe { std::env::remove_var("APKG_CACHE_DIR") };
        let dir = cache_dir().unwrap();
        assert!(dir.ends_with(".apkg/cache"));
    }

    #[test]
    fn test_cache_dir_env_override() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("my-cache");
        unsafe { std::env::set_var("APKG_CACHE_DIR", &custom) };
        let dir = cache_dir().unwrap();
        assert_eq!(dir, custom);
        unsafe { std::env::remove_var("APKG_CACHE_DIR") };
    }

    #[test]
    fn test_store_and_load() {
        with_temp_cache(|| {
            let data = b"fake tarball data";
            let integrity = sha256_integrity(data);
            store("my-pkg", "1.0.0", data, &integrity).unwrap();

            let loaded = load("my-pkg", "1.0.0")
                .unwrap()
                .expect("entry should exist");
            assert_eq!(loaded.data, data);
            assert_eq!(loaded.integrity, integrity);
            assert!(!loaded.cached_at.is_empty());
        });
    }

    #[test]
    fn test_store_scoped_package() {
        with_temp_cache(|| {
            let data = b"scoped pkg data";
            let integrity = sha256_integrity(data);
            store("@acme/summarizer", "2.1.0", data, &integrity).unwrap();

            let loaded = load("@acme/summarizer", "2.1.0")
                .unwrap()
                .expect("scoped entry should exist");
            assert_eq!(loaded.data, data);
            assert_eq!(loaded.integrity, integrity);
        });
    }

    #[test]
    fn test_load_nonexistent() {
        with_temp_cache(|| {
            let result = load("nonexistent", "0.0.0").unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_list_entries() {
        with_temp_cache(|| {
            let data1 = b"pkg one data";
            let data2 = b"pkg two longer data here";
            store("alpha", "1.0.0", data1, &sha256_integrity(data1)).unwrap();
            store("beta", "2.3.4", data2, &sha256_integrity(data2)).unwrap();

            let entries = list_entries().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "alpha");
            assert_eq!(entries[0].version, "1.0.0");
            assert_eq!(entries[0].size, data1.len() as u64);
            assert_eq!(entries[1].name, "beta");
            assert_eq!(entries[1].version, "2.3.4");
            assert_eq!(entries[1].size, data2.len() as u64);
        });
    }

    #[test]
    fn test_clean() {
        with_temp_cache(|| {
            let data = b"some data here!";
            store("pkg-a", "1.0.0", data, &sha256_integrity(data)).unwrap();
            store("pkg-b", "0.5.0", data, &sha256_integrity(data)).unwrap();

            let result = clean().unwrap();
            assert_eq!(result.count, 2);
            assert_eq!(result.bytes_freed, data.len() as u64 * 2);

            let entries = list_entries().unwrap();
            assert!(entries.is_empty());
        });
    }

    #[test]
    fn test_clean_also_wipes_metadata_subdir() {
        use crate::config::metadata_cache;
        with_temp_cache(|| {
            // Populate both: a tarball entry and a metadata entry.
            let tarball = b"tarball bytes";
            store("pkg-a", "1.0.0", tarball, &sha256_integrity(tarball)).unwrap();
            metadata_cache::store("pkg-a", r#"{"name":"pkg-a"}"#).unwrap();

            assert!(metadata_cache::load("pkg-a").unwrap().is_some());

            let result = clean().unwrap();
            assert!(result.count >= 1);

            // Both kinds of entries should now be gone.
            assert!(list_entries().unwrap().is_empty());
            assert!(metadata_cache::load("pkg-a").unwrap().is_none());
        });
    }

    #[test]
    fn test_clean_empty() {
        with_temp_cache(|| {
            let result = clean().unwrap();
            assert_eq!(result.count, 0);
            assert_eq!(result.bytes_freed, 0);
        });
    }

    #[test]
    fn test_verify_all_ok() {
        with_temp_cache(|| {
            let data = b"valid package data";
            let integrity = sha256_integrity(data);
            store("valid-pkg", "1.0.0", data, &integrity).unwrap();

            let result = verify().unwrap();
            assert_eq!(result.checked, 1);
            assert_eq!(result.ok, 1);
            assert!(result.corrupted.is_empty());
        });
    }

    #[test]
    fn test_verify_corrupted() {
        with_temp_cache(|| {
            let data = b"original data";
            let integrity = sha256_integrity(data);
            store("bad-pkg", "1.0.0", data, &integrity).unwrap();

            // Corrupt the tarball
            let dir = entry_dir("bad-pkg", "1.0.0").unwrap();
            fs::write(dir.join("package.tar.zst"), b"corrupted!").unwrap();

            let result = verify().unwrap();
            assert_eq!(result.checked, 1);
            assert_eq!(result.ok, 0);
            assert_eq!(result.corrupted.len(), 1);
            assert_eq!(result.corrupted[0], "bad-pkg@1.0.0");

            // Entry should have been removed
            assert!(!dir.exists());
        });
    }
}
