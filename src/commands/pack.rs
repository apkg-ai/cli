use std::env;

use crate::config::manifest;
use crate::error::AppError;
use crate::util::{display, integrity, tarball};

pub fn run() -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    display::info(&format!("Packing {}@{} ...", m.name, m.version));

    let data = tarball::create_tarball(&cwd)?;
    let hash = integrity::sha256_integrity(&data);
    let size = data.len();

    let filename = format!(
        "{}-{}.tar.zst",
        m.name.replace('/', "-").replace('@', ""),
        m.version
    );
    let out_path = cwd.join(&filename);
    tarball::write_tarball(&out_path, &data)?;

    display::success(&format!("Packed {filename}"));
    display::label_value("Size", &display::format_size(size));
    display::label_value("Integrity", &hash);
    println!("{}", out_path.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{Manifest, PackageType};
    use crate::test_utils::env_lock;
    use std::fs;
    use std::path::Path;

    /// Anchor CWD to `CARGO_MANIFEST_DIR` on drop so a failing test doesn't
    /// leave a stale tempdir as the process CWD.
    struct CwdGuard;
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
        }
    }

    fn write_manifest(dir: &Path, name: &str, version: &str) {
        let m = Manifest {
            name: name.to_string(),
            version: version.to_string(),
            package_type: PackageType::Skill,
            description: String::new(),
            license: "MIT".to_string(),
            readme: None,
            keywords: None,
            authors: None,
            repository: None,
            homepage: None,
            platform: vec!["claude-code".to_string()],
            dependencies: None,
            dev_dependencies: None,
            peer_dependencies: None,
            scripts: None,
            hook_permissions: None,
        };
        crate::config::manifest::save(dir, &m).unwrap();
    }

    #[test]
    fn test_run_writes_tarball_for_scoped_package() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "@test/my-pkg", "0.1.0");
        fs::write(tmp.path().join("README.md"), "hello").unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(tmp.path()).unwrap();

        run().unwrap();

        // Filename convention: `<scope-stripped-name>-<version>.tar.zst`.
        let out = tmp.path().join("test-my-pkg-0.1.0.tar.zst");
        assert!(out.exists(), "tarball should be written");
        let data = fs::read(&out).unwrap();
        // zstd magic bytes.
        assert_eq!(&data[..4], [0x28, 0xB5, 0x2F, 0xFD]);
    }

    #[test]
    fn test_run_errors_without_manifest() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();

        let _cwd = CwdGuard;
        std::env::set_current_dir(tmp.path()).unwrap();

        let err = run().unwrap_err();
        // manifest::load surfaces a ManifestNotFound variant.
        assert!(matches!(err, AppError::ManifestNotFound));
    }
}
