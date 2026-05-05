use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::multipart;

use crate::api::errors::parse_error_response;
use crate::api::types::{
    CreateTokenResponse, DistTagResult, ListKeysResponse, ListTokensResponse, LoginResponse,
    MfaChallengeResponse, PackageMetadata, ProvenanceAttestation, PublishResponse,
    RegisterKeyResponse, RegistrySigningKeyCollection, RevokeKeyResponse, RotateKeyResponse,
    SearchResponse, VersionMetadata, WhoamiResponse,
};
use crate::config::credentials;
use crate::config::settings::Settings;
use crate::error::AppError;

pub struct ApiClient {
    client: reqwest::Client,
    settings: Settings,
    registry_override: Option<String>,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(registry_override: Option<&str>) -> Result<Self, AppError> {
        let settings = Settings::load()?;
        let token = credentials::load()?
            .map(|c| c.access_token)
            .or_else(|| std::env::var("APKG_TOKEN").ok());

        let client = reqwest::Client::builder()
            .user_agent(format!("apkg-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            client,
            settings,
            registry_override: registry_override.map(String::from),
            token,
        })
    }

    fn url(&self, service: &str, path: &str) -> String {
        // Per-service config is the most specific — always wins
        let base = if let Some(svc_url) = self.settings.service_url(service) {
            svc_url.trim_end_matches('/').to_string()
        } else if let Some(ovr) = &self.registry_override {
            ovr.trim_end_matches('/').to_string()
        } else if let Ok(env_reg) = std::env::var("APKG_REGISTRY") {
            env_reg.trim_end_matches('/').to_string()
        } else {
            self.settings.base_url(service)
        };
        format!("{base}{path}")
    }

