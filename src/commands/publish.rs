use std::env;

use indicatif::{ProgressBar, ProgressStyle};

use regex_lite::Regex;

use crate::api::client::ApiClient;
use crate::config::manifest::validate_platforms;
use crate::config::{credentials, manifest};
use crate::error::AppError;
use crate::util::{display, integrity, tarball};

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
        return Err(AppError::Other(
            format!(
                "Package name '{}' must be scoped: @username/name or @org/name",
                m.name
            ),
        ));
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
        std::fs::write(dir.join("apkg.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
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
}
