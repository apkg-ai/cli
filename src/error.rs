use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum AppError {
    #[error("Network error: {0}")]
    #[diagnostic(code(qpm::network), help("Check your internet connection and try again."))]
    Network(String),

    #[error("API error: {message}")]
    #[diagnostic(code(qpm::api))]
    Api {
        code: String,
        message: String,
        status: u16,
    },

    #[error("Authentication required. Run `qpm login` first.")]
    #[diagnostic(code(qpm::auth), help("Run `qpm login` to authenticate."))]
    AuthRequired,

    #[error("Authentication failed: {0}")]
    #[diagnostic(code(qpm::auth_failed), help("Check your username and password."))]
    AuthFailed(String),

    #[error("Manifest error: {0}")]
    #[diagnostic(code(qpm::manifest))]
    Manifest(String),

    #[error("Manifest not found. Run `qpm init` to create one.")]
    #[diagnostic(code(qpm::manifest_not_found), help("Run `qpm init` in your package directory."))]
    ManifestNotFound,

    #[error("Validation error: {0}")]
    #[diagnostic(code(qpm::validation))]
    #[allow(dead_code)]
    Validation(String),

    #[error("File already exists: {0}")]
    #[diagnostic(code(qpm::file_exists), help("Use --force to overwrite."))]
    FileExists(String),

    #[error("Package not found: {0}")]
    #[diagnostic(code(qpm::not_found))]
    PackageNotFound(String),

    #[error("Integrity mismatch: expected {expected}, got {actual}")]
    #[diagnostic(
        code(qpm::integrity),
        help("The downloaded package may be corrupted. Try again.")
    )]
    IntegrityMismatch { expected: String, actual: String },

    #[error("Dependency conflict: {0}")]
    #[diagnostic(
        code(qpm::conflict),
        help("Check if a newer version of one of the conflicting packages is available.")
    )]
    DependencyConflict(String),

    #[error("Lockfile is out of date: {0}")]
    #[diagnostic(
        code(qpm::lockfile_stale),
        help("Run `qpm install` to update the lockfile, then commit it.")
    )]
    LockfileStale(String),

    #[error("Lockfile not found.")]
    #[diagnostic(
        code(qpm::lockfile_not_found),
        help("Run `qpm install` locally and commit the generated qpm-lock.json.")
    )]
    LockfileNotFound,

    #[error("Verification failed: {0}")]
    #[diagnostic(
        code(qpm::verify_failed),
        help("One or more packages failed strict verification.")
    )]
    VerifyFailed(String),

    #[error("IO error: {0}")]
    #[diagnostic(code(qpm::io))]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    #[diagnostic(code(qpm::other))]
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