    fn auth_headers(&self) -> Result<HeaderMap, AppError> {
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.token {
            let val = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| AppError::Other(format!("Invalid auth header: {e}")))?;
            headers.insert(AUTHORIZATION, val);
        }
        Ok(headers)
    }

    fn require_auth_headers(&self) -> Result<HeaderMap, AppError> {
        if self.token.is_none() {
            return Err(AppError::AuthRequired);
        }
        self.auth_headers()
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginResponse, AppError> {
        let url = self.url("auth", "/auth/login");
        let body = serde_json::json!({
            "username": username,
            "password": password,
        });
        let resp = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn mfa_challenge(
        &self,
        mfa_token: &str,
        code: &str,
    ) -> Result<MfaChallengeResponse, AppError> {
        let url = self.url("mfa", "/auth/mfa/challenge");
        let body = serde_json::json!({
            "mfaToken": mfa_token,
            "code": code,
        });
        let resp = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn whoami(&self) -> Result<WhoamiResponse, AppError> {
        let url = self.url("auth", "/auth/whoami");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_package(&self, name: &str) -> Result<PackageMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn download_tarball(
        &self,
        name: &str,
        version: &str,
    ) -> Result<(Vec<u8>, Option<String>), AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{version}/tarball"),
        );
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            let integrity = resp
                .headers()
                .get("x-integrity")
                .and_then(|v| v.to_str().ok())
                .map(std::string::ToString::to_string);
            let bytes = resp.bytes().await?;
            Ok((bytes.to_vec(), integrity))
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(format!("{name}@{version}")))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn publish(
        &self,
        name: &str,
        metadata_json: &str,
        tarball_bytes: Vec<u8>,
    ) -> Result<PublishResponse, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.require_auth_headers()?;

        let form = multipart::Form::new()
            .text("metadata", metadata_json.to_string())
            .part(
                "tarball",
                multipart::Part::bytes(tarball_bytes)
                    .file_name("package.tar.zst")
                    .mime_str("application/zstd")
                    .map_err(|e| AppError::Other(format!("Failed to build multipart: {e}")))?,
            );

        let resp = self
            .client
            .put(&url)
            .headers(headers)
            .multipart(form)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn register_key(
        &self,
        public_key: &str,
        name: &str,
    ) -> Result<RegisterKeyResponse, AppError> {
        let url = self.url("key", "/auth/keys");
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({
            "publicKey": public_key,
            "name": name,
        });
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn list_keys(&self) -> Result<ListKeysResponse, AppError> {
        let url = self.url("key", "/auth/keys");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn revoke_key(
        &self,
        key_id: &str,
        reason: &str,
        message: Option<&str>,
    ) -> Result<RevokeKeyResponse, AppError> {
        let encoded_key_id = urlencoding::encode(key_id);
        let url = self.url("key", &format!("/auth/keys/{encoded_key_id}/revoke"));
        let headers = self.require_auth_headers()?;
        let mut body = serde_json::json!({
            "reason": reason,
        });
        if let Some(msg) = message {
            body["message"] = serde_json::Value::String(msg.to_string());
        }
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn rotate_key(
        &self,
        old_key_id: &str,
        new_public_key: &str,
        attestation: &str,
    ) -> Result<RotateKeyResponse, AppError> {
        let url = self.url("key", "/auth/keys/rotate");
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({
            "oldKeyId": old_key_id,
            "newPublicKey": new_public_key,
            "attestation": attestation,
        });
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn create_token(
        &self,
        name: &str,
        scopes: &[String],
        expires_in: &str,
        package_scope: Option<&str>,
    ) -> Result<CreateTokenResponse, AppError> {
        let url = self.url("token", "/auth/tokens");
        let headers = self.require_auth_headers()?;
        let mut body = serde_json::json!({
            "name": name,
            "scopes": scopes,
            "expiresIn": expires_in,
        });
        if let Some(scope) = package_scope {
            body["packageScope"] = serde_json::Value::String(scope.to_string());
        }
        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn list_tokens(&self) -> Result<ListTokensResponse, AppError> {
        let url = self.url("token", "/auth/tokens");
        let headers = self.require_auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn revoke_token(&self, id: &str) -> Result<(), AppError> {
        let encoded_id = urlencoding::encode(id);
        let url = self.url("token", &format!("/auth/tokens/{encoded_id}"));
        let headers = self.require_auth_headers()?;
        let resp = self.client.delete(&url).headers(headers).send().await?;

        if resp.status().as_u16() == 204 || resp.status().is_success() {
            Ok(())
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse, AppError> {
        let encoded_q = urlencoding::encode(query);
        let url = self.url(
            "search",
            &format!("/search?q={encoded_q}&limit={limit}&offset={offset}"),
        );
        let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn deprecate_package(
        &self,
        name: &str,
        message: Option<&str>,
    ) -> Result<PackageMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let url = self.url("package", &format!("/packages/{encoded_name}"));
        let headers = self.require_auth_headers()?;
        let deprecated = match message {
            Some(msg) => serde_json::Value::String(msg.to_string()),
            None => serde_json::Value::Null,
        };
        let body = serde_json::json!({ "deprecated": deprecated });
        let resp = self
            .client
            .patch(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn deprecate_version(
        &self,
        name: &str,
        version: &str,
        message: Option<&str>,
    ) -> Result<VersionMetadata, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_version = urlencoding::encode(version);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{encoded_version}"),
        );
        let headers = self.require_auth_headers()?;
        let deprecated = match message {
            Some(msg) => serde_json::Value::String(msg.to_string()),
            None => serde_json::Value::Null,
        };
        let body = serde_json::json!({ "deprecated": deprecated });
        let resp = self
            .client
            .patch(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(format!("{name}@{version}")))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn set_dist_tag(
        &self,
        name: &str,
        tag: &str,
        version: &str,
    ) -> Result<DistTagResult, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_tag = urlencoding::encode(tag);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/dist-tags/{encoded_tag}"),
        );
        let headers = self.require_auth_headers()?;
        let body = serde_json::json!({ "version": version });
        let resp = self
            .client
            .put(&url)
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_registry_signing_keys(
        &self,
    ) -> Result<RegistrySigningKeyCollection, AppError> {
        let url = self.url("registry", "/registry/signing-keys");
        let resp = self.client.get(&url).send().await?;

        if resp.status().is_success() {
            parse_json_response(resp).await
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn get_provenance(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<ProvenanceAttestation>, AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_version = urlencoding::encode(version);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/{encoded_version}/provenance"),
        );
        let headers = self.auth_headers()?;
        let resp = self.client.get(&url).headers(headers).send().await?;

        if resp.status().is_success() {
            let body: ProvenanceAttestation = parse_json_response(resp).await?;
            Ok(Some(body))
        } else if resp.status().as_u16() == 404 {
            Ok(None)
        } else {
            Err(parse_error_response(resp).await)
        }
    }

    pub async fn remove_dist_tag(&self, name: &str, tag: &str) -> Result<(), AppError> {
        let encoded_name = encode_package_name(name);
        let encoded_tag = urlencoding::encode(tag);
        let url = self.url(
            "package",
            &format!("/packages/{encoded_name}/dist-tags/{encoded_tag}"),
        );
        let headers = self.require_auth_headers()?;
        let resp = self.client.delete(&url).headers(headers).send().await?;

        if resp.status().as_u16() == 204 || resp.status().is_success() {
            Ok(())
        } else if resp.status().as_u16() == 404 {
            Err(AppError::PackageNotFound(name.to_string()))
        } else {
            Err(parse_error_response(resp).await)
        }
    }
}

const MAX_ERROR_BODY_BYTES: usize = 512;

fn truncate_body(body: &str) -> String {
    if body.len() <= MAX_ERROR_BODY_BYTES {
        return body.to_string();
    }
    let mut end = MAX_ERROR_BODY_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let remaining = body.len() - end;
    format!("{}… (truncated, {remaining} more bytes)", &body[..end])
}

async fn parse_json_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, AppError> {
    let body = resp.text().await?;
    serde_json::from_str(&body).map_err(|e| AppError::Parse {
        what: "response body".into(),
        cause: format!("{e}\nBody: {}", truncate_body(&body)),
    })
}

fn encode_package_name(name: &str) -> String {
    urlencoding::encode(name).to_string()
}

#[cfg(test)]
mod tests {
    // Tests acquire std::sync::Mutex ENV_LOCK and hold it across `.await`s that
    // don't touch the locked state (mock HTTP servers). Each `#[tokio::test]`
    // runs on a single-threaded runtime, so the classic deadlock mode (one
    // worker holds the guard, another tries to acquire) cannot occur here.
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::test_utils::env_lock;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_truncate_body_short_unchanged() {
        assert_eq!(truncate_body("short body"), "short body");
    }

    #[test]
    fn test_truncate_body_long_is_cut() {
        let s = "x".repeat(1000);
        let out = truncate_body(&s);
        assert!(out.len() < s.len());
        assert!(out.contains("… (truncated,"));
        assert!(out.contains("488 more bytes"));
    }

    #[test]
    fn test_truncate_body_respects_char_boundary() {
        // Build a string where the cap lands inside a multi-byte char ('é' = 2 bytes):
        // MAX-1 'x' chars, then 'é', then filler. Naïve &s[..MAX] would panic here.
        let mut s = String::new();
        for _ in 0..MAX_ERROR_BODY_BYTES - 1 {
            s.push('x');
        }
        s.push('é');
        for _ in 0..100 {
            s.push('y');
        }
        let out = truncate_body(&s);
        assert!(out.is_char_boundary(out.len()));
        assert!(out.contains("… (truncated,"));
    }

    async fn make_test_client(server: &MockServer) -> ApiClient {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("APKG_TOKEN", "test-token-abc");
        // Leak the TempDir so it stays alive for the duration
        let client = ApiClient::new(Some(&server.uri())).unwrap();
        std::mem::forget(tmp);
        client
    }

    async fn make_unauthed_client(server: &MockServer) -> ApiClient {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("APKG_TOKEN");
        let client = ApiClient::new(Some(&server.uri())).unwrap();
        std::mem::forget(tmp);
        client
    }

    #[test]
    fn test_encode_package_name_simple() {
        assert_eq!(encode_package_name("my-pkg"), "my-pkg");
    }

    #[test]
    fn test_encode_package_name_scoped() {
        assert_eq!(encode_package_name("@scope/pkg"), "%40scope%2Fpkg");
    }

    #[test]
    fn test_encode_package_name_special_chars() {
        assert_eq!(encode_package_name("a b"), "a%20b");
    }

    #[tokio::test]
    async fn test_url_with_registry_override() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        let client = make_test_client(&server).await;
        let url = client.url("auth", "/auth/login");
        assert!(url.starts_with(&server.uri()));
        assert!(url.ends_with("/auth/login"));
    }

    #[tokio::test]
    async fn test_auth_headers_with_token() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        let client = make_test_client(&server).await;
        let headers = client.auth_headers().unwrap();
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert!(auth.starts_with("Bearer "));
    }

    #[tokio::test]
    async fn test_auth_headers_without_token() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        let client = make_unauthed_client(&server).await;
        let headers = client.auth_headers().unwrap();
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[tokio::test]
    async fn test_require_auth_no_token() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        let client = make_unauthed_client(&server).await;
        let result = client.require_auth_headers();
        assert!(matches!(result, Err(AppError::AuthRequired)));
    }

    #[tokio::test]
    async fn test_login_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "accessToken": "tok_123",
                "refreshToken": "rt_456",
                "expiresIn": 3600,
                "tokenType": "Bearer"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.login("user", "pass").await.unwrap();
        assert_eq!(resp.access_token.as_deref(), Some("tok_123"));
        assert!(!resp.requires_mfa());
    }

    #[tokio::test]
    async fn test_login_mfa_required() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "mfaRequired": true,
                "mfaToken": "mfa_tok_789"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.login("user", "pass").await.unwrap();
        assert!(resp.requires_mfa());
        assert_eq!(resp.mfa_token.unwrap(), "mfa_tok_789");
    }

    #[tokio::test]
    async fn test_mfa_challenge_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/mfa/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "accessToken": "tok_mfa",
                "refreshToken": "rt_mfa",
                "expiresIn": 3600,
                "tokenType": "Bearer"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.mfa_challenge("mfa_tok", "123456").await.unwrap();
        assert_eq!(resp.access_token, "tok_mfa");
    }

    #[tokio::test]
    async fn test_whoami_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/whoami"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "alice",
                "email": "alice@example.com"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.whoami().await.unwrap();
        assert_eq!(resp.username, "alice");
        assert_eq!(resp.email, "alice@example.com");
    }

    #[tokio::test]
    async fn test_whoami_requires_auth() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        let client = make_unauthed_client(&server).await;
        let result = client.whoami().await;
        assert!(matches!(result, Err(AppError::AuthRequired)));
    }

    #[tokio::test]
    async fn test_get_package_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/%40scope%2Fpkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@scope/pkg",
                "versions": {},
                "maintainers": [],
                "distTags": {}
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let meta = client.get_package("@scope/pkg").await.unwrap();
        assert_eq!(meta.name, "@scope/pkg");
    }

    #[tokio::test]
    async fn test_get_package_not_found() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/nonexistent"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "code": "NOT_FOUND", "message": "Package not found" }
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let result = client.get_package("nonexistent").await;
        assert!(matches!(result, Err(AppError::PackageNotFound(_))));
    }

    #[tokio::test]
    async fn test_download_tarball_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg/1.0.0/tarball"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"fake-tarball-bytes".to_vec())
                    .insert_header("x-integrity", "sha256-abc123"),
            )
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let (data, integrity) = client.download_tarball("mypkg", "1.0.0").await.unwrap();
        assert_eq!(data, b"fake-tarball-bytes");
        assert_eq!(integrity.unwrap(), "sha256-abc123");
    }

    #[tokio::test]
    async fn test_download_tarball_not_found() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/missing/0.0.1/tarball"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "code": "NOT_FOUND", "message": "Not found" }
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let result = client.download_tarball("missing", "0.0.1").await;
        assert!(matches!(result, Err(AppError::PackageNotFound(_))));
    }

    #[tokio::test]
    async fn test_search_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex("/search.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "name": "@test/skill", "version": "1.0.0" }
                ],
                "total": 1
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.search("test", 10, 0).await.unwrap();
        assert_eq!(resp.total, 1);
        assert_eq!(resp.results[0].name, "@test/skill");
    }

    #[tokio::test]
    async fn test_publish_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/packages/%40scope%2Fpkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "@scope/pkg",
                "version": "1.0.0",
                "integrity": "sha256-xyz"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .publish("@scope/pkg", r#"{"name":"@scope/pkg"}"#, vec![1, 2, 3])
            .await
            .unwrap();
        assert_eq!(resp.name, "@scope/pkg");
        assert_eq!(resp.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_register_key_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keyId": "key-001",
                "name": "mykey",
                "algorithm": "ed25519",
                "createdAt": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.register_key("pubkey123", "mykey").await.unwrap();
        assert_eq!(resp.key_id, "key-001");
    }

    #[tokio::test]
    async fn test_list_keys_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "keyId": "key-001",
                    "name": "mykey",
                    "algorithm": "ed25519",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.list_keys().await.unwrap();
        assert_eq!(resp.keys.len(), 1);
        assert_eq!(resp.keys[0].key_id, "key-001");
    }

    #[tokio::test]
    async fn test_revoke_key_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex("/auth/keys/.+/revoke"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keyId": "key-001",
                "status": "revoked",
                "revokedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .revoke_key("key-001", "compromised", None)
            .await
            .unwrap();
        assert_eq!(resp.status, "revoked");
    }

    #[tokio::test]
    async fn test_rotate_key_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/keys/rotate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "newKeyId": "key-002",
                "oldKeyId": "key-001",
                "rotatedAt": "2026-02-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .rotate_key("key-001", "new-pub", "attestation")
            .await
            .unwrap();
        assert_eq!(resp.new_key_id, "key-002");
    }

    #[tokio::test]
    async fn test_create_token_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "tok-001",
                "name": "ci-token",
                "token": "apkg_abc123",
                "scopes": ["publish"],
                "expiresAt": "2027-01-01T00:00:00Z",
                "createdAt": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let scopes = vec!["publish".to_string()];
        let resp = client
            .create_token("ci-token", &scopes, "365d", None)
            .await
            .unwrap();
        assert_eq!(resp.id, "tok-001");
        assert_eq!(resp.token, "apkg_abc123");
    }

    #[tokio::test]
    async fn test_list_tokens_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth/tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": [{
                    "id": "tok-001",
                    "name": "ci-token",
                    "scopes": ["publish"],
                    "expiresIn": "365d",
                    "expiresAt": "2027-01-01T00:00:00Z",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.list_tokens().await.unwrap();
        assert_eq!(resp.tokens.len(), 1);
    }

    #[tokio::test]
    async fn test_revoke_token_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path_regex("/auth/tokens/.+"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        client.revoke_token("tok-001").await.unwrap();
    }

    #[tokio::test]
    async fn test_set_dist_tag_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex("/packages/.+/dist-tags/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag": "latest",
                "version": "1.0.0"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .set_dist_tag("mypkg", "latest", "1.0.0")
            .await
            .unwrap();
        assert_eq!(resp.tag, "latest");
        assert_eq!(resp.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_remove_dist_tag_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path_regex("/packages/.+/dist-tags/.+"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        client.remove_dist_tag("mypkg", "beta").await.unwrap();
    }

    #[tokio::test]
    async fn test_deprecate_package_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "mypkg",
                "versions": {},
                "maintainers": [],
                "distTags": {},
                "deprecated": "This package is deprecated"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .deprecate_package("mypkg", Some("This package is deprecated"))
            .await
            .unwrap();
        assert_eq!(resp.name, "mypkg");
    }

    #[tokio::test]
    async fn test_deprecate_version_success() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/packages/mypkg/1.0.0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "version": "1.0.0",
                "deprecated": "Use v2"
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client
            .deprecate_version("mypkg", "1.0.0", Some("Use v2"))
            .await
            .unwrap();
        assert_eq!(resp.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_get_registry_signing_keys() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/registry/signing-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "keys": [{
                    "keyId": "rsk-001",
                    "publicKey": "ed25519-pub",
                    "algorithm": "ed25519",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "expiresAt": "2027-01-01T00:00:00Z"
                }]
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.get_registry_signing_keys().await.unwrap();
        assert_eq!(resp.keys.len(), 1);
        assert_eq!(resp.keys[0].key_id, "rsk-001");
    }

    #[tokio::test]
    async fn test_get_provenance_some() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg/1.0.0/provenance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "predicate": { "builderId": "github-actions" }
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.get_provenance("mypkg", "1.0.0").await.unwrap();
        assert!(resp.is_some());
    }

    #[tokio::test]
    async fn test_get_provenance_none() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/packages/mypkg/1.0.0/provenance"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "code": "NOT_FOUND", "message": "Not found" }
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let resp = client.get_provenance("mypkg", "1.0.0").await.unwrap();
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn test_login_failure_401() {
        let _lock = env_lock();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/login"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": { "code": "UNAUTHORIZED", "message": "Invalid credentials" }
            })))
            .mount(&server)
            .await;
        let client = make_test_client(&server).await;
        let result = client.login("user", "wrong").await;
        assert!(result.is_err());
    }
}
