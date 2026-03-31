use std::fs;
use std::io::Write;
use std::path::Path;

use glob_match::glob_match;

use crate::error::AppError;

const DEFAULT_IGNORE: &[&str] = &[
    ".git/**",
    "node_modules/**",
    "target/**",
    "*.tar.zst",
    ".apkgignore",
    ".DS_Store",
    "apkg_packages/**",
];

/// Create a `.tar.zst` tarball from the given directory, returning the bytes.
pub fn create_tarball(dir: &Path) -> Result<Vec<u8>, AppError> {
    let ignore_patterns = load_ignore_patterns(dir);

    let buf = Vec::new();
    let enc = zstd::Encoder::new(buf, 19)
        .map_err(|e| AppError::Other(format!("Failed to create zstd encoder: {e}")))?;
    let mut archive = tar::Builder::new(enc);

    add_dir_recursive(&mut archive, dir, dir, &ignore_patterns)?;

    let enc = archive
        .into_inner()
        .map_err(|e| AppError::Other(format!("Failed to finalize tarball: {e}")))?;
    let compressed = enc
        .finish()
        .map_err(|e| AppError::Other(format!("Failed to compress tarball: {e}")))?;

    Ok(compressed)
}

/// Write tarball bytes to a file.
pub fn write_tarball(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let mut file = fs::File::create(path)?;
    file.write_all(data)?;
    Ok(())
}

/// Extract a `.tar.zst` tarball into the given directory.
pub fn extract_tarball(data: &[u8], dest: &Path) -> Result<(), AppError> {
    use std::io::Cursor;

    fs::create_dir_all(dest)?;

    let cursor = Cursor::new(data);
    let decoder = zstd::Decoder::new(cursor)
        .map_err(|e| AppError::Other(format!("Failed to create zstd decoder: {e}")))?;
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(dest)
        .map_err(|e| AppError::Other(format!("Failed to extract tarball: {e}")))?;

    Ok(())
}

fn load_ignore_patterns(dir: &Path) -> Vec<String> {
    let ignore_file = dir.join(".apkgignore");
    if let Ok(content) = fs::read_to_string(ignore_file) {
        content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(std::string::ToString::to_string)
            .collect()
    } else {
        DEFAULT_IGNORE
            .iter()
            .map(std::string::ToString::to_string)
            .collect()
    }
}

fn should_ignore(relative_path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if glob_match(pattern, relative_path) {
            return true;
        }
    }
    false
}

fn add_dir_recursive<W: Write>(
    archive: &mut tar::Builder<W>,
    base: &Path,
    current: &Path,
    ignore_patterns: &[String],
) -> Result<(), AppError> {
    let entries = fs::read_dir(current).map_err(|e| {
        AppError::Other(format!(
            "Failed to read directory {}: {e}",
            current.display()
        ))
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .map_err(|e| AppError::Other(format!("Path strip error: {e}")))?;
        let relative_str = relative.to_string_lossy().to_string();

        if should_ignore(&relative_str, ignore_patterns) {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            add_dir_recursive(archive, base, &path, ignore_patterns)?;
        } else if metadata.is_file() {
            archive
                .append_path_with_name(&path, &relative_str)
                .map_err(|e| {
                    AppError::Other(format!("Failed to add {relative_str} to tarball: {e}"))
                })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_should_ignore_default_patterns() {
        let patterns: Vec<String> = DEFAULT_IGNORE
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        assert!(should_ignore(".git/config", &patterns));
        assert!(should_ignore("node_modules/foo/bar.js", &patterns));
        assert!(should_ignore("target/debug/apkg", &patterns));
        assert!(should_ignore("package-0.1.0.tar.zst", &patterns));
        assert!(!should_ignore("src/main.rs", &patterns));
        assert!(!should_ignore("apkg.json", &patterns));
    }

    #[test]
    fn test_create_and_extract_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        fs::write(dir.join("apkg.json"), r#"{"name":"test"}"#).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();

        let tarball = create_tarball(dir).unwrap();
        assert!(!tarball.is_empty());
        // Verify zstd magic bytes
        assert_eq!(&tarball[..4], [0x28, 0xB5, 0x2F, 0xFD]);

        let extract_dir = tmp.path().join("extracted");
        extract_tarball(&tarball, &extract_dir).unwrap();
        assert!(extract_dir.join("apkg.json").exists());
        assert!(extract_dir.join("src/main.rs").exists());
    }

    #[test]
    fn test_extract_invalid_data_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_data = b"this is not a valid tarball";
        let result = extract_tarball(bad_data, tmp.path());
        assert!(result.is_err());
    }
}
