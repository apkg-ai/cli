use std::path::Path;

use crate::error::AppError;

/// Restrict file permissions so only the owning user can read or write.
///
/// - Unix: `chmod 0o600`.
/// - Windows: invoke `icacls` to strip inherited ACEs and grant full control
///   only to the current user (`$USERNAME`). Matches the Unix posture as
///   closely as Windows ACLs allow.
/// - Other: no-op.
///
/// Chosen over the `keyring` crate: zero new dependencies, keeps the file
/// layout intact (so `apkg key list` and friends still walk the keys dir),
/// and avoids the D-Bus / Secret Service requirement that breaks headless
/// Linux environments.
// Per-branch `return` keeps all three cfg arms symmetric and avoids
// unreachable-code warnings on unusual targets. The lint is narrow here.
#[allow(clippy::needless_return)]
pub fn restrict_to_owner(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        use std::process::Command;
        let username = std::env::var("USERNAME").map_err(|_| {
            AppError::Environment(
                "Cannot determine current Windows username (USERNAME not set)".into(),
            )
        })?;
        let output = Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{username}:(F)"))
            .output()?;
        if !output.status.success() {
            return Err(AppError::Other(format!(
                "icacls failed to restrict permissions on {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        return Ok(());
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restrict_to_owner_returns_ok() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        restrict_to_owner(tmp.path()).expect("should succeed on this platform");
    }

    #[cfg(unix)]
    #[test]
    fn test_restrict_to_owner_sets_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        restrict_to_owner(tmp.path()).unwrap();
        let mode = tmp.path().metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(windows)]
    #[test]
    fn test_restrict_to_owner_runs_icacls_on_windows() {
        // Smoke test — icacls ships with every supported Windows version.
        // A successful exit status implies inheritance was stripped and the
        // grant applied; parsing icacls output would add noise without value.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        restrict_to_owner(tmp.path()).unwrap();
    }
}
