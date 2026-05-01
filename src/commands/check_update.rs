use std::time::Duration;

use serde::Deserialize;

use crate::error::AppError;
use crate::util::display;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/apkg-ai/cli/releases/latest";
const RELEASES_PAGE_URL: &str = "https://github.com/apkg-ai/cli/releases/latest";

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

pub async fn run() -> Result<(), AppError> {
    check(LATEST_RELEASE_URL).await
}

async fn check(url: &str) -> Result<(), AppError> {
    let current_raw = env!("CARGO_PKG_VERSION");
    let current = semver::Version::parse(current_raw)
        .map_err(|e| AppError::Other(format!("invalid compiled-in version: {e}")))?;

    let client = reqwest::Client::builder()
        .user_agent(format!("apkg-cli/{current_raw}"))
        .timeout(Duration::from_secs(10))
        .build()?;

    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 403
            && resp
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                == Some("0")
        {
            return Err(AppError::Other(
                "GitHub API rate limit exceeded. Try again later.".to_string(),
            ));
        }
        return Err(AppError::Other(format!(
            "GitHub API returned HTTP {status}"
        )));
    }

    let release: LatestRelease = resp.json().await?;
    let tag = release.tag_name.trim_start_matches('v');
    let latest = semver::Version::parse(tag)
        .map_err(|e| AppError::Other(format!("unexpected release tag '{tag}': {e}")))?;

    match current.cmp(&latest) {
        std::cmp::Ordering::Equal => {
            display::success(&format!("apkg is up to date (v{current})"));
        }
        std::cmp::Ordering::Less => {
            display::warn(&format!(
                "apkg v{latest} is available (you have v{current}). Download it from {RELEASES_PAGE_URL}"
            ));
        }
        std::cmp::Ordering::Greater => {
            display::info(&format!(
                "Running a pre-release (v{current}, latest published is v{latest})"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body(tag: &str) -> serde_json::Value {
        serde_json::json!({ "tag_name": tag })
    }

    #[tokio::test]
    async fn up_to_date() {
        let server = MockServer::start().await;
        let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body(&tag)))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        check(&url).await.unwrap();
    }

    #[tokio::test]
    async fn newer_available() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body("v99.0.0")))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        check(&url).await.unwrap();
    }

    #[tokio::test]
    async fn ahead_of_latest() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body("v0.0.1")))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        check(&url).await.unwrap();
    }

    #[tokio::test]
    async fn http_error_surfaces() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        let err = check(&url).await.unwrap_err();
        assert!(err.to_string().contains("500"), "got: {err}");
    }

    #[tokio::test]
    async fn rate_limit_gives_friendly_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        let err = check(&url).await.unwrap_err();
        assert!(err.to_string().contains("rate limit"), "got: {err}");
    }

    #[tokio::test]
    async fn malformed_tag_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body("not-a-version")))
            .mount(&server)
            .await;

        let url = format!("{}/latest", server.uri());
        let err = check(&url).await.unwrap_err();
        assert!(
            err.to_string().contains("unexpected release tag"),
            "got: {err}"
        );
    }
}
