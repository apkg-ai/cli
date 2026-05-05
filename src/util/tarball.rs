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
        .map_err(|e| AppError::Tarball(format!("Failed to create zstd encoder: {e}")))?;
    let mut archive = tar::Builder::new(enc);

    add_dir_recursive(&mut archive, dir, dir, &ignore_patterns)?;

    let enc = archive
        .into_inner()
        .map_err(|e| AppError::Tarball(format!("Failed to finalize tarball: {e}")))?;
    let compressed = enc
        .finish()
        .map_err(|e| AppError::Tarball(format!("Failed to compress tarball: {e}")))?;

    Ok(compressed)
}

/// Write tarball bytes to a file.
pub fn write_tarball(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let mut file = fs::File::create(path)?;
    file.write_all(data)?;
    Ok(())
}

/// Hard cap on decompressed tarball size — guards against zstd bombs. Real
/// packages observed in the wild are 8-32 KB; 2 MB gives ~60× headroom while
/// keeping a malicious package from filling the disk before extraction fails.
const MAX_DECOMPRESSED_BYTES: u64 = 2 * 1024 * 1024;

/// Extract a `.tar.zst` tarball into the given directory.
///
/// Defence-in-depth: on top of the `tar` crate's built-in canonicalization,
/// each entry is validated to reject `..` path components, symlinks/hardlinks,
/// and non-regular entry types before unpacking.
pub fn extract_tarball(data: &[u8], dest: &Path) -> Result<(), AppError> {
    use std::io::{Cursor, Read};
    use std::path::Component;
    use tar::EntryType;

    fs::create_dir_all(dest)?;

    let cursor = Cursor::new(data);
    let decoder = zstd::Decoder::new(cursor)
        .map_err(|e| AppError::Tarball(format!("Failed to create zstd decoder: {e}")))?;
    let capped = decoder.take(MAX_DECOMPRESSED_BYTES);
    let mut archive = tar::Archive::new(capped);

    let entries = archive.entries().map_err(|e| {
        AppError::Tarball(format!(
            "Failed to read tarball entries (decompressed size may exceed {MAX_DECOMPRESSED_BYTES}-byte cap, or archive is malformed): {e}"
        ))
    })?;

    for entry in entries {
        let mut entry =
            entry.map_err(|e| AppError::Tarball(format!("Failed to read tarball entry: {e}")))?;

        // Reject any `..` component — `tar`'s `unpack_in` would otherwise
        // silently skip the entry, hiding the hostile intent from the user.
        let path = entry
            .path()
            .map_err(|e| AppError::Tarball(format!("Invalid entry path: {e}")))?
            .into_owned();
        let path_display = path.display().to_string();

        for part in path.components() {
            if matches!(part, Component::ParentDir) {
                return Err(AppError::Tarball(format!(
                    "refusing to extract entry '{path_display}' — path contains parent-directory reference (..)"
                )));
            }
        }

        // apkg packages have no legitimate use for links; reject them outright.
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(AppError::Tarball(format!(
                "refusing to extract entry '{path_display}' — symlinks and hardlinks are not allowed"
            )));
        }

        // Only regular files, directories, and extended-header metadata are
        // allowed. Rejects devices, FIFOs, sockets.
        if !matches!(
            entry_type,
            EntryType::Regular
                | EntryType::Directory
                | EntryType::XGlobalHeader
                | EntryType::XHeader
        ) {
            return Err(AppError::Tarball(format!(
                "refusing to extract entry '{path_display}' — entry type {entry_type:?} is not supported"
            )));
        }

        entry.unpack_in(dest).map_err(|e| {
            AppError::Tarball(format!("Failed to unpack entry '{path_display}': {e}"))
        })?;
    }

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
        AppError::Tarball(format!(
            "Failed to read directory {}: {e}",
            current.display()
        ))
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .map_err(|e| AppError::Tarball(format!("Path strip error: {e}")))?;
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
                    AppError::Tarball(format!("Failed to add {relative_str} to tarball: {e}"))
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

    #[test]
    fn test_should_ignore_returns_false_when_no_patterns_match() {
        let patterns = vec!["never-match/**".to_string()];
        assert!(!should_ignore("src/main.rs", &patterns));
    }

    #[test]
    fn test_write_tarball_persists_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("package.tar.zst");
        let payload = b"zstd-bytes";

        write_tarball(&path, payload).unwrap();

        let read_back = fs::read(&path).unwrap();
        assert_eq!(read_back, payload);
    }

    #[test]
    fn test_load_ignore_patterns_reads_custom_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        fs::write(
            dir.join(".apkgignore"),
            "# a comment\nsecrets/**\n\n   \n*.log\n",
        )
        .unwrap();

        let patterns = load_ignore_patterns(dir);

        // Comment and blank lines stripped; custom patterns kept.
        assert_eq!(
            patterns,
            vec!["secrets/**".to_string(), "*.log".to_string()]
        );
    }

    #[test]
    fn test_load_ignore_patterns_falls_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let patterns = load_ignore_patterns(tmp.path());
        assert!(patterns.iter().any(|p| p == ".git/**"));
        assert!(patterns.iter().any(|p| p == "target/**"));
    }

    #[test]
    fn test_create_tarball_respects_custom_ignore() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        fs::write(dir.join(".apkgignore"), "secret.txt\n").unwrap();
        fs::write(dir.join("apkg.json"), r#"{"name":"test"}"#).unwrap();
        fs::write(dir.join("secret.txt"), "should-not-ship").unwrap();

        let tarball = create_tarball(dir).unwrap();

        let extract_dir = tmp.path().join("extracted");
        extract_tarball(&tarball, &extract_dir).unwrap();
        assert!(extract_dir.join("apkg.json").exists());
        assert!(!extract_dir.join("secret.txt").exists());
    }

    #[test]
    fn test_create_tarball_preserves_nested_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        fs::create_dir_all(dir.join("a/b/c")).unwrap();
        fs::write(dir.join("a/b/c/leaf.txt"), "deep").unwrap();
        fs::write(dir.join("a/top.txt"), "shallow").unwrap();

        let tarball = create_tarball(dir).unwrap();

        let extract_dir = tmp.path().join("extracted");
        extract_tarball(&tarball, &extract_dir).unwrap();
        assert_eq!(
            fs::read_to_string(extract_dir.join("a/b/c/leaf.txt")).unwrap(),
            "deep"
        );
        assert_eq!(
            fs::read_to_string(extract_dir.join("a/top.txt")).unwrap(),
            "shallow"
        );
    }

    #[test]
    fn test_create_tarball_errors_on_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let result = create_tarball(&missing);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_tarball_excludes_default_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/config"), "git-state").unwrap();
        fs::write(dir.join("apkg.json"), r#"{"name":"test"}"#).unwrap();

        let tarball = create_tarball(dir).unwrap();

        let extract_dir = tmp.path().join("extracted");
        extract_tarball(&tarball, &extract_dir).unwrap();
        assert!(extract_dir.join("apkg.json").exists());
        assert!(!extract_dir.join(".git").exists());
    }

    #[test]
    fn test_extract_tarball_rejects_payload_exceeding_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Produce a file larger than the 2 MB cap. Highly compressible (zeros)
        // simulates a zstd-bomb: small compressed, large decompressed.
        let payload = vec![0u8; (MAX_DECOMPRESSED_BYTES as usize) + 4096];
        fs::write(dir.join("huge.bin"), &payload).unwrap();

        let tarball = create_tarball(dir).unwrap();
        assert!(
            (tarball.len() as u64) < MAX_DECOMPRESSED_BYTES,
            "compressed tarball should be far smaller than decompressed payload"
        );

        let extract_dir = tmp.path().join("extracted");
        let err = extract_tarball(&tarball, &extract_dir).unwrap_err();
        assert!(
            matches!(err, AppError::Tarball(_)),
            "expected Tarball error, got: {err}"
        );
        // Cap must keep the extracted file strictly below the cap (we can't
        // measure the decompression stream directly, but if extraction errored
        // the partially-unpacked file, if any, must be under the cap).
        let partial = extract_dir.join("huge.bin");
        if partial.exists() {
            let sz = fs::metadata(&partial).unwrap().len();
            assert!(
                sz <= MAX_DECOMPRESSED_BYTES,
                "partial file exceeded cap: {sz} bytes"
            );
        }
    }

    #[test]
    fn test_extract_tarball_accepts_payload_under_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Well under 2 MB — should extract cleanly.
        let payload = vec![0u8; 512 * 1024];
        fs::write(dir.join("medium.bin"), &payload).unwrap();

        let tarball = create_tarball(dir).unwrap();
        let extract_dir = tmp.path().join("extracted");
        extract_tarball(&tarball, &extract_dir).unwrap();
        assert_eq!(
            fs::metadata(extract_dir.join("medium.bin")).unwrap().len(),
            payload.len() as u64
        );
    }

    /// Wrap raw tar bytes in zstd so they can be passed to `extract_tarball`.
    fn zstd_wrap(tar_bytes: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = Vec::new();
        let mut enc = zstd::Encoder::new(&mut buf, 0).unwrap();
        enc.write_all(tar_bytes).unwrap();
        enc.finish().unwrap();
        buf
    }

    #[test]
    fn test_extract_rejects_parent_dir_traversal() {
        // `Header::set_path` refuses `..` entries, so we write raw header bytes
        // to simulate what a malicious tarball would contain.
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            let payload = b"pwned";
            header.set_size(payload.len() as u64);
            header.set_entry_type(tar::EntryType::Regular);
            {
                let name_bytes = &mut header.as_old_mut().name;
                let evil = b"../evil";
                name_bytes[..evil.len()].copy_from_slice(evil);
            }
            header.set_cksum();
            builder.append(&header, &payload[..]).unwrap();
            builder.finish().unwrap();
        }
        let data = zstd_wrap(&tar_buf);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&data, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("parent-directory reference"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_rejects_symlink_entry() {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_path("link").unwrap();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("/etc/passwd").unwrap();
            header.set_size(0);
            header.set_cksum();
            builder.append(&header, std::io::empty()).unwrap();
            builder.finish().unwrap();
        }
        let data = zstd_wrap(&tar_buf);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&data, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("symlinks and hardlinks"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_rejects_hardlink_entry() {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_path("hardlink").unwrap();
            header.set_entry_type(tar::EntryType::Link);
            header.set_link_name("/etc/passwd").unwrap();
            header.set_size(0);
            header.set_cksum();
            builder.append(&header, std::io::empty()).unwrap();
            builder.finish().unwrap();
        }
        let data = zstd_wrap(&tar_buf);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&data, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("symlinks and hardlinks"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_rejects_non_regular_entry_type() {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_path("fifo").unwrap();
            header.set_entry_type(tar::EntryType::Fifo);
            header.set_size(0);
            header.set_cksum();
            builder.append(&header, std::io::empty()).unwrap();
            builder.finish().unwrap();
        }
        let data = zstd_wrap(&tar_buf);

        let tmp = tempfile::tempdir().unwrap();
        let err = extract_tarball(&data, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("not supported"),
            "unexpected error: {err}"
        );
    }
}
