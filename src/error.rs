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

    #[error("Network error (connection): {0}")]
    #[diagnostic(
        code(apkg::network_connect),
        help("The server may be unreachable. Check the registry URL and your network.")
    )]
    NetworkConnect(String),

    #[error("Network error (timeout): {0}")]
    #[diagnostic(
        code(apkg::network_timeout),
        help("The server took too long to respond. A retry may succeed.")
    )]
    NetworkTimeout(String),

    #[error("Network error (decode): {0}")]
    #[diagnostic(
        code(apkg::network_decode),
        help("The server returned a body we couldn't read. This is usually a server-side issue.")
    )]
    NetworkDecode(String),

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

    #[error("JSON error: {0}")]
    #[diagnostic(
        code(apkg::json),
        help("The response did not match the expected JSON schema. This is usually a server-side issue.")
    )]
    Json(String),

    #[error("Invalid input: {0}")]
    #[diagnostic(code(apkg::invalid_input))]
    InvalidInput(String),

    #[error("Environment error: {0}")]
    #[diagnostic(
        code(apkg::environment),
        help("apkg could not determine a required path or runtime context. Check your user home directory and environment variables.")
    )]
    Environment(String),

    #[error("Failed to parse {what}: {cause}")]
    #[diagnostic(
        code(apkg::parse),
        help("The content was malformed. If this is a local file, check it by hand; if it came from the registry, the server returned something unexpected.")
    )]
    Parse { what: String, cause: String },

    #[error("Tarball error: {0}")]
    #[diagnostic(
        code(apkg::tarball),
        help("Packing or unpacking the tarball failed. This is usually a local filesystem or disk issue.")
    )]
    Tarball(String),

    #[error("Interactive prompt error: {0}")]
    #[diagnostic(
        code(apkg::interactive),
        help("Could not read from the terminal. If you're in a non-interactive shell or CI, pass values via CLI flags instead.")
    )]
    Interactive(String),

    #[error("{0}")]
    #[diagnostic(code(apkg::other))]
    Other(String),
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        let msg = err.to_string();
        if err.is_connect() {
            AppError::NetworkConnect(msg)
        } else if err.is_timeout() {
            AppError::NetworkTimeout(msg)
        } else if err.is_decode() {
            AppError::NetworkDecode(msg)
        } else {
            AppError::Network(msg)
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Json(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_serde_json_error() {
        let err: Result<serde_json::Value, _> = serde_json::from_str("not json");
        let app_err: AppError = err.unwrap_err().into();
        assert!(matches!(app_err, AppError::Json(_)));
        assert!(app_err.to_string().starts_with("JSON error"));
    }

    #[test]
    fn test_new_variants_display() {
        assert!(AppError::InvalidInput("x".into())
            .to_string()
            .starts_with("Invalid input"));
        assert!(AppError::Environment("x".into())
            .to_string()
            .starts_with("Environment error"));
        assert!(AppError::Parse {
            what: "x".into(),
            cause: "y".into()
        }
        .to_string()
        .starts_with("Failed to parse x"));
        assert!(AppError::Tarball("x".into())
            .to_string()
            .starts_with("Tarball error"));
        assert!(AppError::Interactive("x".into())
            .to_string()
            .starts_with("Interactive prompt error"));
    }

    #[test]
    fn test_error_display() {
        assert_eq!(
            AppError::AuthRequired.to_string(),
            "Authentication required. Run `apkg login` first."
        );
        assert!(AppError::Network("timeout".into())
            .to_string()
            .contains("timeout"));
    }

    #[tokio::test]
    async fn test_from_reqwest_error_transport() {
        // Provoke a real reqwest::Error by targeting an unroutable address.
        let err = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1))
            .build()
            .unwrap()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        let app_err: AppError = err.into();
        // Any of connect / timeout / generic network is acceptable — depends on OS timing.
        assert!(matches!(
            app_err,
            AppError::NetworkConnect(_) | AppError::NetworkTimeout(_) | AppError::Network(_)
        ));
        assert!(app_err.to_string().starts_with("Network error"));
    }
}
