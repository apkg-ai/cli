use console::Style;

use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;

pub struct InfoOptions<'a> {
    pub package: &'a str,
    pub json: bool,
    pub registry: Option<&'a str>,
}

#[allow(clippy::too_many_lines)]
pub async fn run(opts: InfoOptions<'_>) -> Result<(), AppError> {
    let client = ApiClient::new(opts.registry)?;
    let metadata = client.get_package(opts.package).await?;

    // Resolve the latest version's platform info for display
    let latest_platform = metadata
        .dist_tags
        .get("latest")
        .and_then(|v| metadata.versions.get(v))
        .and_then(|vm| vm.platform.clone());

    if opts.json {
        let json_val = serde_json::to_string_pretty(&serde_json::json!({
            "name": metadata.name,
            "description": metadata.description,
            "distTags": metadata.dist_tags,
            "versions": metadata.versions.keys().collect::<Vec<_>>(),
            "maintainers": metadata.maintainers.iter().map(|m| &m.username).collect::<Vec<_>>(),
            "platform": latest_platform,
            "createdAt": metadata.created_at,
            "updatedAt": metadata.updated_at,
        }))?;
        println!("{json_val}");
        return Ok(());
    }

    let name_style = Style::new().bold().cyan();
    let version_style = Style::new().green();
    let header_style = Style::new().bold();
    let dim_style = Style::new().dim();

    // Name + latest version
    let latest = metadata
        .dist_tags
        .get("latest")
        .map_or("?.?.?", std::string::String::as_str);
    println!(
        "\n{} {}",
        name_style.apply_to(&metadata.name),
        version_style.apply_to(latest)
    );

    // Description
    if let Some(desc) = &metadata.description {
        if !desc.is_empty() {
            println!("{desc}");
        }
    }
    println!();

    // Dist-tags
    if !metadata.dist_tags.is_empty() {
        println!("{}", header_style.apply_to("Dist-Tags:"));
        for (tag, version) in &metadata.dist_tags {
            println!("  {}: {}", tag, version_style.apply_to(version));
        }
        println!();
    }

    // Platform
    if let Some(ref platforms) = latest_platform {
        if !platforms.is_empty() {
            println!("{}", header_style.apply_to("Platform:"));
            println!("  {}", platforms.join(", "));
            println!();
        }
    }

    // Versions
    if !metadata.versions.is_empty() {
        println!("{}", header_style.apply_to("Versions:"));
        let mut versions: Vec<(&String, &crate::api::types::VersionMetadata)> =
            metadata.versions.iter().collect();
        versions.sort_by(|a, b| {
            let va = semver::Version::parse(&a.1.version);
            let vb = semver::Version::parse(&b.1.version);
            match (va, vb) {
                (Ok(a), Ok(b)) => b.cmp(&a),
                _ => b.0.cmp(a.0),
            }
        });
        for (_, v) in &versions {
            let yanked = v.yanked.unwrap_or(false);
            let type_str = v
                .package_type
                .as_deref()
                .map(|t| format!(" [{t}]"))
                .unwrap_or_default();
            let date = v
                .published_at
                .as_deref()
                .map(|d| format!("  {}", dim_style.apply_to(d)))
                .unwrap_or_default();
            if yanked {
                println!(
                    "  {} {}{}",
                    dim_style.apply_to(&v.version),
                    Style::new().red().apply_to("(yanked)"),
                    date
                );
            } else {
                println!(
                    "  {}{}{}",
                    version_style.apply_to(&v.version),
                    type_str,
                    date
                );
            }
        }
        println!();
    }

    // Maintainers
    if !metadata.maintainers.is_empty() {
        println!("{}", header_style.apply_to("Maintainers:"));
        for m in &metadata.maintainers {
            let role = m
                .role
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            println!("  {}{}", m.username, dim_style.apply_to(role));
        }
        println!();
    }

    // Timestamps
    if let Some(created) = &metadata.created_at {
        display::label_value("Created", created);
    }
    if let Some(updated) = &metadata.updated_at {
        display::label_value("Updated", updated);
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

    #[tokio::test]
    async fn test_info_json_output() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "description": "A test package",
                "distTags": { "latest": "1.0.0" },
                "versions": {
                    "1.0.0": {
                        "version": "1.0.0",
                        "type": "skill",
                        "platform": ["claude"]
                    }
                },
                "maintainers": [{ "username": "alice" }],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = run(InfoOptions {
            package: "mypkg",
            json: true,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_info_human_output() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "description": "A test package",
                "distTags": { "latest": "1.0.0" },
                "versions": {
                    "1.0.0": { "version": "1.0.0" }
                },
                "maintainers": [{ "username": "alice", "role": "owner" }],
                "createdAt": "2026-01-01T00:00:00Z",
                "updatedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = run(InfoOptions {
            package: "mypkg",
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_info_with_versions_and_yanked() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "distTags": { "latest": "2.0.0" },
                "versions": {
                    "1.0.0": { "version": "1.0.0", "yanked": true, "publishedAt": "2026-01-01" },
                    "2.0.0": { "version": "2.0.0", "type": "agent", "publishedAt": "2026-02-01" }
                },
                "maintainers": []
            })))
            .mount(&server)
            .await;

        let result = run(InfoOptions {
            package: "mypkg",
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_info_with_empty_description() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "description": "",
                "distTags": {},
                "versions": {},
                "maintainers": [],
                "platform": ["claude", "cursor"]
            })))
            .mount(&server)
            .await;

        let result = run(InfoOptions {
            package: "mypkg",
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_info_not_found() {
        let _guard = crate::test_utils::ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/nonexistent"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "code": "NOT_FOUND", "message": "Not found" }
            })))
            .mount(&server)
            .await;

        let result = run(InfoOptions {
            package: "nonexistent",
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_err());
    }
}
