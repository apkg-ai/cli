use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum AppError {
    #[error("Network error: {0}")]
    #[diagnostic(
        code(apkg::network),
        help("Check your internet connection and try again.")
    )]
    Network(String),

    #[error("API error: {message}")]
    #[diagnostic(code(apkg::api))]
    Api {
        code: String,
        message: String,
        status: u16,
    },

    #[error("Authentication required. Run `apkg login` first.")]
    #[diagnostic(code(apkg::auth), help("Run `apkg login` to authenticate."))]
    AuthRequired,

    #[error("Authentication failed: {0}")]
    #[diagnostic(code(apkg::auth_failed), help("Check your username and password."))]
    AuthFailed(String),

    #[error("Manifest error: {0}")]
    #[diagnostic(code(apkg::manifest))]
    Manifest(String),

    #[error("Manifest not found. Run `apkg init` to create one.")]
    #[diagnostic(
        code(apkg::manifest_not_found),
        help("Run `apkg init` in your package directory.")
    )]
    ManifestNotFound,

    #[error("Validation error: {0}")]
    #[diagnostic(code(apkg::validation))]
    #[allow(dead_code)]
    Validation(String),

    #[error("File already exists: {0}")]
    #[diagnostic(code(apkg::file_exists), help("Use --force to overwrite."))]
    FileExists(String),

    #[error("Package not found: {0}")]
    #[diagnostic(code(apkg::not_found))]
    PackageNotFound(String),

    #[error("Integrity mismatch: expected {expected}, got {actual}")]
    #[diagnostic(
        code(apkg::integrity),
        help("The downloaded package may be corrupted. Try again.")
    )]
    IntegrityMismatch { expected: String, actual: String },

    #[error("Dependency conflict: {0}")]
    #[diagnostic(
        code(apkg::conflict),
        help("Check if a newer version of one of the conflicting packages is available.")
    )]
    DependencyConflict(String),

    #[error("Lockfile is out of date: {0}")]
    #[diagnostic(
        code(apkg::lockfile_stale),
        help("Run `apkg install` to update the lockfile, then commit it.")
    )]
    LockfileStale(String),

    #[error("Lockfile not found.")]
    #[diagnostic(
        code(apkg::lockfile_not_found),
        help("Run `apkg install` locally and commit the generated apkg-lock.json.")
    )]
    LockfileNotFound,

    #[error("Verification failed: {0}")]
    #[diagnostic(
        code(apkg::verify_failed),
        help("One or more packages failed strict verification.")
    )]
    VerifyFailed(String),

    #[error("IO error: {0}")]
    #[diagnostic(code(apkg::io))]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    #[diagnostic(code(apkg::other))]
    Other(String),
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Other(format!("JSON error: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_serde_json_error() {
        let err: Result<serde_json::Value, _> = serde_json::from_str("not json");
        let app_err: AppError = err.unwrap_err().into();
        assert!(app_err.to_string().contains("JSON error"));
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            AppError::AuthRequired.to_string(),
            "Authentication required. Run `apkg login` first."
        );
        assert!(AppError::Network("timeout".into()).to_string().contains("timeout"));
    }
}
