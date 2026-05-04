use std::process::Command;

/// Run `git remote get-url origin` and return the URL if available.
pub fn get_remote_url() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Normalize a git remote URL to a clean HTTPS URL.
///
/// Handles SSH (`git@host:user/repo.git`, `ssh://git@host/user/repo.git`)
/// and HTTPS formats, stripping any trailing `.git` suffix.
pub fn normalize_remote_url(raw: &str) -> String {
    let url = raw.trim().strip_suffix(".git").unwrap_or(raw.trim());

    if let Some(rest) = url.strip_prefix("git@") {
        // git@github.com:user/repo -> https://github.com/user/repo
        let normalized = rest.replacen(':', "/", 1);
        return format!("https://{normalized}");
    }

    if let Some(rest) = url.strip_prefix("ssh://git@") {
        // ssh://git@github.com/user/repo -> https://github.com/user/repo
        return format!("https://{rest}");
    }

    url.to_string()
}

/// Get the repository URL from git remote, normalized to HTTPS.
pub fn get_repository_url() -> Option<String> {
    get_remote_url().map(|url| normalize_remote_url(&url))
}

/// Get the user name from `git config user.name`.
pub fn get_user_name() -> Option<String> {
    let output = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Get the user email from `git config user.email`.
pub fn get_user_email() -> Option<String> {
    let output = Command::new("git")
        .args(["config", "user.email"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let email = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if email.is_empty() {
        None
    } else {
        Some(email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    #[test]
    fn test_normalize_https_with_git_suffix() {
        assert_eq!(
            normalize_remote_url("https://github.com/user/repo.git"),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_https_without_git_suffix() {
        assert_eq!(
            normalize_remote_url("https://github.com/user/repo"),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_ssh_git_at() {
        assert_eq!(
            normalize_remote_url("git@github.com:user/repo.git"),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_ssh_git_at_no_suffix() {
        assert_eq!(
            normalize_remote_url("git@github.com:user/repo"),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_ssh_protocol() {
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/user/repo.git"),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_gitlab() {
        assert_eq!(
            normalize_remote_url("git@gitlab.com:org/project.git"),
            "https://gitlab.com/org/project"
        );
    }

    #[test]
    fn test_normalize_preserves_plain_https() {
        assert_eq!(
            normalize_remote_url("https://bitbucket.org/team/repo"),
            "https://bitbucket.org/team/repo"
        );
    }

    #[test]
    fn test_get_remote_url_in_git_repo() {
        // This test runs inside the apkg/cli git repo, so origin should exist
        let url = get_remote_url();
        // In a git repo with a remote, this should return Some
        // If running in an environment without a remote, it returns None (still valid)
        if let Some(ref u) = url {
            assert!(!u.is_empty());
        }
    }

    #[test]
    fn test_get_repository_url_normalizes() {
        let url = get_repository_url();
        if let Some(ref u) = url {
            // Should be normalized to https
            assert!(u.starts_with("https://"));
            assert!(!u.ends_with(".git"));
        }
    }

    #[test]
    fn test_get_user_name() {
        let name = get_user_name();
        // In most dev/CI environments, git user.name is configured
        if let Some(ref n) = name {
            assert!(!n.is_empty());
        }
    }

    #[test]
    fn test_get_user_email() {
        let email = get_user_email();
        if let Some(ref e) = email {
            assert!(!e.is_empty());
        }
    }

    /// Save/restore a process-global env var for the duration of a test. The
    /// `ENV_LOCK` serializes these tests with everything else that touches
    /// `HOME`, `PATH`, or `CWD`.
    struct EnvVarGuard {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.original {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn test_get_remote_url_returns_none_when_git_missing() {
        let _lock = env_lock();
        let _path = EnvVarGuard::set("PATH", "");

        assert!(get_remote_url().is_none());
    }

    #[test]
    fn test_get_user_name_returns_none_when_git_missing() {
        let _lock = env_lock();
        let _path = EnvVarGuard::set("PATH", "");

        assert!(get_user_name().is_none());
    }

    #[test]
    fn test_get_user_email_returns_none_when_git_missing() {
        let _lock = env_lock();
        let _path = EnvVarGuard::set("PATH", "");

        assert!(get_user_email().is_none());
    }

    #[test]
    fn test_get_remote_url_returns_none_outside_git_repo() {
        // Poisoning can happen if another test (not using ENV_LOCK) dropped a
        // tempdir that was the CWD. We only need exclusive access for the
        // duration of our own CWD change, so treat poison as acquirable.
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        // Anchor the restore to a directory we know exists, not the (possibly
        // stale) CWD left behind by an earlier test.
        let anchor = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = get_remote_url();

        std::env::set_current_dir(&anchor).unwrap();
        // Outside a git repo, `git remote get-url origin` exits non-zero.
        assert!(result.is_none());
    }

    #[test]
    fn test_get_repository_url_returns_none_when_git_missing() {
        let _lock = env_lock();
        let _path = EnvVarGuard::set("PATH", "");

        assert!(get_repository_url().is_none());
    }
}
