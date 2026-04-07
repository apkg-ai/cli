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

#[cfg(test)]
mod tests {
    use super::*;

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
}
