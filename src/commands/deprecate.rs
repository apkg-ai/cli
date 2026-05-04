use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::{display, package::parse_package_spec};

pub struct DeprecateOptions<'a> {
    pub target: &'a str,
    pub message: Option<&'a str>,
    pub registry: Option<&'a str>,
}

pub async fn run(opts: DeprecateOptions<'_>) -> Result<(), AppError> {
    let (name, version) = parse_package_spec(opts.target);
    let client = ApiClient::new(opts.registry)?;

    if let Some(ver) = version {
        client.deprecate_version(&name, ver, opts.message).await?;
        if opts.message.is_some() {
            display::success(&format!("Deprecated {name}@{ver}"));
        } else {
            display::success(&format!("Removed deprecation from {name}@{ver}"));
        }
    } else {
        client.deprecate_package(&name, opts.message).await?;
        if opts.message.is_some() {
            display::success(&format!("Deprecated {name}"));
        } else {
            display::success(&format!("Removed deprecation from {name}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;
    use crate::test_utils::env_lock;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    #[tokio::test]
    async fn test_deprecate_package_with_message() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "versions": {},
                "maintainers": [],
                "distTags": {},
                "deprecated": "Use v2 instead"
            })))
            .mount(&server)
            .await;

        let result = run(DeprecateOptions {
            target: "mypkg",
            message: Some("Use v2 instead"),
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_deprecate_version_with_message() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg/1.0.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": "1.0.0",
                "deprecated": "Use v2"
            })))
            .mount(&server)
            .await;

        let result = run(DeprecateOptions {
            target: "mypkg@1.0.0",
            message: Some("Use v2"),
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_undeprecate_package() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "versions": {},
                "maintainers": [],
                "distTags": {}
            })))
            .mount(&server)
            .await;

        let result = run(DeprecateOptions {
            target: "mypkg",
            message: None,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_undeprecate_version() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg/1.0.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": "1.0.0"
            })))
            .mount(&server)
            .await;

        let result = run(DeprecateOptions {
            target: "mypkg@1.0.0",
            message: None,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }
}
