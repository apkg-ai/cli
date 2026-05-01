use console::Style;

use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;
use crate::util::package::parse_package_spec;

pub enum DistTagAction<'a> {
    Add {
        package_at_version: &'a str,
        tag: &'a str,
    },
    Rm {
        package: &'a str,
        tag: &'a str,
    },
    Ls {
        package: &'a str,
    },
}

pub async fn run(action: DistTagAction<'_>, registry: Option<&str>) -> Result<(), AppError> {
    match action {
        DistTagAction::Add {
            package_at_version,
            tag,
        } => run_add(package_at_version, tag, registry).await,
        DistTagAction::Rm { package, tag } => run_rm(package, tag, registry).await,
        DistTagAction::Ls { package } => run_ls(package, registry).await,
    }
}

async fn run_add(
    package_at_version: &str,
    tag: &str,
    registry: Option<&str>,
) -> Result<(), AppError> {
    let (name, version) = parse_package_spec(package_at_version);
    let version = version.ok_or_else(|| {
        AppError::Other(
            "Version is required. Usage: apkg dist-tag add <pkg>@<version> <tag>".to_string(),
        )
    })?;

    let client = ApiClient::new(registry)?;
    let result = client.set_dist_tag(&name, tag, version).await?;

    display::success(&format!(
        "Set tag \"{}\" on {}@{}",
        result.tag, name, result.version
    ));

    Ok(())
}

async fn run_rm(package: &str, tag: &str, registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    client.remove_dist_tag(package, tag).await?;

    display::success(&format!("Removed tag \"{tag}\" from {package}"));

    Ok(())
}

async fn run_ls(package: &str, registry: Option<&str>) -> Result<(), AppError> {
    let client = ApiClient::new(registry)?;
    let metadata = client.get_package(package).await?;

    if metadata.dist_tags.is_empty() {
        println!("No dist-tags for {package}");
        return Ok(());
    }

    let tag_style = Style::new().bold();
    let version_style = Style::new().green();

    // Find longest tag name for alignment
    let max_len = metadata
        .dist_tags
        .keys()
        .map(String::len)
        .max()
        .unwrap_or(0);

    for (tag, version) in &metadata.dist_tags {
        println!(
            "{}  {}",
            tag_style.apply_to(format!("{tag:<max_len$}")),
            version_style.apply_to(version),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    #[tokio::test]
    async fn test_add_success() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex("/packages/.+/dist-tags/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag": "latest",
                "version": "1.0.0"
            })))
            .mount(&server)
            .await;

        let result = run(
            DistTagAction::Add {
                package_at_version: "mypkg@1.0.0",
                tag: "latest",
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_add_missing_version() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;

        let result = run(
            DistTagAction::Add {
                package_at_version: "mypkg",
                tag: "latest",
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rm_success() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path_regex("/packages/.+/dist-tags/.+"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let result = run(
            DistTagAction::Rm {
                package: "mypkg",
                tag: "beta",
            },
            Some(&server.uri()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ls_with_tags() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "distTags": { "latest": "1.0.0", "beta": "2.0.0-beta.1" },
                "versions": {},
                "maintainers": []
            })))
            .mount(&server)
            .await;

        let result = run(DistTagAction::Ls { package: "mypkg" }, Some(&server.uri())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ls_empty() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "distTags": {},
                "versions": {},
                "maintainers": []
            })))
            .mount(&server)
            .await;

        let result = run(DistTagAction::Ls { package: "mypkg" }, Some(&server.uri())).await;
        assert!(result.is_ok());
    }
}
