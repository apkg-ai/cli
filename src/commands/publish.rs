use std::env;

use indicatif::{ProgressBar, ProgressStyle};

use regex_lite::Regex;

use crate::api::client::ApiClient;
use crate::config::manifest::validate_platforms;
use crate::config::{credentials, manifest};
use crate::error::AppError;
use crate::util::{display, integrity, tarball};

/// Mirror of the server's `readme` size cap (`publishMetadataSchema.readme.max`).
const MAX_README_BYTES: usize = 20_480;

pub async fn run(registry: Option<&str>) -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    if !m.package_type.is_publishable() {
        return Err(AppError::Other(
            "Cannot publish a project. Only skill, agent, command, and rule packages can be published.".to_string(),
        ));
    }

    // Validate package name is scoped
    let re = Regex::new(r"^@[a-z0-9-]+/[a-z0-9]([a-z0-9._-]*[a-z0-9])?$").unwrap();
    if !re.is_match(&m.name) {
        return Err(AppError::Other(format!(
            "Package name '{}' must be scoped: @username/name or @org/name",
            m.name
        )));
    }

    // Warn if scope doesn't match the logged-in user
    if let Ok(Some(creds)) = credentials::load() {
        let scope = &m.name[1..m.name.find('/').unwrap()];
        if !scope.eq_ignore_ascii_case(&creds.username) {
            display::warn(&format!(
                "Scope '@{}' does not match your username '{}'. Publishing will succeed only if you are a member of the '@{}' organization.",
                scope, creds.username, scope
            ));
        }
    }

    display::info(&format!("Publishing {}@{} ...", m.name, m.version));

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message("Creating tarball...");
    pb.enable_steady_tick(std::time::Duration::from_millis(80));

    let data = tarball::create_tarball(&cwd)?;
    let hash = integrity::sha256_integrity(&data);
    let size = data.len();

    pb.set_message("Uploading to registry...");

    // Build metadata with only the fields the publish API accepts
    let mut metadata = serde_json::json!({
        "name": m.name,
        "version": m.version,
        "type": m.package_type,
        "integrity": hash,
    });
    let obj = metadata.as_object_mut().unwrap();
    if !m.description.is_empty() {
        obj.insert("description".into(), serde_json::json!(m.description));
    }
    if !m.license.is_empty() {
        obj.insert("license".into(), serde_json::json!(m.license));
    }
    if let Some(kw) = &m.keywords {
        obj.insert("keywords".into(), serde_json::json!(kw));
    }
    if let Some(repo) = &m.repository {
        obj.insert("repository".into(), serde_json::json!(repo));
    }
    if let Some(home) = &m.homepage {
        obj.insert("homepage".into(), serde_json::json!(home));
    }
    if let Some(readme_filename) = &m.readme {
        let readme_path = cwd.join(readme_filename);
        let content = std::fs::read_to_string(&readme_path).map_err(|e| {
            AppError::Other(format!(
                "Failed to read readme file '{readme_filename}': {e}"
            ))
        })?;
        if content.len() > MAX_README_BYTES {
            return Err(AppError::Other(format!(
                "Readme file '{readme_filename}' is {} bytes; registry limit is {MAX_README_BYTES} bytes.",
                content.len()
            )));
        }
        obj.insert("readme".into(), serde_json::json!(content));
    }
    if let Some(deps) = &m.dependencies {
        obj.insert("dependencies".into(), serde_json::json!(deps));
    }
    for warning in validate_platforms(&m.platform) {
        display::warn(&warning);
    }
    obj.insert("platform".into(), serde_json::json!(m.platform));

    let metadata_json = serde_json::to_string(&metadata)?;
    let client = ApiClient::new(registry)?;
    let resp = client.publish(&m.name, &metadata_json, data).await?;

    pb.finish_and_clear();

    display::success(&format!("Published {}@{}", resp.name, resp.version));
    display::label_value("Size", &display::format_size(size));
    display::label_value("Integrity", &hash);
    if let Some(server_integrity) = &resp.integrity {
        display::label_value("Server Integrity", server_integrity);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    fn write_manifest(dir: &std::path::Path, name: &str, pkg_type: &str) {
        let manifest = serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "type": pkg_type,
            "description": "Test package",
            "license": "MIT",
            "platform": ["claude"]
        });
        std::fs::write(
            dir.join("apkg.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_publish_unpublishable_type() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest(tmp.path(), "@user/proj", "project");
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let result = run(Some(&server.uri())).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Cannot publish a project"));
    }

    #[tokio::test]
    async fn test_publish_invalid_name() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest(tmp.path(), "unscoped-name", "skill");
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let result = run(Some(&server.uri())).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must be scoped"));
    }

    #[tokio::test]
    async fn test_publish_success() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest(tmp.path(), "@user/my-skill", "skill");
        // Create a source file so the tarball is non-empty
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/packages/%40user%2Fmy-skill"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@user/my-skill",
                "version": "1.0.0",
                "integrity": "sha256-xyz"
            })))
            .mount(&server)
            .await;

        let result = run(Some(&server.uri())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_publish_scope_mismatch_warning() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        // Create credentials with a different username than scope
        let creds_dir = tmp.path().join(".apkg");
        std::fs::create_dir_all(&creds_dir).unwrap();
        std::fs::write(
            creds_dir.join("credentials.json"),
            r#"{"registry":"https://api.apkg.ai","accessToken":"tok","refreshToken":"rt","username":"alice"}"#,
        ).unwrap();
        write_manifest(tmp.path(), "@bob/my-skill", "skill");
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/packages/%40bob%2Fmy-skill"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@bob/my-skill",
                "version": "1.0.0"
            })))
            .mount(&server)
            .await;

        // Should succeed but print warning (we just verify no error)
        let result = run(Some(&server.uri())).await;
        assert!(result.is_ok());
    }

    fn write_manifest_with(dir: &std::path::Path, extra: serde_json::Value) {
        let mut manifest = serde_json::json!({
            "name": "@user/my-skill",
            "version": "1.0.0",
            "type": "skill",
            "description": "Test package",
            "license": "MIT",
            "platform": ["claude"],
        });
        let base = manifest.as_object_mut().unwrap();
        for (k, v) in extra.as_object().unwrap() {
            base.insert(k.clone(), v.clone());
        }
        std::fs::write(
            dir.join("apkg.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// Extract the multipart `metadata` string field from a recorded request body.
    /// The body also contains the binary tarball, so we work on bytes, not utf8.
    fn extract_metadata_field(body: &[u8]) -> serde_json::Value {
        let marker = b"name=\"metadata\"";
        let start = find_subslice(body, marker).expect("metadata part present") + marker.len();
        let after = &body[start..];
        let blank_line = find_subslice(after, b"\r\n\r\n").expect("end of metadata headers");
        let content_start = blank_line + 4;
        let remaining = &after[content_start..];
        let boundary = find_subslice(remaining, b"\r\n--").expect("boundary after metadata");
        let slice = &remaining[..boundary];
        let text = std::str::from_utf8(slice).expect("metadata part is utf8");
        serde_json::from_str(text).expect("metadata is valid json")
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    #[tokio::test]
    async fn test_publish_sends_repository_and_homepage() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest_with(
            tmp.path(),
            serde_json::json!({
                "repository": "https://github.com/user/my-skill",
                "homepage": "https://example.test/my-skill",
            }),
        );
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/packages/%40user%2Fmy-skill"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@user/my-skill",
                "version": "1.0.0",
            })))
            .mount(&server)
            .await;

        run(Some(&server.uri())).await.unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        let metadata = extract_metadata_field(&received.body);
        assert_eq!(metadata["repository"], "https://github.com/user/my-skill");
        assert_eq!(metadata["homepage"], "https://example.test/my-skill");
    }

    #[tokio::test]
    async fn test_publish_inlines_readme_content() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest_with(tmp.path(), serde_json::json!({ "readme": "README.md" }));
        std::fs::write(tmp.path().join("README.md"), "# Hello\nSome docs.\n").unwrap();
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/packages/%40user%2Fmy-skill"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@user/my-skill",
                "version": "1.0.0",
            })))
            .mount(&server)
            .await;

        run(Some(&server.uri())).await.unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        let metadata = extract_metadata_field(&received.body);
        assert_eq!(metadata["readme"], "# Hello\nSome docs.\n");
    }

    #[tokio::test]
    async fn test_publish_errors_when_readme_exceeds_cap() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest_with(tmp.path(), serde_json::json!({ "readme": "README.md" }));
        // One byte over the cap.
        std::fs::write(
            tmp.path().join("README.md"),
            "a".repeat(MAX_README_BYTES + 1),
        )
        .unwrap();
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let err = run(Some(&server.uri())).await.unwrap_err();
        assert!(err.to_string().contains("registry limit"));
    }

    #[tokio::test]
    async fn test_publish_errors_when_readme_file_missing() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        write_manifest_with(tmp.path(), serde_json::json!({ "readme": "MISSING.md" }));
        std::fs::write(tmp.path().join("index.ts"), "export default {};").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let server = MockServer::start().await;
        let err = run(Some(&server.uri())).await.unwrap_err();
        assert!(err.to_string().contains("Failed to read readme file"));
    }
}
