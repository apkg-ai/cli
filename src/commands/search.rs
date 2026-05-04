use console::Style;

use crate::api::client::ApiClient;
use crate::error::AppError;
use crate::util::display;

pub struct SearchOptions<'a> {
    pub query: &'a str,
    pub limit: u32,
    pub json: bool,
    pub registry: Option<&'a str>,
}

pub async fn run(opts: SearchOptions<'_>) -> Result<(), AppError> {
    let client = ApiClient::new(opts.registry)?;

    let resp = match client.search(opts.query, opts.limit, 0).await {
        Ok(resp) => resp,
        Err(AppError::Network(_)) => {
            display::warn("Search service is unavailable.");
            display::info("Try searching at https://apkg.ai/search instead.");
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "results": resp.results.iter().map(|r| serde_json::json!({
                    "name": r.name,
                    "version": r.version,
                    "description": r.description,
                    "type": r.package_type,
                    "platform": r.platform,
                })).collect::<Vec<_>>(),
                "total": resp.total,
            }))?
        );
        return Ok(());
    }

    if resp.results.is_empty() {
        display::info(&format!("No packages found for \"{}\".", opts.query));
        return Ok(());
    }

    println!(
        "Found {} package{}:\n",
        resp.total,
        if resp.total == 1 { "" } else { "s" }
    );

    let name_style = Style::new().bold().cyan();
    let version_style = Style::new().green();
    let dim_style = Style::new().dim();

    for result in &resp.results {
        let version_str = result.version.as_deref().unwrap_or("?.?.?");
        let desc = result.description.as_deref().unwrap_or("");
        let type_str = result
            .package_type
            .as_deref()
            .map(|t| format!(" [{t}]"))
            .unwrap_or_default();
        let platform_str = result
            .platform
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| format!(" ({})", p.join(", ")))
            .unwrap_or_default();

        println!(
            "  {} {} {}{}{}",
            name_style.apply_to(&result.name),
            version_style.apply_to(version_str),
            dim_style.apply_to(type_str),
            dim_style.apply_to(&platform_str),
            if desc.is_empty() {
                String::new()
            } else {
                format!("\n    {}", dim_style.apply_to(desc))
            }
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)] // ENV_LOCK guard held across mock-server awaits; see src/api/client.rs tests block for rationale.

    use super::*;
    use crate::test_utils::env_lock;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_env(tmp: &std::path::Path) {
        std::env::set_var("HOME", tmp);
        std::env::set_var("APKG_TOKEN", "test-token");
    }

    #[tokio::test]
    async fn test_search_json_output() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/search.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "name": "@test/skill",
                        "version": "1.0.0",
                        "description": "A test skill",
                        "type": "skill",
                        "platform": ["claude"]
                    }
                ],
                "total": 1
            })))
            .mount(&server)
            .await;

        let result = run(SearchOptions {
            query: "test",
            limit: 10,
            json: true,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search_human_output() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/search.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "name": "@test/skill",
                        "version": "1.0.0",
                        "description": "A cool skill",
                        "type": "skill",
                        "platform": ["claude", "cursor"]
                    },
                    {
                        "name": "@test/agent",
                        "description": "",
                        "type": "agent"
                    }
                ],
                "total": 2
            })))
            .mount(&server)
            .await;

        let result = run(SearchOptions {
            query: "test",
            limit: 10,
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search_empty_results() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/search.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [],
                "total": 0
            })))
            .mount(&server)
            .await;

        let result = run(SearchOptions {
            query: "nonexistent",
            limit: 10,
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search_single_result() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        setup_env(tmp.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/search.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "name": "@test/one", "version": "0.1.0" }],
                "total": 1
            })))
            .mount(&server)
            .await;

        let result = run(SearchOptions {
            query: "one",
            limit: 10,
            json: false,
            registry: Some(&server.uri()),
        })
        .await;
        assert!(result.is_ok());
    }
}
